//! Safe by default: Workspace, commands in the sandbox wherever it runs, and asking where it
//! cannot, unless someone chose Full access.

use super::reading::{load_all, scratch};
use super::*;

fn flags(permissions: Permissions) -> ConfigFile {
    ConfigFile {
        permissions: Some(permissions),
        ..ConfigFile::default()
    }
}

/// `layers` and `policies` loaded on a computer where no sandbox can run.
fn unsandboxed(layers: &[Layer], policies: &[(PathBuf, PolicyFile)]) -> Loaded {
    let mut loaded = assemble(layers, policies, None);
    loaded.no_sandbox_here = Some("no bubblewrap".into());
    loaded
}

#[test]
fn by_default_the_agent_works_in_the_project_in_the_sandbox_with_the_network() {
    let choices = settle(&[], &[]).unwrap();
    assert_eq!(
        (choices.permissions.value, &choices.permissions.from),
        (Permissions::Workspace, &Source::Default)
    );
    assert!(choices.network.value);
    assert_eq!(choices.unsandboxed, None);
}

#[test]
fn where_the_sandbox_cannot_run_the_preset_stays_and_the_choices_say_why() {
    let defaults = unsandboxed(&[], &[]).settle().unwrap();
    assert_eq!(defaults.permissions.value, Permissions::Workspace);
    assert_eq!(defaults.unsandboxed.as_deref(), Some("no bubblewrap"));
    let chosen = layers(flags(Permissions::ReadOnly), ConfigFile::default(), vec![]);
    let kept = unsandboxed(&chosen, &[]).settle().unwrap();
    assert_eq!(
        (kept.permissions.value, kept.permissions.from),
        (Permissions::ReadOnly, Source::Flag("--permissions")),
        "a chosen preset gives way to asking, never to refusing"
    );
}

