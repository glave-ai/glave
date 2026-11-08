//! `cargo xtask release-workflow`: writes `.github/workflows/release.yml`, the workflow dist
//! generates from `dist-workspace.toml` and `.github/build-setup.yml`, with the three changes
//! dist has no setting for:
//!
//! - dist gives the whole workflow `contents: write`, so every job's token, the builds' too,
//!   could change the repository's releases and tags. Here the workflow's token only reads, and
//!   the job that publishes keeps the write access it asks for itself.
//! - dist's install script installs an archive unchecked where `sha256sum` is missing, as on
//!   macOS before 15. Here the job that writes it runs `.github/install-script-checksums.sh` on
//!   it before uploading it, so it checks every archive, with `shasum` where it must.
//! - The jobs that install dist fetch dist's install script with curl, and the script fetches
//!   dist's archive with a curl of its own, each with one try, so a moment's HTTP 504 from
//!   GitHub's release downloads failed every run (#1295). Here every curl those steps run reads
//!   `.github/curl/.curlrc`, which tries each download again for up to five minutes, then fails.
//!
//! `cargo xtask ci` checks all three.

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
/// The upload of the job that writes the install script, which the step below goes before.
const GLOBAL_UPLOAD: &str = "      - name: \"Upload artifacts\"\n";
/// What names that job's upload, after [`GLOBAL_UPLOAD`].
const GLOBAL_ARTIFACTS: &str = "          name: artifacts-build-global\n";
/// The step that makes the install script check every archive it downloads.
const CHECKSUMS: &str = "      - name: \"The install script checks every archive it downloads\"
        run: \"sh .github/install-script-checksums.sh target/distrib/glave-installer.sh\"
";
/// How a step starts, under its job's `steps:`.
const STEP: &str = "      - ";
/// How each line of a step after its first starts.
const STEP_LINE: &str = "        ";
/// How a step's environment starts.
const STEP_ENV: &str = "        env:\n";
/// What a step that runs dist's install script runs: the script by its URL (the plan job), or
/// the command dist's plan gives each build.
const DIST_INSTALLS: [&str; 2] = ["/cargo-dist-installer.sh", "${{ matrix.install_dist.run }}"];
/// The environment, at the end of each step that runs dist's install script, in which every
/// curl reads `.github/curl/.curlrc`, so each download tries again while the server fails.
const RETRIES: &str = "        env:
          CURL_HOME: \"${{ github.workspace }}/.github/curl\"
";

/// Has dist write the workflow, then makes the three changes.
pub(crate) fn run(args: &[String]) -> Result<(), String> {
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
    let workflow = ship(&read(&workflow_path)?)?;
    write(&workflow_path, &workflow)?;
    println!("xtask: wrote {WORKFLOW}");
    Ok(())
}

/// Fails unless the checked-in workflow has all three changes.
pub(crate) fn check() -> Result<(), String> {
    let workflow = read(&workspace::root()?.join(WORKFLOW))?;
    as_shipped(&workflow)
        .map_err(|err| format!("{WORKFLOW}: {err}: run `cargo xtask release-workflow`"))?;
    println!(
        "xtask: in {WORKFLOW}, only the `{PUBLISHER}` job can write, the install script \
         checks every archive, and installing dist tries each download again"
    );
    Ok(())
}

/// dist's workflow with all three changes: its own permissions read-only, the step that makes
/// the install script check every archive, and curl's config for each step that installs dist.
fn ship(workflow: &str) -> Result<String, String> {
    let narrowed = workflow.replacen(DIST_PERMISSIONS, READ_ONLY, 1);
    let shipped = with_checksums(&with_retries(&narrowed))
        .and_then(|shipped| as_shipped(&shipped).map(|()| shipped))
        .map_err(|err| format!("dist's workflow has changed: {err}"))?;
    Ok(shipped)
}

/// Fails unless the workflow has all three changes.
fn as_shipped(workflow: &str) -> Result<(), String> {
    only_publisher_writes(workflow)?;
    dist_installs_retry(workflow)?;
    let upload = global_upload(workflow)?;
    let checked = workflow
        .get(..upload)
        .is_some_and(|before| before.ends_with(CHECKSUMS));
    if !checked || workflow.matches(CHECKSUMS).count() != 1 {
        return Err(
            "the job that writes the install script doesn't make it check every archive, \
             just before uploading it"
                .to_owned(),
        );
    }
    Ok(())
}

