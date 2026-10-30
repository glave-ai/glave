//! `cargo xtask release-workflow`: writes `.github/workflows/release.yml`, the workflow dist
//! generates from `dist-workspace.toml` and `.github/build-setup.yml`, with the changes dist
//! has no setting for:
//!
//! - dist gives the whole workflow `contents: write`, so every job's token, the builds' too,
//!   could change the repository's releases and tags. Here the workflow's token only reads, and
//!   only the jobs that publish write, each only what it needs: the release, and glave's
//!   computer image (`.github/workflows/publish-computer-image.yml`), which also reads the
//!   repository and attests the image, where dist gives a publish job of the repository's own
//!   neither.
//! - dist's install script installs an archive unchecked where `sha256sum` is missing, as on
//!   macOS before 15. Here the job that writes it runs `.github/install-script-checksums.sh` on
//!   it before uploading it, so it checks every archive, with `shasum` where it must.
//!
//! `cargo xtask ci` checks both.

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
/// The jobs that publish, the only ones that may write, and what each may write: the release
/// and its provenance, and glave's computer image and its provenance.
const PUBLISHERS: &[(&str, &[&str])] = &[
    ("host", &["attestations", "contents", "id-token"]),
    (
        "custom-publish-computer-image",
        &["attestations", "id-token", "packages"],
    ),
];
/// The permissions dist gives a publish job of the repository's own (`publish-jobs`).
const DIST_PUBLISH: &str = "    # publish jobs get escalated permissions
    permissions:
      \"id-token\": \"write\"
      \"packages\": \"write\"
";
/// The permissions the computer image's publish job asks for, and is given instead.
const IMAGE_PUBLISH: &str =
    "    # What publish-computer-image.yml asks for: the image pushed and attested.
    permissions:
      \"attestations\": \"write\"
      \"contents\": \"read\"
      \"id-token\": \"write\"
      \"packages\": \"write\"
";
/// The upload of the job that writes the install script, which the step below goes before.
const GLOBAL_UPLOAD: &str = "      - name: \"Upload artifacts\"\n";
/// What names that job's upload, after [`GLOBAL_UPLOAD`].
const GLOBAL_ARTIFACTS: &str = "          name: artifacts-build-global\n";
/// The step that makes the install script check every archive it downloads.
const CHECKSUMS: &str = "      - name: \"The install script checks every archive it downloads\"
        run: \"sh .github/install-script-checksums.sh target/distrib/glave-installer.sh\"
";

/// Has dist write the workflow, then makes the two changes.
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

/// Fails unless the checked-in workflow has both changes.
pub(crate) fn check() -> Result<(), String> {
    let workflow = read(&workspace::root()?.join(WORKFLOW))?;
    as_shipped(&workflow)
        .map_err(|err| format!("{WORKFLOW}: {err}: run `cargo xtask release-workflow`"))?;
    println!(
        "xtask: in {WORKFLOW}, only the jobs that publish can write, each only what it needs, \
         and the install script checks every archive"
    );
    Ok(())
}

/// dist's workflow with the changes: its own permissions read-only, the image's publish job
/// given what it asks for, and the step that makes the install script check every archive.
fn ship(workflow: &str) -> Result<String, String> {
    let narrowed =
        workflow
            .replacen(DIST_PERMISSIONS, READ_ONLY, 1)
            .replacen(DIST_PUBLISH, IMAGE_PUBLISH, 1);
    let shipped = with_checksums(&narrowed)
        .and_then(|shipped| as_shipped(&shipped).map(|()| shipped))
        .map_err(|err| format!("dist's workflow has changed: {err}"))?;
    Ok(shipped)
}

/// Fails unless the workflow has the changes.
fn as_shipped(workflow: &str) -> Result<(), String> {
    only_publishers_write(workflow)?;
    if workflow.matches(IMAGE_PUBLISH).count() != 1 {
        return Err("the computer image's publish job is not given what it asks for".to_owned());
    }
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

/// Fails unless the workflow's own permissions are read-only and only the jobs that publish
/// are given write access, each only to what [`PUBLISHERS`] says.
fn only_publishers_write(workflow: &str) -> Result<(), String> {
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
        } else if let Some(permission) = written(line) {
            let may = PUBLISHERS
                .iter()
                .any(|(name, may)| job == Some(*name) && may.contains(&permission));
            if !may {
                return Err(match job {
                    Some(name) => format!(
                        "the `{name}` job can write `{permission}`; only the jobs that publish \
                         may write, each only what it needs"
                    ),
                    None => "the workflow's own permissions write; only the jobs that publish may"
                        .to_owned(),
                });
            }
        }
    }
    Ok(())
}

/// The permission a line gives write access to: `      "packages": "write"` gives `packages`.
fn written(line: &str) -> Option<&str> {
    let name = line.trim().strip_suffix(": \"write\"")?;
    Some(name.trim_matches('"'))
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

    /// The shape of dist's workflow: its own permissions, triggers indented as jobs are, both
    /// build jobs uploading what they built, and the publishing jobs asking for write access
    /// themselves.
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
    steps:
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
  custom-publish-computer-image:
    uses: ./.github/workflows/publish-computer-image.yml
    # publish jobs get escalated permissions
    permissions:
      "id-token": "write"
      "packages": "write"

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
    fn the_image_publisher_is_given_what_it_asks_for_and_no_more() {
        let shipped = ship(DIST_WORKFLOW).unwrap();
        assert!(shipped.contains(IMAGE_PUBLISH), "{shipped}");
        assert!(!shipped.contains(DIST_PUBLISH));
        let err = as_shipped(&shipped.replace(IMAGE_PUBLISH, DIST_PUBLISH)).unwrap_err();
        assert!(err.contains("not given what it asks for"), "{err}");
        let contents = shipped.replace(
            "      \"contents\": \"read\"\n      \"id-token\"",
            "      \"contents\": \"write\"\n      \"id-token\"",
        );
        let err = as_shipped(&contents).unwrap_err();
        assert!(
            err.contains("`custom-publish-computer-image` job can write `contents`"),
            "{err}"
        );
        let host_packages = shipped.replace(
            "      \"contents\": \"write\"\n",
            "      \"contents\": \"write\"\n      \"packages\": \"write\"\n",
        );
        let err = as_shipped(&host_packages).unwrap_err();
        assert!(err.contains("`host` job can write `packages`"), "{err}");
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
