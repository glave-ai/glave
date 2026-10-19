//! A server's settings read again under a policy that changed while it ran: a constraint added
//! narrows the next turn even when the person's file loosens what it constrains.

use std::path::Path;

use glave_config::policy::Locations;
use glave_harness::ApprovalMode;
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

fn runs_now(locations: &Locations, start: &Choices, glave_home: &Path) -> Runs {
    block_on!(async {
        let setup = reloaded(&Args::default(), locations, start).await.unwrap();
        runs(&setup, glave_home).unwrap()
    })
}

#[test]
fn a_constraint_added_while_it_runs_narrows_the_next_turn_whatever_the_file_loosens() {
    let (dir, locations) = place("narrowed");
    let loose = "approval = \"auto\"\nsandbox = \"off\"\nmemory = \"off\"\n";
    std::fs::write(&locations.person, loose).unwrap();
    let start = block_on!(async {
        let (setup, aside) = Setup::reload(&Args::default(), &locations).await.unwrap();
        assert_eq!(aside, Vec::<String>::new());
        setup.choices
    });
    let before = runs_now(&locations, &start, &dir);
    assert_eq!(before.defaults.approval, ApprovalMode::Auto);
    assert_eq!(before.defaults.sandbox, None);
    assert_eq!(before.modes, [ApprovalMode::Ask, ApprovalMode::Auto]);

    let strict = "version = 1\n[approval]\nmodes = [\"ask\"]\n[sandbox]\nrequired = true\n";
    std::fs::write(&locations.policies[0], strict).unwrap();
    // And the person's file loosens further: the network too.
    std::fs::write(&locations.person, format!("{loose}network = true\n")).unwrap();
    let after = runs_now(&locations, &start, &dir);
    assert_eq!(
        after.defaults.approval,
        ApprovalMode::Ask,
        "asks from now on"
    );
    assert_eq!(
        after.modes,
        [ApprovalMode::Ask],
        "and no thread may choose auto"
    );
    if glave_computer::sandbox_available() {
        let sandbox = after.defaults.sandbox.unwrap();
        assert_eq!(
            sandbox.level,
            SandboxLevel::Project,
            "the sandbox is back on"
        );
    } else {
        assert_eq!(after.defaults.sandbox, None, "it cannot run here");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_sandbox_asked_for_where_none_can_run_leaves_every_command_asking() {
    let (dir, locations) = place("unwalled");
    let chosen = "approval = \"auto\"\nsandbox = \"project\"\n";
    std::fs::write(&locations.person, chosen).unwrap();
    let start = block_on!(async {
        let (setup, _) = Setup::reload(&Args::default(), &locations).await.unwrap();
        setup.choices
    });
    let _ = std::fs::remove_dir_all(dir);
    let started = Choices {
        sandbox: Chosen {
            value: None,
            from: Source::Unavailable("no bubblewrap".to_owned()),
        },
        ..start.clone()
    };
    let walled = walled(start, &started, &Constraints::default());
    assert_eq!(walled.sandbox.value, None);
    assert_eq!(walled.approval.value, ApprovalMode::Ask);
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
