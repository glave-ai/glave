//! A server's settings read again under a policy that changed while it ran: a constraint added
//! narrows the next turn even when the person's file loosens what it constrains.

use std::path::Path;

use glave_config::policy::Locations;
use glave_harness::Permissions;
use glave_ports::block_on;
use glave_ports::computer::SandboxLevel;
use glave_ports::testing::TempDir;

use super::*;

/// The person's file and the organization's policy, fresh for the test `name`.
fn place(name: &str) -> (TempDir, Locations) {
    let dir = TempDir::new(&format!("reread-{name}")).unwrap();
    let locations = Locations {
        root: dir.to_path_buf(),
        person: dir.join("config.toml"),
        project: dir.join("none.toml"),
        policies: vec![dir.join("policy.toml")],
    };
    (dir, locations)
}

/// How threads run from now on, where the most a sandbox allows is the project.
fn runs_now(locations: &Locations, start: &Choices, glave_home: &Path) -> Runs {
    block_on!(async {
        let setup = reloaded(&Args::default(), locations, start).await.unwrap();
        runs(&setup, Some(SandboxLevel::Project), glave_home).unwrap()
    })
}

#[test]
fn a_constraint_added_while_it_runs_narrows_the_next_turn_whatever_the_file_loosens() {
    let (dir, locations) = place("narrowed");
    let loose = "permissions = \"full-access\"\nmemory = \"off\"\n";
    std::fs::write(&locations.person, loose).unwrap();
    let start = block_on!(async {
        let (setup, aside) = Setup::reload(&Args::default(), &locations).await.unwrap();
        assert_eq!(aside, Vec::<String>::new());
        setup.choices
    });
    let before = runs_now(&locations, &start, &dir);
    assert_eq!(before.defaults.permissions, Permissions::FullAccess);
    assert!(before.defaults.ask_network);
    assert_eq!(before.defaults.highest, Permissions::FullAccess, "no cap");
    let sandbox = before.defaults.sandbox.unwrap();
    assert_eq!(
        (sandbox.level, sandbox.network),
        (SandboxLevel::Project, true),
        "there for the threads that choose it"
    );

    let strict = "version = 1\n[permissions]\nhighest = \"ask\"\n[sandbox]\n\
                  network = false\nask_network = false\n";
    std::fs::write(&locations.policies[0], strict).unwrap();
    // And the person's file loosens further: the network too.
    std::fs::write(&locations.person, format!("{loose}network = true\n")).unwrap();
    let after = runs_now(&locations, &start, &dir);
    assert_eq!(
        after.defaults.permissions,
        Permissions::Ask,
        "asks from now on"
    );
    assert_eq!(
        after.defaults.highest,
        Permissions::Ask,
        "and no thread may run with more"
    );
    assert!(!after.defaults.ask_network, "no command may ask for it");
    assert!(
        !after.defaults.sandbox.unwrap().network,
        "without the network, whatever the file says"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn where_no_sandbox_ran_at_the_start_none_runs_after_and_the_choices_say_why() {
    let (dir, locations) = place("unwalled");
    std::fs::write(&locations.person, "permissions = \"auto\"\n").unwrap();
    let start = block_on!(async {
        let (setup, _) = Setup::reload(&Args::default(), &locations).await.unwrap();
        Choices {
            unsandboxed: Some("no bubblewrap".to_owned()),
            ..setup.choices
        }
    });
    let setup = block_on!(reloaded(&Args::default(), &locations, &start)).unwrap();
    assert_eq!(setup.choices.unsandboxed.as_deref(), Some("no bubblewrap"));
    let runs = runs(&setup, None, &dir).unwrap();
    assert_eq!(runs.defaults.sandbox, None, "every command asks");
    assert_eq!(runs.defaults.permissions, Permissions::Auto);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn while_the_models_stay_the_judge_never_names_one_they_do_not_serve() {
    let named = |model: &str| Judge::Model(model.to_owned());
    assert_eq!(kept(&named("k"), named("j")), named("j"), "the last stays");
    assert_eq!(kept(&named("k"), Judge::Off), Judge::Main, "never off");
    assert_eq!(
        kept(&Judge::Off, named("j")),
        Judge::Off,
        "off needs no model"
    );
    assert_eq!(kept(&Judge::Main, named("j")), Judge::Main);
}
