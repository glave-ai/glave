//! The files read from their places, and the explanation of what they set.

use std::path::PathBuf;

use glave_harness::ApprovalMode;
use glave_ports::block_on;

use super::*;

pub(super) fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glave-policy-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Reads the policy files at `locations`, then everything else with `project` as the
/// project's choices.
pub(super) fn load_all(
    locations: &Locations,
    project: Option<ConfigFile>,
) -> crate::Result<Loaded> {
    block_on!(async {
        let policies = policies(locations).await?;
        load(
            locations,
            &policies,
            project,
            ConfigFile::default(),
            ConfigFile::default(),
        )
        .await
    })
}

#[test]
fn with_no_files_nothing_changes() {
    let dir = scratch("none");
    let mut locations = Locations::here(&dir.join("home"), &dir);
    locations.policies = vec![dir.join("policy.toml")];
    let loaded = load_all(&locations, None).unwrap();
    assert!(loaded.configs.is_empty() && loaded.policies.is_empty());
    assert_eq!(loaded.project, None);
    assert_eq!(loaded.settle().unwrap(), no_choices());
    assert_eq!(loaded.constraints.rules(), []);
}

#[test]
fn files_are_read_from_their_places_and_the_project_s_counts_once_trusted() {
    let dir = scratch("read");
    let home = dir.join("home");
    let project = dir.join("repo");
    std::fs::create_dir_all(home.clone()).unwrap();
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        home.join("config.toml"),
        "approval = \"ask\"\nmodel = \"mine\"\n",
    )
    .unwrap();
    let org = dir.join("policy.toml");
    std::fs::write(&org, "version = 1\n[sandbox]\nrequired = true\n").unwrap();
    let mut locations = Locations::here(&home, &project.join("src"));
    assert_eq!(locations.root, project);
    assert_eq!(locations.project, project.join(".glave/config.toml"));
    locations.policies = vec![org.clone()];
    let loaded = load_all(&locations, None).unwrap();
    let choices = loaded.settle().unwrap();
    assert_eq!(choices.approval.value, ApprovalMode::Ask, "the person's");
    assert_eq!(choices.sandbox.from, Source::Narrowed(org));
    let theirs = ConfigFile {
        approval: Some(ApprovalMode::Auto),
        ..ConfigFile::default()
    };
    let loaded = load_all(&locations, Some(theirs)).unwrap();
    let choices = loaded.settle().unwrap();
    assert_eq!(
        choices.approval.value,
        ApprovalMode::Auto,
        "nearer than the person's"
    );
    assert_eq!(
        choices.approval.from,
        Source::File(locations.project.clone())
    );
    assert_eq!(choices.model.map(|m| m.value).as_deref(), Some("mine"));
    assert_eq!(
        loaded.configs,
        [locations.project.clone(), locations.person.clone()]
    );
    std::fs::write(home.join("config.toml"), "aproval = \"ask\"\n").unwrap();
    let typo = load_all(&locations, None);
    assert!(format!("{:#}", typo.unwrap_err()).contains("aproval"));
}

