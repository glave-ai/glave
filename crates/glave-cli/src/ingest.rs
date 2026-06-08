//! `glave ingest` — turn a real dataset instance (Multi-SWE-bench / SWE-bench)
//! into a runnable Glave task.
//!
//! Recipe (proven on tokio-rs/bytes and grpc/grpc-go): clone @ base sha → make
//! the repo standalone where the host toolchain would otherwise attach it to the
//! Glave workspace → vendor deps so the test command runs offline under
//! `--network none` → workspace = the repo (bug present), oracle = the held test
//! patch, solution = the gold fix (the fix patch applied, changed files extracted).
//!
//! Per-language bits (vendoring, the offline test command, the editable surface
//! extension) live in [`Lang`] + [`derive`]; everything else is shared.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::Command;

pub struct IngestArgs<'a> {
    pub dataset: &'a Path,
    pub index: usize,
    pub out: &'a Path,
    pub lang: &'a str,
}

#[derive(Copy, Clone)]
enum Lang {
    Rust,
    Go,
    Python,
    Typescript,
}

impl Lang {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "rust" => Ok(Self::Rust),
            "go" => Ok(Self::Go),
            "python" | "py" => Ok(Self::Python),
            "typescript" | "ts" => Ok(Self::Typescript),
            other => bail!("unsupported language `{other}` (have: rust, go, python, typescript)"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Go => "go",
            Self::Python => "python",
            Self::Typescript => "typescript",
        }
    }

    fn image(self) -> &'static str {
        match self {
            Self::Rust => "glave/rust-base:latest",
            Self::Go => "glave/go-base:latest",
            Self::Python => "glave/python-base:latest",
            Self::Typescript => "glave/typescript-base:latest",
        }
    }
}

/// Language-specific task wiring derived from the instance's patches.
struct Derived {
    editable: Vec<String>,
    fail_to_pass: String,
    pass_to_pass: String,
}

pub fn ingest(args: &IngestArgs) -> Result<()> {
    let lang = Lang::parse(args.lang)?;
    let line = nth_line(args.dataset, args.index)?;
    let inst: Value = serde_json::from_str(&line).context("parsing instance JSON")?;

    let org = sval(&inst, "org")?;
    let repo = sval(&inst, "repo")?;
    let sha = inst
        .pointer("/base/sha")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("instance missing base.sha"))?;
    let fix_patch = sval(&inst, "fix_patch")?;
    let test_patch = sval(&inst, "test_patch")?;
    let title = inst.get("title").and_then(Value::as_str).unwrap_or("");
    let body = inst.get("body").and_then(Value::as_str).unwrap_or("");

    let fix_files = patch_files(fix_patch);
    let derived = derive(lang, test_patch, &fix_files)?;

    let out = args.out;
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir_all(out)?;
    let ws = out.join("workspace");
    let ws_s = ws.to_string_lossy().into_owned();

    // clone @ sha (keep .git: the oracle applies/reverts patches against it)
    run("git", &["clone", "--quiet", &format!("https://github.com/{org}/{repo}"), &ws_s])?;
    run("git", &["-C", &ws_s, "checkout", "--quiet", sha])?;

    vendor(lang, &ws, &ws_s)?;

    // oracle = the held test patch
    std::fs::create_dir_all(out.join("oracle"))?;
    std::fs::write(out.join("oracle/test.patch"), test_patch)?;

    extract_solution(&ws, out, fix_patch, &fix_files)?;

    // Compute SWE-bench-grade F2P/P2P by diffing per-test results at base vs
    // base+fix (Python now; other langs keep their whole-suite oracle for now).
    compute_oracle(lang, out, test_patch)?;

    let id = format!("{repo}-{}", args.index);
    let statement = format!("{title}\n\n{body}").trim().replace("'''", "''");
    std::fs::write(
        out.join("task.toml"),
        task_toml(&id, lang, &statement, org, repo, sha, &derived),
    )?;

    println!("ingested {org}/{repo}@{sha} ({}) -> {}", lang.name(), out.display());
    println!("  editable: {}", derived.editable.join(", "));
    println!("  fail_to_pass: {}", derived.fail_to_pass);
    Ok(())
}