/// The workflow with [`CHECKSUMS`] just before the upload of the job that writes the install
/// script, unless it is there already.
fn with_checksums(workflow: &str) -> Result<String, String> {
    let upload = global_upload(workflow)?;
    let (before, after) = workflow
        .split_at_checked(upload)
        .ok_or_else(|| "the install script's upload is not where it was found".to_owned())?;
    if before.ends_with(CHECKSUMS) {
        return Ok(workflow.to_owned());
    }
    Ok(format!("{before}{CHECKSUMS}{after}"))
}

/// The workflow with [`RETRIES`] at the end of each step that runs dist's install script,
/// unless it is there already.
fn with_retries(workflow: &str) -> String {
    let mut out = String::with_capacity(workflow.len());
    for piece in pieces(workflow) {
        if !installs_dist(&piece) || piece.contains(RETRIES) {
            out.push_str(&piece);
            continue;
        }
        // Before the blank lines that may end it.
        let step = piece.trim_end();
        let after = piece.get(step.len()..).unwrap_or_default();
        out.push_str(step);
        out.push('\n');
        out.push_str(RETRIES);
        out.push_str(after.strip_prefix('\n').unwrap_or(after));
    }
    out
}

/// Fails unless each step that runs dist's install script, of which there is one at least, has
/// [`RETRIES`] as its one environment.
fn dist_installs_retry(workflow: &str) -> Result<(), String> {
    let installs: Vec<String> = pieces(workflow)
        .into_iter()
        .filter(|piece| installs_dist(piece))
        .collect();
    if installs.is_empty() {
        return Err("no step installs dist with its install script".to_owned());
    }
    let retried = installs
        .iter()
        .all(|step| step.matches(RETRIES).count() == 1 && step.matches(STEP_ENV).count() == 1);
    if !retried {
        return Err(
            "a step installs dist without curl's config that tries each download again \
             (.github/curl), as its one environment"
                .to_owned(),
        );
    }
    Ok(())
}

/// Whether `piece` of the workflow is a step that runs dist's install script.
fn installs_dist(piece: &str) -> bool {
    piece.starts_with(STEP) && DIST_INSTALLS.iter().any(|install| piece.contains(install))
}

/// The workflow in pieces that, joined, are the workflow again: each step (a line that starts
/// with [`STEP`], and the lines after it that are blank or start with [`STEP_LINE`]), and what
/// lies between them.
fn pieces(workflow: &str) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    let mut in_step = false;
    for line in workflow.split_inclusive('\n') {
        let starts = line.starts_with(STEP);
        let continues =
            in_step && !starts && (line.trim().is_empty() || line.starts_with(STEP_LINE));
        match pieces.last_mut() {
            Some(piece) if continues || !(starts || in_step) => piece.push_str(line),
            _ => pieces.push(line.to_owned()),
        }
        in_step = starts || continues;
    }
    pieces
}

