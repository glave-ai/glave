//! `glave ingest` — turn a real dataset instance (Multi-SWE-bench / SWE-bench)
//! into a runnable Glave task. Rust-focused for now (cargo vendor + cargo test);
//! other languages get their own recipe later.
//!
//! Recipe (proven on tokio-rs/bytes): clone @ base sha → mark standalone so host
//! cargo doesn't attach it to the Glave workspace → `cargo vendor` + a relative
//! config so `cargo test --offline` works under `--network none` → workspace =
//! the repo (bug present), oracle = the held test patch, solution = the gold fix.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::Command;

pub struct IngestArgs<'a> {
    pub dataset: &'a Path,
    pub index: usize,
    pub out: &'a Path,
}

pub fn ingest(args: &IngestArgs) -> Result<()> {
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
    let test_files = patch_files(test_patch);
    let test_stem = test_files
        .iter()
        .find_map(|f| f.strip_prefix("tests/").and_then(|s| s.strip_suffix(".rs")))
        .ok_or_else(|| anyhow!("could not derive a tests/<name>.rs target from test_patch"))?;
    let edit_top = fix_files
        .iter()
        .find_map(|f| f.split('/').next())
        .ok_or_else(|| anyhow!("could not derive an editable dir from fix_patch"))?;

    let out = args.out;
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir_all(out)?;
    let ws = out.join("workspace");
    let ws_s = ws.to_string_lossy().into_owned();

    // clone @ sha
    run("git", &["clone", "--quiet", &format!("https://github.com/{org}/{repo}"), &ws_s])?;
    run("git", &["-C", &ws_s, "checkout", "--quiet", sha])?;

    // mark standalone so host cargo vendor doesn't attach to the Glave workspace
    append(&ws.join("Cargo.toml"), "\n[workspace]\n")?;

    // vendor deps + relative config -> offline builds
    let manifest = ws.join("Cargo.toml").to_string_lossy().into_owned();
    let vendor = ws.join("vendor").to_string_lossy().into_owned();
    run("cargo", &["vendor", "--manifest-path", &manifest, &vendor])?;
    std::fs::create_dir_all(ws.join(".cargo"))?;
    std::fs::write(
        ws.join(".cargo/config.toml"),
        "[source.crates-io]\nreplace-with = \"vendored-sources\"\n[source.vendored-sources]\ndirectory = \"vendor\"\n",
    )?;
    let _ = std::fs::remove_dir_all(ws.join("target"));

    // oracle = the held test patch
    std::fs::create_dir_all(out.join("oracle"))?;
    std::fs::write(out.join("oracle/test.patch"), test_patch)?;

    // solution = apply the fix patch to a copy of the edited dir, extract changed files
    let soltmp = out.join(".soltmp");
    let _ = std::fs::remove_dir_all(&soltmp);
    copy_dir(&ws.join(edit_top), &soltmp.join(edit_top))?;
    let fixpath = out.join(".fix.patch");
    std::fs::write(&fixpath, fix_patch)?;
    let fixabs = std::fs::canonicalize(&fixpath)?.to_string_lossy().into_owned();
    let soltmp_s = soltmp.to_string_lossy().into_owned();
    run("git", &["-C", &soltmp_s, "init", "--quiet"])?;
    run("git", &["-C", &soltmp_s, "apply", &fixabs])?;
    for f in &fix_files {
        let dst = out.join("solution").join(f);
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(soltmp.join(f), &dst).with_context(|| format!("copying solution file {f}"))?;
    }
    let _ = std::fs::remove_dir_all(&soltmp);
    let _ = std::fs::remove_file(&fixpath);

    // task.toml
    let id = format!("{repo}-{}", args.index);
    let statement = format!("{title}\n\n{body}").trim().replace("'''", "''");
    std::fs::write(
        out.join("task.toml"),
        task_toml(&id, edit_top, &statement, org, repo, sha, test_stem),
    )?;

    println!("ingested {org}/{repo}@{sha} -> {}", out.display());
    println!("  edit surface: {edit_top}/**/*.rs   test target: {test_stem}");
    Ok(())
}

fn task_toml(
    id: &str,
    edit_top: &str,
    statement: &str,
    org: &str,
    repo: &str,
    sha: &str,
    test_stem: &str,
) -> String {
    format!(
        r#"id = "{id}"
language = "rust"
archetype = "real-issue"
difficulty = "high"
codebase_size = "medium"
editable_surface = ["{edit_top}/**/*.rs"]
statement = '''
{statement}
'''

[provenance]
source = "multi-swe-bench:{org}/{repo}@{sha}"
contamination = "known"
calibration = false

[image]
base = "glave/rust-base:latest"
workdir = "/work"

[oracle]
fail_to_pass = "git apply --check /oracle/test.patch 2>/dev/null && git apply /oracle/test.patch; cargo test --offline --test {test_stem}"
pass_to_pass = "cargo test --offline --lib"

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