fn derive(lang: Lang, test_patch: &str, fix_files: &[String]) -> Result<Derived> {
    let test_files = patch_files(test_patch);
    let dirs = top_dirs(fix_files);
    if dirs.is_empty() {
        bail!("fix patch touches no files");
    }
    match lang {
        Lang::Rust => {
            let stem = test_files
                .iter()
                .find_map(|f| f.strip_prefix("tests/").and_then(|s| s.strip_suffix(".rs")))
                .ok_or_else(|| anyhow!("rust: need a tests/<name>.rs target in test_patch"))?;
            Ok(Derived {
                editable: dirs.iter().map(|d| format!("{d}/**/*.rs")).collect(),
                fail_to_pass: format!(
                    "git apply --check /oracle/test.patch 2>/dev/null && git apply /oracle/test.patch; cargo test --offline --test {stem}"
                ),
                pass_to_pass: "cargo test --offline --lib".to_string(),
            })
        }
        Lang::Go => {
            let testfile = test_files
                .first()
                .ok_or_else(|| anyhow!("go: test_patch touches no files"))?;
            let pkg = testfile.rsplit_once('/').map_or(".", |(d, _)| d);
            let funcs = go_test_funcs(test_patch);
            if funcs.is_empty() {
                bail!("go: could not find added `func Test...` in test_patch");
            }
            let run = funcs.join("|");
            Ok(Derived {
                editable: dirs.iter().map(|d| format!("{d}/**/*.go")).collect(),
                fail_to_pass: format!(
                    "git apply --check /oracle/test.patch 2>/dev/null && git apply /oracle/test.patch; go test -mod=vendor -run '{run}' ./{pkg}/"
                ),
                // The test patch often updates the test API to match the fix, so we
                // can't revert it; keep it applied and run the whole package — a
                // bad fix that breaks a sibling test in the package still fails.
                pass_to_pass: format!(
                    "git apply --check /oracle/test.patch 2>/dev/null && git apply /oracle/test.patch; go test -mod=vendor ./{pkg}/"
                ),
            })
        }
        Lang::Python => {
            let testfile = test_files
                .iter()
                .find(|f| has_ext(f, "py"))
                .ok_or_else(|| anyhow!("python: need a .py file in test_patch"))?;
            let root = test_root(testfile);
            // Deps live in an in-container venv at /work/.venv (see `vendor`); the
            // editable install makes the repo source the importable package, so the
            // agent's edits change what the test sees. The oracle is the *computed*
            // F2P/P2P lists (see `compute_oracle`): run the suite, check the list.
            Ok(Derived {
                editable: dirs.iter().map(|d| format!("{d}/**/*.py")).collect(),
                fail_to_pass: format!(
                    "git apply --check /oracle/test.patch 2>/dev/null && git apply /oracle/test.patch; .venv/bin/python /oracle/check_oracle.py {root} /oracle/f2p.txt"
                ),
                pass_to_pass: format!(
                    "git apply --check /oracle/test.patch 2>/dev/null && git apply /oracle/test.patch; .venv/bin/python /oracle/check_oracle.py {root} /oracle/p2p.txt"
                ),
            })
        }
        Lang::Typescript => {
            let testfile = test_files
                .iter()
                .find(|f| ["ts", "tsx", "js", "jsx"].into_iter().any(|e| has_ext(f, e)))
                .ok_or_else(|| anyhow!("typescript: need a .ts/.tsx/.js test file in test_patch"))?;
            // glob crate has no brace expansion -> one editable pattern per extension.
            let editable = dirs
                .iter()
                .flat_map(|d| {
                    ["ts", "tsx", "js", "jsx"]
                        .iter()
                        .map(move |e| format!("{d}/**/*.{e}"))
                })
                .collect();
            Ok(Derived {
                editable,
                fail_to_pass: format!(
                    "git apply --check /oracle/test.patch 2>/dev/null && git apply /oracle/test.patch; npx --no-install jest {testfile} || npx --no-install vitest run {testfile}"
                ),
                pass_to_pass: "npx --no-install jest || npx --no-install vitest run".to_string(),
            })
        }
    }
}

