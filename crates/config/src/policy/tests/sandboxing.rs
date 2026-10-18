//! Safe by default: the sandbox on wherever it runs, and commands asking where it cannot,
//! unless someone chose to run them unconfined.

use super::reading::{load_all, scratch};
use super::*;

fn flags(sandbox: Option<SandboxChoice>, approval: Option<ApprovalMode>) -> ConfigFile {
    ConfigFile {
        approval,
        sandbox,
        ..ConfigFile::default()
    }
}

#[test]
fn by_default_commands_run_sandboxed_with_the_network_and_nothing_asks() {
    let choices = settle(&[], &[]).unwrap();
    assert_eq!(choices.sandbox.value, Some(SandboxLevel::Project));
    assert!(choices.network.value);
    assert_eq!(
        (choices.approval.value, &choices.approval.from),
        (ApprovalMode::Auto, &Source::Default)
    );
}

#[test]
fn where_the_sandbox_cannot_run_commands_ask_never_run_unconfined() {
    let why = "no bubblewrap";
    let defaults = unsandboxed(settle(&[], &[]).unwrap(), why, &Constraints::default());
    assert_eq!(
        (defaults.sandbox.value, &defaults.sandbox.from),
        (None, &Source::Unavailable(why.into()))
    );
    assert_eq!(
        (defaults.approval.value, &defaults.approval.from),
        (ApprovalMode::Ask, &Source::Unsandboxed)
    );
    // Auto chosen for a sandboxed world is not a choice to run unconfined.
    let auto = layers(
        flags(None, Some(ApprovalMode::Auto)),
        ConfigFile::default(),
        vec![],
    );
    let chosen_auto = unsandboxed(settle(&auto, &[]).unwrap(), why, &Constraints::default());
    assert_eq!(chosen_auto.approval.value, ApprovalMode::Ask);
    // An organization that allows only auto has decided.
    let only_auto = constraints("version = 1\n[approval]\nmodes = [\"auto\"]\n");
    let org = unsandboxed(settle(&[], &[]).unwrap(), why, &only_auto);
    assert_eq!(org.approval.value, ApprovalMode::Auto);
}

#[test]
fn a_sandbox_someone_chose_or_a_policy_requires_never_gives_way() {
    let chosen = layers(
        flags(Some(SandboxChoice::Project), None),
        ConfigFile::default(),
        vec![],
    );
    let kept = unsandboxed(
        settle(&chosen, &[]).unwrap(),
        "none",
        &Constraints::default(),
    );
    assert_eq!(kept.sandbox.value, Some(SandboxLevel::Project));
    assert_eq!(kept.sandbox.from, Source::Flag("--sandbox"));
    let required = [policy_at(ORG, "version = 1\n[sandbox]\nrequired = true\n")];
    let settled = settle(&[], &required).unwrap();
    assert_eq!(settled.sandbox.from, Source::Narrowed(PathBuf::from(ORG)));
    let kept = unsandboxed(settled, "none", &Constraints::of(&required));
    assert_eq!(kept.sandbox.value, Some(SandboxLevel::Project));
}

#[test]
fn sandbox_off_asks_unless_auto_was_chosen_too() {
    let off = layers(
        flags(Some(SandboxChoice::Off), None),
        ConfigFile::default(),
        vec![],
    );
    let asks = settle(&off, &[]).unwrap();
    assert_eq!(asks.sandbox.value, None);
    assert_eq!(
        (asks.approval.value, &asks.approval.from),
        (ApprovalMode::Ask, &Source::Unsandboxed)
    );
    let both = layers(
        ConfigFile::default(),
        ConfigFile::default(),
        vec![file(PERSON, "sandbox = \"off\"\napproval = \"auto\"\n")],
    );
    let unconfined = settle(&both, &[]).unwrap();
    assert_eq!(unconfined.sandbox.value, None);
    assert_eq!(
        (unconfined.approval.value, &unconfined.approval.from),
        (ApprovalMode::Auto, &file_source(PERSON))
    );
    let required = [policy_at(ORG, "version = 1\n[sandbox]\nrequired = true\n")];
    let error = settle(&both, &required).unwrap_err().to_string();
    assert!(error.contains("sandbox off"), "{error}");
}

