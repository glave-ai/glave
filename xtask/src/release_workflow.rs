//! `cargo xtask release-workflow`: writes `.github/workflows/release.yml`, the workflow dist
//! generates from `dist-workspace.toml` and `.github/build-setup.yml`, with the one change dist
//! has no setting for. dist gives the whole workflow `contents: write`, so every job's token, the
//! builds' too, could change the repository's releases and tags. Here the workflow's token only
//! reads, and the job that publishes keeps the write access it asks for itself. `cargo xtask ci`
//! checks that nothing else can write.

use std::path::Path;

use crate::{cmd, workspace};

/// dist's config.
const CONFIG: &str = "dist-workspace.toml";
/// The line of [`CONFIG`] that says the workflow is edited by hand: dist then neither rewrites
/// it nor fails when it differs from what dist would write.
const HAND_EDITED: &str = "allow-dirty = [\"ci\"]\n";
/// The workflow dist generates.
const WORKFLOW: &str = ".github/workflows/release.yml";
/// The workflow's own permissions, as dist writes them.
const DIST_PERMISSIONS: &str = "\npermissions:\n  \"contents\": \"write\"\n";
/// The workflow's own permissions, as they ship.
const READ_ONLY: &str = "\npermissions:\n  \"contents\": \"read\"\n";
/// The job that publishes the release and attests to it: the only one that may write.
const PUBLISHER: &str = "host";

/// Has dist write the workflow, then makes its token read-only.
pub fn run(args: &[String]) -> Result<(), String> {
    if !args.is_empty() {
        return Err("usage: cargo xtask release-workflow".to_owned());
    }
    let root = workspace::root()?;
    let config_path = root.join(CONFIG);
    let config = read(&config_path)?;
    let unlocked = config.replacen(HAND_EDITED, "", 1);
    if unlocked == config {
        return Err(format!(
            "{CONFIG} no longer has `{}`, so dist would fail on the change this makes",
            HAND_EDITED.trim_end()
        ));
    }
    // dist rewrites the workflow only while its config doesn't say it is edited by hand.
    write(&config_path, &unlocked)?;
    let generated = cmd::run("dist", &["generate", "--mode", "ci"]);
    write(&config_path, &config)?;
    generated.map_err(|err| format!("{err} (is dist, the version {CONFIG} names, installed?)"))?;
    let workflow_path = root.join(WORKFLOW);
    let workflow = narrow(&read(&workflow_path)?)?;
    write(&workflow_path, &workflow)?;
    println!("xtask: wrote {WORKFLOW}");
    Ok(())
}

/// Fails unless only the publishing job of the checked-in workflow can write.
pub fn check() -> Result<(), String> {
    let workflow = read(&workspace::root()?.join(WORKFLOW))?;
    only_publisher_writes(&workflow)
        .map_err(|err| format!("{WORKFLOW}: {err}: run `cargo xtask release-workflow`"))?;
    println!("xtask: in {WORKFLOW}, only the `{PUBLISHER}` job can write");
    Ok(())
}

/// dist's workflow with its own permissions read-only.
fn narrow(workflow: &str) -> Result<String, String> {
    let narrowed = workflow.replacen(DIST_PERMISSIONS, READ_ONLY, 1);
    only_publisher_writes(&narrowed)
        .map_err(|err| format!("dist's workflow has changed: {err}"))?;
    Ok(narrowed)
}

/// Fails unless the workflow's own permissions are read-only and only the publishing job is
/// given write access.
fn only_publisher_writes(workflow: &str) -> Result<(), String> {
    if !workflow.contains(READ_ONLY) {
        return Err("the workflow's own permissions are not `contents: read`".to_owned());
    }
    // Whether a line is under the top-level `jobs:`, and if so, in which job.
    let mut in_jobs = false;
    let mut job = None;
    for line in workflow.lines() {
        if line.starts_with(|c: char| c.is_ascii_alphabetic()) {
            in_jobs = line == "jobs:";
            job = None;
        } else if in_jobs && let Some(name) = job_name(line) {
            job = Some(name);
        } else if line.trim_end().ends_with(": \"write\"") && job != Some(PUBLISHER) {
            return Err(match job {
                Some(name) => format!("the `{name}` job can write; only `{PUBLISHER}` may"),
                None => format!("the workflow's own permissions write; only `{PUBLISHER}` may"),
            });
        }
    }
    Ok(())
}

/// The job a line under `jobs:` starts: `  host:` starts `host`.
fn job_name(line: &str) -> Option<&str> {
    let name = line.strip_prefix("  ")?.strip_suffix(':')?;
    let is_name = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    is_name.then_some(name)
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("could not read {}: {e}", path.display()))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of dist's workflow: its own permissions, triggers indented as jobs are, and
    /// the publishing job asking for write access itself.
    const DIST_WORKFLOW: &str = r#"name: Release
permissions:
  "contents": "write"

on:
  pull_request:
  push:

jobs:
  # Run 'dist plan'
  plan:
    runs-on: "ubuntu-22.04"
    steps:
      - run: dist plan
  build-local-artifacts:
    runs-on: ${{ matrix.runner }}
  host:
    permissions:
      "attestations": "write"
      "contents": "write"
      "id-token": "write"
  announce:
    runs-on: "ubuntu-22.04"
"#;

    #[test]
    fn the_workflow_token_reads_and_the_publisher_still_writes() {
        let narrowed = narrow(DIST_WORKFLOW).unwrap();
        assert!(narrowed.starts_with("name: Release\npermissions:\n  \"contents\": \"read\"\n"));
        assert!(narrowed.contains("      \"contents\": \"write\"\n      \"id-token\": \"write\""));
        assert_eq!(
            narrow(&narrowed),
            Ok(narrowed.clone()),
            "narrowing twice changes nothing"
        );
    }

    #[test]
    fn any_other_job_that_writes_fails() {
        let build_writes = DIST_WORKFLOW.replace(
            "    runs-on: ${{ matrix.runner }}\n",
            "    runs-on: ${{ matrix.runner }}\n    permissions:\n      \"contents\": \"write\"\n",
        );
        let err = narrow(&build_writes).unwrap_err();
        assert!(
            err.contains("`build-local-artifacts` job can write"),
            "{err}"
        );
    }

    #[test]
    fn the_workflow_must_set_its_own_permissions() {
        let unset = DIST_WORKFLOW.replace("permissions:\n  \"contents\": \"write\"\n\n", "");
        let err = narrow(&unset).unwrap_err();
        assert!(err.contains("not `contents: read`"), "{err}");
        let more = DIST_WORKFLOW.replace(
            "  \"contents\": \"write\"\n\n",
            "  \"contents\": \"write\"\n  \"id-token\": \"write\"\n\n",
        );
        let err = narrow(&more).unwrap_err();
        assert!(
            err.contains("the workflow's own permissions write"),
            "{err}"
        );
    }
}
