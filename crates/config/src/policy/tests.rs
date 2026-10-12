//! Choices nearest-wins, constraints that only narrow, the two together, and the explanation.

use std::path::{Path, PathBuf};

use glave_harness::{ApprovalMode, SandboxLevel};

use super::*;
use crate::policy::choices::Judge;
use crate::policy::files::{PolicyFile, SandboxChoice};

const ORG: &str = "/etc/glave/policy.toml";
const SPACE: &str = "/srv/space/policy.toml";
const PERSON: &str = "/home/ada/.glave/config.toml";
const PROJECT: &str = "/code/app/.glave/config.toml";

/// The rules of one policy file at the organization's place.
pub(crate) fn constraints(text: &str) -> Constraints {
    Constraints::of(&[policy_at(ORG, text)])
}

fn policy_at(path: &str, text: &str) -> (PathBuf, PolicyFile) {
    (
        PathBuf::from(path),
        files::policy(text, Path::new(path)).unwrap(),
    )
}

fn config(text: &str) -> ConfigFile {
    files::config(text, Path::new(PERSON)).unwrap()
}

fn file(path: &str, text: &str) -> Layer {
    Layer {
        level: Level::File(PathBuf::from(path)),
        values: config(text),
    }
}

fn layers(flags: ConfigFile, env: ConfigFile, files: Vec<Layer>) -> Vec<Layer> {
    let mut all = vec![
        Layer {
            level: Level::Flags,
            values: flags,
        },
        Layer {
            level: Level::Env,
            values: env,
        },
    ];
    all.extend(files);
    all
}

fn settle(choices: &[Layer], policies: &[(PathBuf, PolicyFile)]) -> crate::Result<Choices> {
    assemble(choices, policies, None).settle()
}

fn file_source(path: &str) -> Source {
    Source::File(PathBuf::from(path))
}

#[test]
fn each_choice_comes_from_the_nearest_level_that_sets_it() {
    let flags = ConfigFile {
        approval: Some(ApprovalMode::Ask),
        ..ConfigFile::default()
    };
    let env = ConfigFile {
        model: Some("env-model".into()),
        ..ConfigFile::default()
    };
    let project = file(
        PROJECT,
        "model = \"project-model\"\nsandbox = \"read-only\"\nworktree = true\n",
    );
    let person = file(
        PERSON,
        "model = \"m\"\njudge_model = \"off\"\napproval = \"auto\"\nsandbox = \"project\"\nmcp = [\"github\"]\n",
    );
    let choices = choices::resolve(&layers(flags, env, vec![project, person]));
    assert_eq!(
        (choices.approval.value, &choices.approval.from),
        (ApprovalMode::Ask, &Source::Flag("--approval"))
    );
    let model = choices.model.unwrap();
    assert_eq!(
        (model.value.as_str(), model.from),
        ("env-model", Source::Env("GLAVE_MODEL"))
    );
    assert_eq!(choices.sandbox.value, Some(SandboxLevel::ReadOnly));
    assert_eq!(choices.sandbox.from, file_source(PROJECT));
    assert_eq!(choices.judge.value, Judge::Off);
    assert_eq!(choices.judge.from, file_source(PERSON));
    assert_eq!(choices.mcp.unwrap().from, file_source(PERSON));
    assert_eq!(
        (choices.worktree.value, choices.worktree.from),
        (true, file_source(PROJECT))
    );
}

#[test]
fn models_offered_come_from_the_nearest_file_that_lists_them() {
    let project = file(
        PROJECT,
        "[[models]]\nid = \"hope-12b\"\nname = \"Hope\"\nhope = true\n\n[[models]]\nid = \"small\"\n",
    );
    let person = file(PERSON, "[[models]]\nid = \"mine\"\n");
    let none = ConfigFile::default;
    let choices = choices::resolve(&layers(none(), none(), vec![project, person]));
    let models = choices.models.unwrap();
    assert_eq!(models.from, file_source(PROJECT));
    assert_eq!(
        models.value,
        [
            files::ModelEntry {
                id: "hope-12b".into(),
                name: Some("Hope".into()),
                hope: true,
            },
            files::ModelEntry {
                id: "small".into(),
                name: None,
                hope: false,
            },
        ]
    );
    let unknown = files::config("[[models]]\nid = \"x\"\nprice = 1\n", Path::new(PERSON));
    assert!(unknown.is_err(), "an unknown key is refused");
}