#[test]
fn the_network_is_chosen_like_the_rest_and_a_project_only_takes_it_away() {
    let offline = layers(
        ConfigFile {
            network: Some(false),
            ..ConfigFile::default()
        },
        ConfigFile::default(),
        vec![file(PERSON, "network = true\n")],
    );
    let choices = settle(&offline, &[]).unwrap();
    assert_eq!(
        (choices.network.value, &choices.network.from),
        (false, &Source::Flag("--network or --no-network"))
    );
    let dir = scratch("network");
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("config.toml"), "network = false\n").unwrap();
    let locations = Locations::here(&home, &dir);
    let project = |on: bool| ConfigFile {
        network: Some(on),
        ..ConfigFile::default()
    };
    let given_back = load_all(&locations, Some(project(true))).unwrap();
    assert!(
        !given_back.choices.network.value,
        "a project cannot give it"
    );
    std::fs::write(home.join("config.toml"), "").unwrap();
    let taken = load_all(&locations, Some(project(false))).unwrap();
    assert!(!taken.choices.network.value, "but it can take it away");
}

#[test]
fn explain_says_when_the_sandbox_cannot_run_here_and_that_commands_ask() {
    let mut loaded = assemble(&[], &[], None);
    loaded.no_sandbox_here = Some("no bubblewrap".into());
    let (text, runs) = explain(&loaded, None);
    assert!(runs, "{text}");
    assert!(
        text.contains("from default when commands run without a sandbox"),
        "{text}"
    );
    assert!(
        text.contains("from default, but it cannot run here: no bubblewrap"),
        "{text}"
    );
}

#[test]
fn a_thread_may_choose_auto_only_behind_a_wall_and_within_the_policy() {
    let both = [ApprovalMode::Ask, ApprovalMode::Auto];
    let none = Constraints::default();
    assert_eq!(thread_modes(&settle(&[], &[]).unwrap(), &none), both);
    let walled_off = unsandboxed(settle(&[], &[]).unwrap(), "no bubblewrap", &none);
    assert_eq!(thread_modes(&walled_off, &none), [ApprovalMode::Ask]);
    let unconfined = layers(
        flags(Some(SandboxChoice::Off), Some(ApprovalMode::Auto)),
        ConfigFile::default(),
        vec![],
    );
    let unconfined = settle(&unconfined, &[]).unwrap();
    assert_eq!(thread_modes(&unconfined, &none), both);
    let ask_only = "version = 1\n[approval]\nmodes = [\"ask\"]\n";
    let settled = settle(&[], &[policy_at(ORG, ask_only)]).unwrap();
    assert_eq!(
        thread_modes(&settled, &constraints(ask_only)),
        [ApprovalMode::Ask]
    );
}

#[test]
fn on_a_running_server_a_rule_added_narrows_what_the_person_s_file_loosens() {
    let loose = layers(
        ConfigFile::default(),
        ConfigFile::default(),
        vec![file(
            PERSON,
            "approval = \"auto\"\nsandbox = \"off\"\njudge_model = \"off\"\nmemory = \"on\"\n",
        )],
    );
    let strict = [policy_at(
        ORG,
        "version = 1\n[approval]\nmodes = [\"ask\"]\njudge = \"required\"\n[sandbox]\n\
         required = true\n[memory]\nenabled = false\n",
    )];
    let loaded = assemble(&loose, &strict, None);
    assert!(loaded.settle().is_err(), "a start refuses");
    let (choices, aside) = loaded.settle_giving_way().unwrap();
    assert_eq!(choices.approval.value, ApprovalMode::Ask);
    assert_eq!(choices.sandbox.value, Some(SandboxLevel::Project));
    assert_eq!(choices.judge.value, Judge::Main);
    assert!(!memory_settings(&choices, &loaded.constraints).unwrap().on);
    assert_eq!(aside.len(), 4, "each said: {aside:?}");
    assert!(
        aside[0].contains(&format!("approval auto (from {PERSON})")),
        "{aside:?}"
    );

    // With nothing refused, nothing gives way.
    let (same, none) = assemble(&loose, &[], None).settle_giving_way().unwrap();
    assert_eq!(same.approval.value, ApprovalMode::Auto);
    assert_eq!(same.sandbox.value, None);
    assert_eq!(none, Vec::<String>::new());
}