#[test]
fn explain_shows_every_setting_and_rule_with_its_source() {
    let chosen = layers(
        ConfigFile {
            approval: Some(ApprovalMode::Ask),
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
    let trust = ProjectTrust {
        root: PathBuf::from("/code/app"),
        rows: vec![
            [
                "MCP server docs (docs-mcp --fast)".to_owned(),
                "trusted (recorded in trust.toml)".to_owned(),
            ],
            [
                "settings (approval = \"auto\")".to_owned(),
                "off until trusted: glave will ask once".to_owned(),
            ],
        ],
    };
    let loaded = assemble(&chosen, &policies, Some(trust));
    let (text, runs) = explain(&loaded, Some("acme"));
    assert!(runs);
    insta::assert_snapshot!("explain", text);
    let (refused, runs) = explain(&loaded, Some("other"));
    assert!(!runs);
    insta::assert_snapshot!("explain_refused", refused);
    let empty = assemble(&[], &[], None);
    insta::assert_snapshot!("explain_defaults", explain(&empty, None).0);
}

#[test]
fn the_person_s_hooks_are_read_shown_and_refused_when_the_policy_forbids_them() {
    let dir = scratch("hooks");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        "[[hooks.PostToolUse]]\nmatcher = \"edit|write\"\ncommand = \"cargo fmt\"\ntimeout = 30\n",
    )
    .unwrap();
    let mut locations = Locations::here(&home, &dir);
    locations.policies = vec![dir.join("policy.toml")];
    let loaded = load_all(&locations, None).unwrap();
    assert_eq!(loaded.person_hooks.len(), 1);
    assert_eq!(loaded.person_hooks[0].timeout_secs, Some(30));
    assert_eq!(
        loaded.choices,
        load_all(&Locations::here(&dir.join("none"), &dir), None)
            .unwrap()
            .choices,
        "hooks are not choices"
    );
    let (text, runs) = explain(&loaded, None);
    assert!(runs);
    assert!(
        text.contains("PostToolUse  cargo fmt (for tools matching edit|write)"),
        "{text}"
    );
    std::fs::write(
        dir.join("policy.toml"),
        "version = 1\n[hooks]\nown = false\n",
    )
    .unwrap();
    let forbidden = load_all(&locations, None).unwrap();
    let (text, runs) = explain(&forbidden, None);
    assert!(!runs);
    assert!(
        text.contains("your hooks are not allowed: [hooks] own = false"),
        "{text}"
    );
}

#[test]
fn only_the_person_s_own_file_chooses_a_gateway_and_only_a_safe_one() {
    let dir = scratch("gateway");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("config.toml"),
        "[gateway]\nurl = \"https://gw.example.com\"\n",
    )
    .unwrap();
    let mut locations = Locations::here(&home, &dir);
    locations.policies = vec![dir.join("policy.toml")];
    let theirs = files::config(
        "[gateway]\nurl = \"https://evil.example.com\"\n",
        &dir.join("project.toml"),
    )
    .unwrap();
    let loaded = load_all(&locations, Some(theirs)).unwrap();
    let gateway = loaded.settle().unwrap().gateway.unwrap();
    assert_eq!(
        gateway.value, "https://gw.example.com",
        "a project never chooses it"
    );
    assert_eq!(gateway.from, Source::File(home.join("config.toml")));
    let plain = files::config("[gateway]\nurl = \"http://gw.example.com\"\n", &home);
    assert!(plain.unwrap_err().to_string().contains("not https"));
    for local in [
        "http://127.0.0.1:8080",
        "http://localhost",
        "http://[::1]:9/",
    ] {
        let text = format!("[gateway]\nurl = \"{local}\"\n");
        assert!(files::config(&text, &home).is_ok(), "{local}");
    }
    let lookalike = files::config("[gateway]\nurl = \"http://localhost.evil.com\"\n", &home);
    assert!(lookalike.is_err());
    let typo = files::config("[gateway]\nurls = \"https://x\"\n", &home);
    assert!(typo.unwrap_err().to_string().contains("urls"));
}

#[test]
fn telemetry_goes_only_where_the_person_says_never_where_a_project_does() {
    let dir = scratch("telemetry");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut locations = Locations::here(&home, &dir);
    locations.policies = vec![];
    let project = files::config(
        "[telemetry]\notlp = \"https://collector.example.com\"\n",
        &locations.project,
    )
    .unwrap();
    let loaded = load_all(&locations, Some(project)).unwrap();
    assert_eq!(loaded.choices.telemetry, None, "a project's is ignored");
    std::fs::write(
        home.join("config.toml"),
        "[telemetry]\notlp = \"http://localhost:4318\"\n",
    )
    .unwrap();
    let loaded = load_all(&locations, None).unwrap();
    let chosen = loaded.choices.telemetry.unwrap();
    assert_eq!(chosen.value, "http://localhost:4318");
    assert_eq!(chosen.from, Source::File(home.join("config.toml")));
    let refused = files::config(
        "[telemetry]\notlp = \"collector:4318\"\n",
        &locations.person,
    );
    let refused = refused.unwrap_err().to_string();
    assert!(refused.contains("[telemetry] otlp"), "{refused}");
}

#[test]
fn the_server_to_sign_in_to_is_the_person_s_choice_alone() {
    let dir = scratch("server");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut locations = Locations::here(&home, &dir);
    locations.policies = vec![];
    let text = "[server]\nurl = \"https://glave.example.com\"\n";
    let project = files::config(text, &locations.project).unwrap();
    let loaded = load_all(&locations, Some(project)).unwrap();
    assert_eq!(loaded.choices.server, None, "a project's is ignored");
    std::fs::write(home.join("config.toml"), text).unwrap();
    let loaded = load_all(&locations, None).unwrap();
    let chosen = loaded.choices.server.as_ref().unwrap();
    assert_eq!(chosen.value, "https://glave.example.com");
    assert!(
        explain(&loaded, None)
            .0
            .contains("https://glave.example.com")
    );
    let plain = "[server]\nurl = \"http://glave.example.com\"\n";
    let refused = files::config(plain, &locations.person)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("[server] url"), "{refused}");
    assert!(files::config("[server]\nurl = \"http://127.0.0.1:8080\"\n", &home).is_ok());
}