#[test]
fn with_nothing_set_every_choice_is_the_default() {
    let choices = choices::resolve(&layers(
        ConfigFile::default(),
        ConfigFile::default(),
        vec![],
    ));
    assert_eq!(choices.model, None);
    assert_eq!(
        (choices.approval.value, &choices.approval.from),
        (ApprovalMode::Auto, &Source::Default)
    );
    assert_eq!(
        (choices.judge.value, choices.judge.from),
        (Judge::Main, Source::Default)
    );
    assert_eq!(
        (choices.sandbox.value, choices.sandbox.from),
        (Some(SandboxLevel::Project), Source::Default),
        "the sandbox is on by default"
    );
    assert_eq!(
        (choices.network.value, choices.network.from),
        (true, Source::Default),
        "with the network, so installs and fetches work"
    );
    assert_eq!(choices.mcp, None);
    assert_eq!(
        (choices.worktree.value, choices.worktree.from),
        (false, Source::Default),
        "threads work in the folder itself unless asked"
    );
    assert_eq!(
        (choices.titles.value, choices.titles.from),
        (true, Source::Default),
        "a model titles threads unless turned off"
    );
    let off = choices::resolve(&layers(
        ConfigFile::default(),
        ConfigFile::default(),
        vec![file(PERSON, "titles = false\n")],
    ));
    assert_eq!(
        (off.titles.value, off.titles.from),
        (false, file_source(PERSON))
    );
    let flags_beat_files = choices::resolve(&layers(
        ConfigFile {
            sandbox: Some(SandboxChoice::Off),
            ..ConfigFile::default()
        },
        ConfigFile::default(),
        vec![file(PERSON, "sandbox = \"project\"\n")],
    ));
    assert_eq!(flags_beat_files.sandbox.value, None, "--sandbox off wins");
}

#[test]
fn the_judge_is_the_main_model_unless_named_or_off() {
    assert_eq!(Judge::named(" ").model("m").as_deref(), Some("m"));
    assert_eq!(Judge::named("small").model("m").as_deref(), Some("small"));
    assert_eq!(Judge::named("off").model("m"), None);
}

#[test]
fn unknown_keys_are_refused_naming_the_file_and_the_key() {
    let typo = files::config("modle = \"m\"\n", Path::new(PERSON)).unwrap_err();
    let shown = format!("{typo:#}");
    assert!(shown.contains(PERSON) && shown.contains("modle"), "{shown}");
    let nested = files::policy("version = 1\n[models]\nalow = []\n", Path::new(ORG));
    let shown = format!("{:#}", nested.unwrap_err());
    assert!(shown.contains(ORG) && shown.contains("alow"), "{shown}");
    let section = files::policy("version = 1\n[network]\nallow = []\n", Path::new(ORG));
    assert!(format!("{:#}", section.unwrap_err()).contains("network"));
    let mode = files::config("approval = \"bypass\"\n", Path::new(PERSON)).unwrap_err();
    assert!(format!("{mode:#}").contains("bypass"));
}

#[test]
fn a_policy_names_its_version_and_its_globs_compile() {
    assert!(format!("{:#}", files::policy("", Path::new(ORG)).unwrap_err()).contains("version"));
    let future = files::policy("version = 2\n", Path::new(ORG)).unwrap_err();
    assert!(format!("{future:#}").contains("version = 2"));
    let glob = files::policy("version = 1\n[models]\nallow = [\"a/[\"]\n", Path::new(ORG));
    assert!(format!("{:#}", glob.unwrap_err()).contains("not a glob"));
}