fn vendor(lang: Lang, ws: &Path, ws_s: &str) -> Result<()> {
    match lang {
        Lang::Rust => {
            // standalone so host cargo vendor doesn't attach to the Glave workspace
            append(&ws.join("Cargo.toml"), "\n[workspace]\n")?;
            let manifest = ws.join("Cargo.toml").to_string_lossy().into_owned();
            let vendor_dir = ws.join("vendor").to_string_lossy().into_owned();
            run("cargo", &["vendor", "--manifest-path", &manifest, &vendor_dir])?;
            std::fs::create_dir_all(ws.join(".cargo"))?;
            std::fs::write(
                ws.join(".cargo/config.toml"),
                "[source.crates-io]\nreplace-with = \"vendored-sources\"\n[source.vendored-sources]\ndirectory = \"vendor\"\n",
            )?;
            let _ = std::fs::remove_dir_all(ws.join("target"));
        }
        Lang::Go => {
            run("go", &["-C", ws_s, "mod", "vendor"])?;
        }
        Lang::Python => {
            // Host (mac) can't build Linux wheels, so vendor INSIDE the image into a
            // venv at /work/.venv — reusable at eval (same image, same /work mount).
            // Network is available at ingest; the eval run is offline against .venv.
            // Editable install (-e .) makes the repo source the importable package.
            let mount = format!("{ws_s}:/work");
            run(
                "docker",
                &[
                    "run", "--rm", "-v", &mount, "-w", "/work", Lang::Python.image(), "sh", "-c",
                    "python -m venv .venv && .venv/bin/pip install -q -U pip \
                     && .venv/bin/pip install -q -e . pytest",
                ],
            )?;
        }
        Lang::Typescript => {
            // Native node modules must be built for Linux -> install inside the image.
            let mount = format!("{ws_s}:/work");
            run(
                "docker",
                &[
                    "run", "--rm", "-v", &mount, "-w", "/work", Lang::Typescript.image(), "sh",
                    "-c", "npm ci || npm install",
                ],
            )?;
        }
    }
    Ok(())
}

/// Apply the fix patch to a copy of the edited dir(s); copy changed files to solution/.
fn extract_solution(ws: &Path, out: &Path, fix_patch: &str, fix_files: &[String]) -> Result<()> {
    let soltmp = out.join(".soltmp");
    let _ = std::fs::remove_dir_all(&soltmp);
    for d in top_dirs(fix_files) {
        copy_dir(&ws.join(&d), &soltmp.join(&d))?;
    }
    let fixpath = out.join(".fix.patch");
    std::fs::write(&fixpath, fix_patch)?;
    let fixabs = std::fs::canonicalize(&fixpath)?.to_string_lossy().into_owned();
    let soltmp_s = soltmp.to_string_lossy().into_owned();
    run("git", &["-C", &soltmp_s, "init", "--quiet"])?;
    run("git", &["-C", &soltmp_s, "apply", &fixabs])?;
    for f in fix_files {
        let dst = out.join("solution").join(f);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(soltmp.join(f), &dst).with_context(|| format!("copying solution file {f}"))?;
    }
    let _ = std::fs::remove_dir_all(&soltmp);
    let _ = std::fs::remove_file(&fixpath);
    Ok(())
}