#[test]
fn the_store_is_the_person_s_choice_and_its_password_never_shown() {
    let dir = scratch("store");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut locations = Locations::here(&home, &dir);
    locations.policies = vec![];
    let url = "postgres://glave:hunter2hunter2@db.example.com/glave";
    let text = format!("[store]\nurl = \"{url}\"\n");
    let project = files::config(&text, &locations.project).unwrap();
    let loaded = load_all(&locations, Some(project)).unwrap();
    assert_eq!(loaded.choices.store, None, "a project's is ignored");
    std::fs::write(home.join("config.toml"), &text).unwrap();
    let loaded = load_all(&locations, None).unwrap();
    assert_eq!(loaded.choices.store.as_ref().unwrap().value, url);
    let (shown, _) = explain(&loaded, None);
    assert!(
        shown.contains("postgres://glave:***@db.example.com/glave"),
        "{shown}"
    );
    assert!(!shown.contains("hunter2"), "{shown}");
    let wrong = "[store]\nurl = \"mysql://glave:hunter2hunter2@db\"\n";
    let refused = files::config(wrong, &locations.person)
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("[store] url") && !refused.contains("hunter2"),
        "{refused}"
    );
}

#[test]
fn the_github_app_is_the_person_s_or_server_s_choice() {
    let dir = scratch("github");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut locations = Locations::here(&home, &dir);
    locations.policies = vec![];
    let text = "[github]\nclient_id = \"Iv23li0123abcd\"\napp = \"glave-dev\"\n";
    let project = files::config(text, &locations.project).unwrap();
    let loaded = load_all(&locations, Some(project)).unwrap();
    assert_eq!(loaded.choices.github, None, "a project's is ignored");
    std::fs::write(home.join("config.toml"), text).unwrap();
    let loaded = load_all(&locations, None).unwrap();
    let chosen = loaded.choices.github.as_ref().unwrap();
    assert_eq!(chosen.value.client_id, "Iv23li0123abcd");
    assert_eq!(chosen.value.app.as_deref(), Some("glave-dev"));
    for refused in [
        "[github]\nclient_id = \"a b\"\n",
        "[github]\nclient_id = \"Iv1.x\"\nurl = \"http://github.example.com\"\n",
        "[github]\nclient_id = \"Iv1.x\"\nsecret = \"no\"\n",
        "[github]\nclient_id = \"Iv1.x\"\napp = \"https://github.com/apps/x\"\n",
    ] {
        let error = files::config(refused, &locations.person).unwrap_err();
        assert!(error.to_string().contains("github"), "{error}");
    }
    let local = "[github]\nclient_id = \"Iv1.x\"\nurl = \"http://127.0.0.1:9000\"\n";
    assert!(files::config(local, &home).is_ok());
}

#[test]
fn a_cloud_provider_is_the_person_s_or_server_s_choice_and_typed() {
    let dir = scratch("provider");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let mut locations = Locations::here(&home, &dir);
    locations.policies = vec![];
    let azure = "[provider]\nname = \"azure\"\nresource = \"acme-openai\"\n";
    let project = files::config(azure, &locations.project).unwrap();
    let loaded = load_all(&locations, Some(project)).unwrap();
    assert_eq!(loaded.choices.provider, None, "a project's is ignored");
    std::fs::write(home.join("config.toml"), azure).unwrap();
    let loaded = load_all(&locations, None).unwrap();
    let chosen = loaded.choices.provider.as_ref().unwrap();
    assert_eq!(
        chosen.value,
        ProviderFile::Azure {
            resource: "acme-openai".into(),
            scope: None
        }
    );
    assert_eq!(chosen.value.name(), "azure");
    let (shown, _) = explain(&loaded, None);
    assert!(shown.contains("azure (acme-openai)"), "{shown}");
    let bedrock = "[provider]\nname = \"bedrock\"\nregion = \"us-east-1\"\nendpoint = \"mantle\"\n";
    let read = files::config(bedrock, &home).unwrap().provider.unwrap();
    assert_eq!(
        read.describe(),
        "bedrock (us-east-1, mantle, API by model family)"
    );
    let vertex = "[provider]\nname = \"vertex\"\nproject = \"acme-ai\"\napi = \"anthropic\"\n";
    let read = files::config(vertex, &home).unwrap().provider.unwrap();
    assert_eq!(read.name(), "vertex");
    assert_eq!(read.describe(), "vertex (acme-ai, global, api = anthropic)");
    std::fs::write(home.join("config.toml"), vertex).unwrap();
    let (shown, _) = explain(&load_all(&locations, None).unwrap(), Some("vertex"));
    assert!(
        shown.contains("vertex (acme-ai, global, api = anthropic)"),
        "{shown}"
    );
    for wrong in [
        "[provider]\nname = \"vertex\"\n",
        "[provider]\nname = \"azure\"\n",
        "[provider]\nname = \"azure\"\nresource = \"a\"\nregion = \"us-east-1\"\n",
        "[provider]\nname = \"bedrock\"\nregion = \"us-east-1\"\nendpoint = \"edge\"\n",
        "[provider]\nname = \"vertex\"\nproject = \"acme-ai\"\napi = \"gemini\"\n",
        "[provider]\nname = \"vertex\"\nproject = \"acme-ai\"\nregion = \"us-east5\"\n",
    ] {
        let refused = files::config(wrong, &locations.person)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("config.toml"), "{wrong}: {refused}");
    }
}