#[test]
fn model_allow_lists_intersect_and_a_deny_always_wins() {
    let rules = Constraints::of(&[
        policy_at(
            ORG,
            "version = 1\n[models]\nallow = [\"acme/*\", \"bedrock/*\"]\n",
        ),
        policy_at(
            SPACE,
            "version = 1\n[models]\nallow = [\"acme/*\"]\ndeny = [\"acme/old-*\"]\n",
        ),
    ]);
    assert_eq!(rules.refuses_model("acme/new"), None);
    let space = rules.refuses_model("bedrock/claude").unwrap();
    assert_eq!(
        space.from,
        Path::new(SPACE),
        "the space narrowed the organization's list"
    );
    let denied = rules.refuses_model("acme/old-1").unwrap();
    assert_eq!(
        denied.to_string(),
        format!("[models] deny = [\"acme/old-*\"] in {SPACE}")
    );
    let open = Constraints::of(&[policy_at(ORG, "version = 1\n[models]\ndeny = [\"x/*\"]\n")]);
    assert!(open.refuses_model("anything/else").is_none());
    assert!(open.refuses_model("x/y").is_some());
}

#[test]
fn approval_modes_intersect_and_none_left_is_an_error() {
    let rules = Constraints::of(&[
        policy_at(
            ORG,
            "version = 1\n[approval]\nmodes = [\"ask\", \"auto\"]\n",
        ),
        policy_at(SPACE, "version = 1\n[approval]\nmodes = [\"ask\"]\n"),
    ]);
    assert_eq!(rules.approval_modes().0, [ApprovalMode::Ask]);
    let empty = Constraints::of(&[
        policy_at(ORG, "version = 1\n[approval]\nmodes = [\"auto\"]\n"),
        policy_at(SPACE, "version = 1\n[approval]\nmodes = [\"ask\"]\n"),
    ]);
    let error = effective::settle(no_choices(), &empty)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("no approval mode is left") && error.contains(SPACE),
        "{error}"
    );
}

#[test]
fn the_strictest_judge_sandbox_and_mcp_rules_win() {
    let both = |org: &str, space: &str| {
        Constraints::of(&[
            policy_at(ORG, &format!("version = 1\n{org}")),
            policy_at(SPACE, &format!("version = 1\n{space}")),
        ])
    };
    let judge = both(
        "[approval]\njudge = \"allowed\"\n",
        "[approval]\njudge = \"required\"\n",
    );
    assert_eq!(
        judge.judge().unwrap().map(|(rule, _)| rule),
        Some(files::JudgeRule::Required)
    );
    let clash = both(
        "[approval]\njudge = \"off\"\n",
        "[approval]\njudge = \"required\"\n",
    );
    assert!(
        clash
            .judge()
            .unwrap_err()
            .to_string()
            .contains("contradict")
    );
    let sandbox = both(
        "[sandbox]\nrequired = false\n",
        "[sandbox]\nrequired = true\n",
    );
    assert_eq!(sandbox.sandbox_required().unwrap().from, Path::new(SPACE));
    let own = both("[mcp]\nown = true\n", "[mcp]\nown = false\n");
    assert_eq!(own.own_mcp_forbidden().unwrap().from, Path::new(SPACE));
    assert!(
        both("[mcp]\nown = true\n", "")
            .own_mcp_forbidden()
            .is_none()
    );
}

#[test]
fn limits_take_the_smallest_and_unenforced_rules_refuse_to_run() {
    let rules = Constraints::of(&[
        policy_at(ORG, "version = 1\n[budget]\nperson_per_day_usd = 20\n"),
        policy_at(SPACE, "version = 1\n[budget]\nperson_per_day_usd = 5.5\n"),
    ]);
    assert_eq!(rules.person_per_day_usd().unwrap().from, Path::new(SPACE));
    let refused = effective::settle(no_choices(), &rules)
        .unwrap_err()
        .to_string();
    assert!(refused.contains("not enforced yet"), "{refused}");
    assert!(refused.contains("person_per_day_usd = 5.5"), "{refused}");
    assert_eq!(rules.person_per_day_micros(), Some(5_500_000));
    assert!(
        rules.not_enforced_by_the_gateway().is_empty(),
        "a gateway serving many keeps daily budgets"
    );
}