/// Compute SWE-bench-grade F2P/P2P test-id lists by running the suite twice (base,
/// then base+fix) on a throwaway copy of the vendored workspace, and write them +
/// the check script into `oracle/`. The check script (run at grade time) executes
/// the suite once and verifies the stored ids — robust to env-broken tests and to
/// inter-test state. Python only for now; other languages keep their existing
/// whole-suite oracle until their per-test result parsers land.
fn compute_oracle(lang: Lang, out: &Path, test_patch: &str) -> Result<()> {
    if !matches!(lang, Lang::Python) {
        return Ok(());
    }
    let root = patch_files(test_patch)
        .first()
        .map_or_else(|| "tests".to_string(), |f| test_root(f));

    let oracle = out.join("oracle");
    std::fs::write(oracle.join("check_oracle.py"), check_py())?;
    std::fs::write(oracle.join("compute_oracle.py"), compute_py())?;

    // Copy the vendored workspace so the real one stays pristine at base.
    let tmp = out.join(".computetmp");
    let _ = std::fs::remove_dir_all(&tmp);
    copy_dir(&out.join("workspace"), &tmp)?;

    let tmp_mount = format!("{}:/work", tmp.to_string_lossy());
    let oracle_mount = format!("{}:/oracle", oracle.to_string_lossy());
    let sol_mount = format!("{}:/solution:ro", out.join("solution").to_string_lossy());
    run(
        "docker",
        &[
            "run", "--rm", "-v", &tmp_mount, "-v", &oracle_mount, "-v", &sol_mount, "-w", "/work",
            lang.image(), "sh", "-c", &format!(".venv/bin/python /oracle/compute_oracle.py {root}"),
        ],
    )?;

    let _ = std::fs::remove_dir_all(&tmp);
    let _ = std::fs::remove_file(oracle.join("compute_oracle.py")); // keep oracle/ lean

    // The task must discriminate: a non-empty FAIL_TO_PASS is required.
    let f2p = std::fs::read_to_string(oracle.join("f2p.txt")).unwrap_or_default();
    if f2p.split_whitespace().next().is_none() {
        bail!("compute produced an empty FAIL_TO_PASS — task does not discriminate");
    }
    println!("  computed F2P/P2P lists -> oracle/f2p.txt, oracle/p2p.txt");
    Ok(())
}

/// The top-level directory of a test file (`tests/test_x.py` → `tests`), i.e. what
/// pytest runs as the suite; the file itself if it sits at the repo root.
fn test_root(testfile: &str) -> String {
    testfile.split_once('/').map_or(testfile, |(d, _)| d).to_string()
}

fn task_toml(
    id: &str,
    lang: Lang,
    statement: &str,
    org: &str,
    repo: &str,
    sha: &str,
    d: &Derived,
) -> String {
    let editable = toml_array(&d.editable);
    let (lang_name, image, f2p, p2p) = (lang.name(), lang.image(), &d.fail_to_pass, &d.pass_to_pass);
    format!(
        r#"id = "{id}"
language = "{lang_name}"
archetype = "real-issue"
difficulty = "high"
editable_surface = {editable}
statement = '''
{statement}
'''

[provenance]
source = "multi-swe-bench:{org}/{repo}@{sha}"
contamination = "known"
calibration = false

[image]
base = "{image}"
workdir = "/work"

[oracle]
fail_to_pass = "{f2p}"
pass_to_pass = "{p2p}"

[resource]
mem_mb = 2048
cpus = 2.0
pids = 512
network = "none"
"#
    )
}

fn nth_line(path: &Path, n: usize) -> Result<String> {
    let f = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    BufReader::new(f)
        .lines()
        .nth(n)
        .ok_or_else(|| anyhow!("dataset has no instance at index {n}"))?
        .context("reading instance line")
}

fn sval<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("instance missing string field `{key}`"))
}

/// Case-insensitive file-extension check (clippy-friendly; patch paths vary).
fn has_ext(path: impl AsRef<str>, ext: &str) -> bool {
    std::path::Path::new(path.as_ref())
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// New-file paths touched by a unified diff (the `+++ b/<path>` lines).
fn patch_files(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|l| l.strip_prefix("+++ b/"))
        .map(|s| s.trim().to_string())
        .collect()
}

/// Top-level directory of each touched file, unique and sorted.
fn top_dirs(files: &[String]) -> Vec<String> {
    let mut v: Vec<String> = files
        .iter()
        .filter_map(|f| f.split('/').next())
        .map(str::to_string)
        .collect();
    v.sort();
    v.dedup();
    v
}

/// Top-level `func TestXxx` definitions added by a Go test patch.
fn go_test_funcs(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|l| l.strip_prefix("+func "))
        .filter_map(|s| s.split('(').next())
        .map(str::trim)
        .filter(|n| n.starts_with("Test"))
        .map(str::to_string)
        .collect()
}

