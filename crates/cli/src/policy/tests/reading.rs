//! The files read from their places, and the explanation of what they set.

use std::path::PathBuf;

use glave_harness::ApprovalMode;

use super::*;

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glave-policy-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn with_no_files_nothing_changes() {
    let dir = scratch("none");
    let locations = Locations::here(&dir.join("home"), &dir);
    let mut locations = locations;
    locations.policies = vec![dir.join("policy.toml")];
    let loaded = block_on(load(
        &locations,
        ConfigFile::default(),
        ConfigFile::default(),
    ))
    .unwrap();
    assert!(loaded.configs.is_empty() && loaded.policies.is_empty());
    assert_eq!(loaded.untrusted, None);
    assert_eq!(loaded.settle().unwrap(), no_choices());
    assert_eq!(loaded.constraints.rules(), []);
}

#[test]
fn files_are_read_from_their_places_and_a_project_s_waits_for_trust() {
    let dir = scratch("read");
    let home = dir.join("home");
    let project = dir.join("repo");
    std::fs::create_dir_all(home.clone()).unwrap();
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(project.join(".glave")).unwrap();
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(home.join("config.toml"), "approval = \"ask\"\n").unwrap();
    std::fs::write(project.join(".glave/config.toml"), "approval = \"auto\"\n").unwrap();
    let org = dir.join("policy.toml");
    std::fs::write(&org, "version = 1\n[sandbox]\nrequired = true\n").unwrap();
    let mut locations = Locations::here(&home, &project.join("src"));
    assert_eq!(locations.project, project.join(".glave/config.toml"));
    locations.policies = vec![org.clone()];
    let loaded = block_on(load(
        &locations,
        ConfigFile::default(),
        ConfigFile::default(),
    ))
    .unwrap();
    let choices = loaded.settle().unwrap();
    assert_eq!(
        choices.approval.value,
        ApprovalMode::Ask,
        "the person's, not the project's"
    );
    assert_eq!(choices.sandbox.from, Source::Narrowed(org));
    assert_eq!(loaded.untrusted, Some(project.join(".glave/config.toml")));
    std::fs::write(home.join("config.toml"), "aproval = \"ask\"\n").unwrap();
    let typo = block_on(load(
        &locations,
        ConfigFile::default(),
        ConfigFile::default(),
    ));
    assert!(format!("{:#}", typo.unwrap_err()).contains("aproval"));
}

#[test]
fn explain_shows_every_setting_and_rule_with_its_source() {
    let chosen = layers(
        ConfigFile {
            approval: Some(Approval::Ask),
            ..ConfigFile::default()
        },
        ConfigFile {
            model: Some("acme-large".into()),
            ..ConfigFile::default()
        },
        vec![file(
            PERSON,
            "judge_model = \"acme-small\"\nmcp = [\"github\"]\n",
        )],
    );
    let policies = [
        policy_at(
            ORG,
            "version = 1\n[models]\nallow = [\"acme/*\"]\ndeny = [\"acme/old-*\"]\n\
             [approval]\nmodes = [\"ask\", \"auto\"]\njudge = \"required\"\n\
             [sandbox]\nrequired = true\n[mcp]\nown = true\n",
        ),
        policy_at(SPACE, "version = 1\n[approval]\nmodes = [\"ask\"]\n"),
    ];
    let loaded = assemble(&chosen, &policies, Some(PathBuf::from(PROJECT)));
    let (text, runs) = explain(&loaded, Some("acme"));
    assert!(runs);
    insta::assert_snapshot!("explain", text);
    let (refused, runs) = explain(&loaded, Some("other"));
    assert!(!runs);
    insta::assert_snapshot!("explain_refused", refused);
    let empty = assemble(&[], &[], None);
    insta::assert_snapshot!("explain_defaults", explain(&empty, None).0);
}