/// Where the step starts that uploads what `build-global-artifacts` built, the install script
/// among it: the last [`GLOBAL_UPLOAD`] before [`GLOBAL_ARTIFACTS`], with no other step between.
fn global_upload(workflow: &str) -> Result<usize, String> {
    let missing = || "no step uploads what build-global-artifacts builds".to_owned();
    if workflow.matches(GLOBAL_ARTIFACTS).count() != 1 {
        return Err(missing());
    }
    let artifacts = workflow.find(GLOBAL_ARTIFACTS).ok_or_else(missing)?;
    let upload = workflow
        .get(..artifacts)
        .and_then(|before| before.rfind(GLOBAL_UPLOAD))
        .ok_or_else(missing)?;
    let between = workflow
        .get(upload + GLOBAL_UPLOAD.len()..artifacts)
        .ok_or_else(missing)?;
    if between.lines().any(|line| line.starts_with("      - ")) {
        return Err(missing());
    }
    Ok(upload)
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

    /// The shape of dist's workflow: its own permissions, triggers indented as jobs are, the plan
    /// and the local builds installing dist with its install script, both build jobs uploading
    /// what they built, and the publishing job asking for write access itself.
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
      - name: Install dist
        # we specify bash to get pipefail
        shell: bash
        run: "curl --proto '=https' --tlsv1.2 -LsSf https://github.com/axodotdev/cargo-dist/releases/download/v0.33.0/cargo-dist-installer.sh | sh"
      - run: dist plan
  build-local-artifacts:
    runs-on: ${{ matrix.runner }}
    steps:
      - name: Install dist
        run: ${{ matrix.install_dist.run }}

      # Get the dist-manifest
      - name: "Upload artifacts"
        uses: actions/upload-artifact@v7
        with:
          name: artifacts-build-local-${{ join(matrix.targets, '_') }}
  build-global-artifacts:
    runs-on: "ubuntu-22.04"
    steps:
      - id: cargo-dist
        run: dist build --artifacts=global
      - name: "Upload artifacts"
        uses: actions/upload-artifact@v7
        with:
          name: artifacts-build-global
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
        let shipped = ship(DIST_WORKFLOW).unwrap();
        assert!(shipped.starts_with("name: Release\npermissions:\n  \"contents\": \"read\"\n"));
        assert!(shipped.contains("      \"contents\": \"write\"\n      \"id-token\": \"write\""));
        assert_eq!(
            ship(&shipped),
            Ok(shipped.clone()),
            "shipping twice changes nothing"
        );
    }

    #[test]
    fn the_install_script_is_made_to_check_before_its_job_uploads_it() {
        let shipped = ship(DIST_WORKFLOW).unwrap();
        let global = format!(
            "        run: dist build --artifacts=global\n{CHECKSUMS}{GLOBAL_UPLOAD}        uses:"
        );
        assert!(shipped.contains(&global), "{shipped}");
        assert_eq!(
            shipped.matches(CHECKSUMS).count(),
            1,
            "not in the local builds"
        );
        assert_eq!(as_shipped(&shipped), Ok(()));
        let err = as_shipped(&shipped.replace(CHECKSUMS, "")).unwrap_err();
        assert!(err.contains("check every archive"), "{err}");
    }

    #[test]
    fn a_workflow_whose_install_script_upload_moved_fails() {
        let unnamed = DIST_WORKFLOW.replace(GLOBAL_ARTIFACTS, "          name: elsewhere\n");
        let err = ship(&unnamed).unwrap_err();
        assert!(err.contains("no step uploads"), "{err}");
        let other_step = DIST_WORKFLOW.replace(
            "        with:\n          name: artifacts-build-global",
            "      - run: true\n        with:\n          name: artifacts-build-global",
        );
        let err = ship(&other_step).unwrap_err();
        assert!(err.contains("no step uploads"), "{err}");
    }

    #[test]
    fn any_other_job_that_writes_fails() {
        let build_writes = DIST_WORKFLOW.replace(
            "    runs-on: ${{ matrix.runner }}\n",
            "    runs-on: ${{ matrix.runner }}\n    permissions:\n      \"contents\": \"write\"\n",
        );
        let err = ship(&build_writes).unwrap_err();
        assert!(
            err.contains("`build-local-artifacts` job can write"),
            "{err}"
        );
    }

    #[test]
    fn every_step_that_installs_dist_has_curl_try_each_download_again() {
        let shipped = ship(DIST_WORKFLOW).unwrap();
        let plan = format!("cargo-dist-installer.sh | sh\"\n{RETRIES}      - run: dist plan\n");
        assert!(shipped.contains(&plan), "{shipped}");
        let build =
            format!("        run: ${{{{ matrix.install_dist.run }}}}\n{RETRIES}\n      # Get the");
        assert!(shipped.contains(&build), "{shipped}");
        assert_eq!(shipped.matches(RETRIES).count(), 2, "nowhere else");
        assert_eq!(
            pieces(&shipped).concat(),
            shipped,
            "the pieces are the workflow"
        );
        let err = as_shipped(&shipped.replacen(RETRIES, "", 1)).unwrap_err();
        assert!(err.contains("tries each download again"), "{err}");
    }

    #[test]
    fn a_step_that_installs_dist_with_an_environment_of_its_own_fails() {
        let own = DIST_WORKFLOW.replace(
            "        run: ${{ matrix.install_dist.run }}\n",
            "        run: ${{ matrix.install_dist.run }}\n        env:\n          A: b\n",
        );
        let err = ship(&own).unwrap_err();
        assert!(err.contains("as its one environment"), "{err}");
    }

    #[test]
    fn a_workflow_that_installs_dist_some_other_way_fails() {
        let other = DIST_WORKFLOW
            .replace("/cargo-dist-installer.sh", "/installer.sh")
            .replace("${{ matrix.install_dist.run }}", "dist-install");
        let err = ship(&other).unwrap_err();
        assert!(err.contains("no step installs dist"), "{err}");
    }

    #[test]
    fn the_workflow_must_set_its_own_permissions() {
        let unset = DIST_WORKFLOW.replace("permissions:\n  \"contents\": \"write\"\n\n", "");
        let err = ship(&unset).unwrap_err();
        assert!(err.contains("not `contents: read`"), "{err}");
        let more = DIST_WORKFLOW.replace(
            "  \"contents\": \"write\"\n\n",
            "  \"contents\": \"write\"\n  \"id-token\": \"write\"\n\n",
        );
        let err = ship(&more).unwrap_err();
        assert!(
            err.contains("the workflow's own permissions write"),
            "{err}"
        );
    }
}