fn toml_array(items: &[String]) -> String {
    let inner = items
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

fn run(cmd: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(cmd)
        .args(args)
        .output()
        .with_context(|| format!("running {cmd}"))?;
    if !out.status.success() {
        bail!("{cmd} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn append(path: &Path, text: &str) -> Result<()> {
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .with_context(|| format!("opening {} to append", path.display()))?;
    f.write_all(text.as_bytes())?;
    Ok(())
}

fn copy_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src).with_context(|| format!("reading {}", src.display()))? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        let ft = entry.file_type()?; // does NOT follow symlinks
        if ft.is_symlink() {
            // Preserve venv symlinks (bin/python → container interpreter) verbatim.
            let target = std::fs::read_link(&from)?;
            let _ = std::fs::remove_file(&to);
            std::os::unix::fs::symlink(&target, &to)?;
        } else if ft.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// Shared: run the suite to junit XML, then parse it to a {node_id: passed} map.
/// Node-ids are reconstructed exactly from the testcase `file` attribute (so
/// class-based `file::Class::test` and parametrized ids match pytest's own and
/// SWE-bench's format); a classname fallback covers older junit without `file`.
/// failure/error → fail; skipped/xfail ignored. Identical at compute & grade time.
const PY_COMMON: &str = r#"
def run_suite(test_root, tag):
    rep = "/tmp/" + tag + ".xml"
    subprocess.run([".venv/bin/python", "-m", "pytest", test_root,
                    "--junitxml=" + rep, "--tb=no", "-q", "-p", "no:cacheprovider"],
                   cwd="/work", stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return rep

def parse(rep):
    out = {}
    if not os.path.exists(rep):
        return out
    for tc in ET.parse(rep).getroot().iter("testcase"):
        name, cls, fname = tc.get("name", ""), tc.get("classname", ""), tc.get("file")
        if fname and fname.endswith(".py"):
            module = fname[:-3].replace("/", ".")
            klass = cls[len(module):].lstrip(".") if cls.startswith(module) else ""
            nid = fname + "::" + (klass + "::" + name if klass else name)
        else:
            nid = cls.replace(".", "/") + ".py::" + name
        kinds = {c.tag for c in tc}
        if "skipped" in kinds:
            continue
        out[nid] = not ({"failure", "error"} & kinds)
    return out
"#;

/// Ingest-time: diff per-test results at base vs base+fix → write f2p/p2p lists.
const COMPUTE_PY_BODY: &str = r#"
TEST_ROOT = sys.argv[1]
subprocess.run(["git", "apply", "/oracle/test.patch"], cwd="/work")
base = parse(run_suite(TEST_ROOT, "base"))
subprocess.run("cp -a /solution/. /work/", shell=True, cwd="/work")
fix = parse(run_suite(TEST_ROOT, "fix"))

touched = set(re.findall(r"^\+\+\+ b/(.+?)\s*$", open("/oracle/test.patch").read(), re.M))
def in_patch(nid):
    return nid.split("::", 1)[0] in touched

f2p = sorted(k for k in base if base[k] is False and fix.get(k) is True and in_patch(k))
p2p = sorted(k for k in base if base[k] is True and fix.get(k) is True)
broke = sorted(k for k in base if base[k] is True and fix.get(k) is False)

open("/oracle/f2p.txt", "w").write("\n".join(f2p) + "\n")
open("/oracle/p2p.txt", "w").write("\n".join(p2p) + "\n")
sys.stderr.write("computed F2P=%d P2P=%d regressed=%d\n" % (len(f2p), len(p2p), len(broke)))
"#;

/// Grade-time: run the suite once, then verify every id in the list passed.
const CHECK_PY_BODY: &str = r#"
TEST_ROOT, ID_FILE = sys.argv[1], sys.argv[2]
res = parse(run_suite(TEST_ROOT, "check"))
ids = [l.strip() for l in open(ID_FILE) if l.strip()]
missing = [i for i in ids if res.get(i) is not True]
if missing:
    sys.stderr.write("%d/%d required tests not passing:\n" % (len(missing), len(ids)))
    for m in missing[:10]:
        sys.stderr.write("  " + m + "\n")
    sys.exit(1)
sys.exit(0)
"#;

fn py_script(body: &str) -> String {
    format!("import subprocess, sys, os, re, xml.etree.ElementTree as ET\n{PY_COMMON}\n{body}")
}

fn compute_py() -> String {
    py_script(COMPUTE_PY_BODY)
}

fn check_py() -> String {
    py_script(CHECK_PY_BODY)
}