#[test]
fn a_policy_that_leaves_no_full_access_refuses_to_start_where_no_sandbox_runs() {
    let capped = [policy_at(
        ORG,
        "version = 1\n[permissions]\nhighest = \"workspace\"\n",
    )];
    let error = unsandboxed(&[], &capped).settle().unwrap_err().to_string();
    assert_eq!(
        error,
        format!(
            "the sandbox cannot run on this computer (no bubblewrap), and [permissions] \
             highest = \"workspace\" in {ORG} requires it"
        )
    );
    let (running, aside) = unsandboxed(&[], &capped).settle_giving_way().unwrap();
    assert_eq!(running.unsandboxed.as_deref(), Some("no bubblewrap"));
    assert_eq!(running.permissions.value, Permissions::Workspace);
    assert!(aside.is_empty(), "{aside:?}");
    let open = [policy_at(
        ORG,
        "version = 1\n[permissions]\nhighest = \"full-access\"\n",
    )];
    assert!(unsandboxed(&[], &open).settle().is_ok());
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
fn an_organization_takes_the_network_and_only_glave_s_default_gives_way() {
    let off = [policy_at(ORG, "version = 1\n[sandbox]\nnetwork = false\n")];
    let narrowed = settle(&[], &off).unwrap();
    assert_eq!(
        (narrowed.network.value, &narrowed.network.from),
        (false, &Source::Narrowed(PathBuf::from(ORG)))
    );
    // Only the sandbox keeps the network from commands, so it never gives way.
    let error = unsandboxed(&[], &off).settle().unwrap_err().to_string();
    assert!(error.contains("[sandbox] network = false"), "{error}");
    let on = layers(
        ConfigFile {
            network: Some(true),
            ..ConfigFile::default()
        },
        ConfigFile::default(),
        vec![],
    );
    let refused = settle(&on, &off).unwrap_err().to_string();
    assert!(
        refused.contains(&format!(
            "network on (from --network or --no-network) is not allowed: [sandbox] network = \
             false in {ORG}"
        )),
        "{refused}"
    );
    let unconfined = layers(
        flags(Permissions::FullAccess),
        ConfigFile::default(),
        vec![],
    );
    let refused = settle(&unconfined, &off).unwrap_err().to_string();
    assert!(
        refused.contains("permissions full-access")
            && refused.contains("[sandbox] network = false"),
        "{refused}"
    );
    assert_eq!(
        Constraints::of(&off).permissions_allowed(),
        [Permissions::ReadOnly, Permissions::Workspace],
        "no full access: out of the sandbox, commands would have the network"
    );
    let mine = layers(
        ConfigFile::default(),
        ConfigFile::default(),
        vec![file(PERSON, "network = false\n")],
    );
    assert_eq!(
        settle(&mine, &off).unwrap().network.from,
        file_source(PERSON),
        "choosing it off agrees"
    );
    // A running server cannot refuse: the person's own `network = true` gives way, said.
    let loose = layers(
        ConfigFile::default(),
        ConfigFile::default(),
        vec![file(PERSON, "network = true\n")],
    );
    let (choices, aside) = assemble(&loose, &off, None).settle_giving_way().unwrap();
    assert!(!choices.network.value);
    assert!(
        aside.len() == 1 && aside[0].contains(&format!("network on (from {PERSON})")),
        "{aside:?}"
    );
}

#[test]
fn taking_the_network_from_commands_takes_it_from_workers_too() {
    use glave_ports::computer::Network;
    let off = constraints("version = 1\n[sandbox]\nnetwork = false\n");
    assert_eq!(off.worker_limits().network, Some(Network::Off));
    let workers_on =
        constraints("version = 1\n[sandbox]\nnetwork = false\n[workers]\nnetwork = \"on\"\n");
    assert_eq!(
        workers_on.worker_limits().network,
        Some(Network::Off),
        "off wins"
    );
    assert_eq!(Constraints::default().worker_limits().network, None);
}

#[test]
fn asking_for_the_network_is_allowed_unless_a_rule_forbids_it() {
    let off = constraints("version = 1\n[sandbox]\nnetwork = false\n");
    assert_eq!(off.network_ask_forbidden(), None);
    let forbids = constraints("version = 1\n[sandbox]\nnetwork = false\nask_network = false\n");
    assert_eq!(
        forbids.network_ask_forbidden().map(ToString::to_string),
        Some(format!("[sandbox] ask_network = false in {ORG}"))
    );
    let allows = constraints("version = 1\n[sandbox]\nnetwork = true\nask_network = true\n");
    assert_eq!(allows, Constraints::default(), "true limits nothing");
}

#[test]
fn explain_says_when_the_sandbox_cannot_run_here_and_that_commands_ask() {
    let row = |text: &str| {
        let line = text
            .lines()
            .find(|line| line.trim_start().starts_with("permissions"));
        line.map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
    };
    let (text, runs) = explain(&unsandboxed(&[], &[]), None);
    assert!(runs, "{text}");
    assert_eq!(
        row(&text).as_deref(),
        Some(
            "permissions workspace, but the sandbox cannot run here (no bubblewrap), so every \
             change and command asks from default"
        )
    );
    let full = layers(
        flags(Permissions::FullAccess),
        ConfigFile::default(),
        vec![],
    );
    let (text, _) = explain(&unsandboxed(&full, &[]), None);
    assert_eq!(
        row(&text).as_deref(),
        Some("permissions full-access from --permissions"),
        "nothing to give way"
    );
}

#[test]
fn on_a_running_server_a_rule_added_narrows_what_the_person_s_file_loosens() {
    let loose = layers(
        ConfigFile::default(),
        ConfigFile::default(),
        vec![file(
            PERSON,
            "permissions = \"full-access\"\njudge_model = \"off\"\nmemory = \"on\"\n",
        )],
    );
    let strict = [policy_at(
        ORG,
        "version = 1\n[permissions]\nhighest = \"read-only\"\n[approval]\n\
         judge = \"required\"\n[memory]\nenabled = false\n",
    )];
    let loaded = assemble(&loose, &strict, None);
    assert!(loaded.settle().is_err(), "a start refuses");
    let (choices, aside) = loaded.settle_giving_way().unwrap();
    assert_eq!(choices.permissions.value, Permissions::ReadOnly);
    assert_eq!(choices.judge.value, Judge::Main);
    assert!(!memory_settings(&choices, &loaded.constraints).unwrap().on);
    assert_eq!(aside.len(), 3, "each said: {aside:?}");
    assert!(
        aside[0].contains(&format!("permissions full-access (from {PERSON})")),
        "{aside:?}"
    );

    // With nothing refused, nothing gives way.
    let (same, none) = assemble(&loose, &[], None).settle_giving_way().unwrap();
    assert_eq!(same.permissions.value, Permissions::FullAccess);
    assert_eq!(none, Vec::<String>::new());
}