#[test]
fn catalogs_are_enforced_and_narrow_one_another() {
    let catalog = Constraints::of(&[
        policy_at(
            ORG,
            "version = 1\n[mcp]\ncatalog = [\"github\", \"jira\"]\n",
        ),
        policy_at(
            SPACE,
            "version = 1\n[mcp]\ncatalog = [\"jira\", \"docs\"]\n",
        ),
    ]);
    assert!(effective::settle(no_choices(), &catalog).is_ok());
    let (names, rule) = catalog.mcp_catalog().unwrap();
    assert_eq!(names, ["jira"]);
    assert_eq!(rule.from, Path::new(ORG));
    assert!(Constraints::default().mcp_catalog().is_none());
}

#[test]
fn worker_limits_narrow_and_an_allow_list_refuses_to_run() {
    let rules = Constraints::of(&[
        policy_at(
            ORG,
            "version = 1\n[workers]\ncpu = 4\nmemory_gb = 8\nper_person = 3\n",
        ),
        policy_at(
            SPACE,
            "version = 1\n[workers]\ncpu = 2\nmax_minutes = 90\nnetwork = \"off\"\n",
        ),
    ]);
    let limits = rules.worker_limits();
    assert_eq!(
        (
            limits.cpus,
            limits.memory_gb,
            limits.max_minutes,
            limits.per_person
        ),
        (Some(2), Some(8), Some(90), Some(3))
    );
    assert_eq!(limits.network, Some(glave_ports::Network::Off));
    assert!(effective::settle(no_choices(), &rules).is_ok());
    let allowlist = constraints("version = 1\n[workers]\nnetwork = \"allowlist\"\n");
    let refused = effective::settle(no_choices(), &allowlist)
        .unwrap_err()
        .to_string();
    assert!(
        refused.contains("[workers] network = \"allowlist\""),
        "{refused}"
    );
    assert!(refused.contains("not enforced yet"), "{refused}");
    let typo = files::policy("version = 1\n[workers]\ncpus = 2\n", Path::new(ORG));
    assert!(typo.unwrap_err().to_string().contains("cpus"));
}

#[test]
fn where_threads_work_is_a_choice_and_the_backend_comes_from_config() {
    let chosen = layers(
        ConfigFile {
            on: Some(On::Cloud),
            ..ConfigFile::default()
        },
        ConfigFile::default(),
        vec![file(
            PERSON,
            "on = \"here\"\n[workers]\nbackend = \"docker\"\nglave = \"/opt/glave-linux\"\n",
        )],
    );
    let choices = choices::resolve(&chosen);
    assert_eq!(choices.on.value, On::Cloud);
    assert_eq!(choices.on.from, Source::Flag("--on"));
    let workers = choices.workers.unwrap();
    assert_eq!(workers.value.glave, Path::new("/opt/glave-linux"));
    assert_eq!(no_choices().on.value, On::Here, "here by default");
}

