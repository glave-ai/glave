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
}

impl Lang {
    fn parse(s: &str) -> Result<Self> {
        match s {
            "rust" => Ok(Self::Rust),
            "go" => Ok(Self::Go),
            other => bail!("unsupported language `{other}` (have: rust, go)"),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Go => "go",
        }
    }

    fn image(self) -> &'static str {
        match self {
            Self::Rust => "glave/rust-base:latest",
            Self::Go => "glave/go-base:latest",
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
codebase_size = "medium"
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
        if from.is_dir() {
            copy_dir(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}