#[test]
fn placement_rules_narrow_where_threads_work() {
    let only_here = [policy_at(
        ORG,
        "version = 1\n[placement]\nallowed = [\"here\", \"laptop\"]\n",
    )];
    let cloud = [file(PERSON, "on = \"cloud\"\n")];
    let refused = settle(&cloud, &only_here).unwrap_err().to_string();
    assert_eq!(
        refused,
        format!(
            "threads working worker (on = cloud, from {PERSON}) are not allowed: [placement] \
             allowed = [\"here\", \"laptop\"] in {ORG}"
        )
    );
    assert_eq!(settle(&[], &only_here).unwrap().on.value, On::Here);
    let only_workers = [policy_at(
        ORG,
        "version = 1\n[placement]\nallowed = [\"worker\"]\n",
    )];
    let settled = settle(&[], &only_workers).unwrap();
    assert_eq!(settled.on.value, On::Cloud, "the default gives way");
    assert_eq!(settled.on.from, Source::Narrowed(PathBuf::from(ORG)));
    let here = [file(PERSON, "on = \"here\"\n")];
    assert!(settle(&here, &only_workers).is_err(), "a choice never does");
    let rules = constraints("version = 1\n[placement]\nallowed = [\"here\"]\n");
    assert!(rules.allows_placement(Placement::Here));
    assert!(!rules.allows_placement(Placement::Worker));
    assert!(constraints("version = 1\n").allows_placement(Placement::Laptop));
    let unknown = files::policy(
        "version = 1\n[placement]\nallowed = [\"moon\"]\n",
        Path::new(ORG),
    );
    assert!(unknown.is_err());
}

fn no_choices() -> Choices {
    choices::resolve(&[])
}

#[test]
fn a_choice_outside_the_constraints_is_an_error_naming_rule_and_source() {
    let ask_only = [policy_at(
        ORG,
        "version = 1\n[approval]\nmodes = [\"ask\"]\n",
    )];
    let chosen = layers(
        ConfigFile::default(),
        ConfigFile::default(),
        vec![file(PERSON, "approval = \"auto\"\n")],
    );
    let error = settle(&chosen, &ask_only).unwrap_err().to_string();
    assert_eq!(
        error,
        format!(
            "approval auto (from {PERSON}) is not allowed: [approval] modes = [\"ask\"] in {ORG}"
        )
    );
    let sandboxed = [policy_at(ORG, "version = 1\n[sandbox]\nrequired = true\n")];
    let off = layers(
        ConfigFile {
            sandbox: Some(SandboxChoice::Off),
            ..ConfigFile::default()
        },
        ConfigFile::default(),
        vec![],
    );
    let error = settle(&off, &sandboxed).unwrap_err().to_string();
    assert!(
        error.contains("sandbox off (from --sandbox)") && error.contains(ORG),
        "{error}"
    );
    let required = [policy_at(
        ORG,
        "version = 1\n[approval]\njudge = \"required\"\n",
    )];
    let no_judge = layers(
        ConfigFile::default(),
        ConfigFile {
            judge_model: Some("off".into()),
            ..ConfigFile::default()
        },
        vec![],
    );
    let error = settle(&no_judge, &required).unwrap_err().to_string();
    assert!(
        error.contains("judge_model off (from GLAVE_JUDGE_MODEL)"),
        "{error}"
    );
}

#[test]
fn only_defaults_give_way_to_what_the_policy_leaves() {
    let strict = [policy_at(
        ORG,
        "version = 1\n[approval]\nmodes = [\"ask\"]\njudge = \"off\"\n[sandbox]\nrequired = true\n",
    )];
    let choices = settle(&[], &strict).unwrap();
    let narrowed = Source::Narrowed(PathBuf::from(ORG));
    assert_eq!(
        (choices.approval.value, &choices.approval.from),
        (ApprovalMode::Ask, &narrowed)
    );
    assert_eq!(
        (choices.judge.value, &choices.judge.from),
        (Judge::Off, &narrowed)
    );
    assert_eq!(choices.sandbox.value, Some(SandboxLevel::Project));
    assert_eq!(narrowed.to_string(), format!("default, narrowed by {ORG}"));
}

#[test]
fn people_s_own_mcp_servers_are_refused_when_the_policy_forbids_them() {
    let forbidden = constraints("version = 1\n[mcp]\nown = false\n");
    let error = check_own_servers(&["github"], &forbidden)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("github") && error.contains("[mcp] own = false"),
        "{error}"
    );
    assert!(check_own_servers(&[], &forbidden).is_ok());
    assert!(check_own_servers(&["github"], &Constraints::default()).is_ok());
}

mod reading;
mod sandboxing;
