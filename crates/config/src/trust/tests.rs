//! Project trust: nothing asked without commands to run, asked once and remembered, asked again
//! about what changes, off with no one to ask, and policy that trusts or forbids.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use glave_harness::ApprovalMode;
use glave_ports::block_on;

use super::*;
use crate::policy::ConfigFile;
use crate::policy::tests::constraints;
use crate::trust::assess::Standing;
use crate::trust::project::MCP_FILE;

/// A terminal no one answers.
async fn no_one(_question: String) -> std::io::Result<Option<String>> {
    Ok(None)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("glave-trust-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A repository at `dir` whose `.mcp.json` has `servers`, and whose config is `config`.
fn repository(dir: &Path, servers: &str, config: Option<&str>) -> Project {
    std::fs::create_dir_all(dir.join(".glave")).unwrap();
    std::fs::write(
        dir.join(MCP_FILE),
        format!(r#"{{"mcpServers": {{{servers}}}}}"#),
    )
    .unwrap();
    if let Some(config) = config {
        std::fs::write(dir.join(".glave/config.toml"), config).unwrap();
    }
    block_on!(Project::read(dir, &BTreeMap::new())).unwrap()
}

const DOCS: &str = r#""docs": {"command": "docs-mcp", "args": ["--fast"]}"#;

fn undecided(project: &Project, record: &Record, rules: &Constraints) -> usize {
    assess(project.clone(), record, rules).undecided().len()
}

#[test]
fn a_repository_with_nothing_that_runs_commands_never_asks() {
    let dir = scratch("nothing");
    let project = repository(&dir, "", Some("model = \"m\"\napproval = \"ask\"\n"));
    let assessment = assess(project, &Record::default(), &Constraints::default());
    assert_eq!(assessment.items.len(), 0);
    let outcome = assessment.apply(Answer::NoOne);
    assert!(outcome.rows.is_empty() && outcome.withheld.is_empty());
    let config = outcome.config.unwrap();
    assert_eq!(
        config.model.as_deref(),
        Some("m"),
        "applied without a question"
    );
    assert_eq!(config.approval, Some(ApprovalMode::Ask));
}

#[test]
fn settings_that_loosen_wait_for_trust_and_the_rest_apply_at_once() {
    let dir = scratch("loosen");
    let text = "model = \"m\"\napproval = \"auto\"\nsandbox = \"off\"\n";
    let project = repository(&dir, "", Some(text));
    let assessment = assess(project, &Record::default(), &Constraints::default());
    let outcome = assessment.clone().apply(Answer::NoOne);
    let config = outcome.config.unwrap();
    assert_eq!(config.model.as_deref(), Some("m"));
    assert_eq!((config.approval, config.sandbox), (None, None));
    assert_eq!(outcome.withheld.len(), 1);
    assert!(outcome.withheld[0].contains("approval = \"auto\", sandbox = \"off\""));
    let trusted = assessment.apply(Answer::Yes).config.unwrap();
    assert_eq!(trusted.approval, Some(ApprovalMode::Auto));
}

#[test]
fn a_project_cannot_let_web_fetch_reach_private_addresses_without_trust() {
    let dir = scratch("web");
    let text = "[web]\nprivate_addresses = true\nallow = [\"10.0.0.5\"]\ndeny = [\"x.com\"]\n";
    let project = repository(&dir, "", Some(text));
    let assessment = assess(project, &Record::default(), &Constraints::default());
    assert_eq!(assessment.undecided().len(), 1);
    let outcome = assessment.clone().apply(Answer::NoOne);
    let web = outcome.config.unwrap().web.unwrap();
    assert_eq!((web.private_addresses, web.allow.len()), (None, 0));
    assert_eq!(
        web.deny,
        ["x.com"],
        "a deny only narrows, so it applies at once"
    );
    assert!(outcome.withheld[0].contains("[web] private_addresses = true"));
    let trusted = assessment.apply(Answer::Yes).config.unwrap().web.unwrap();
    assert_eq!(trusted.private_addresses, Some(true));
    let narrowing = repository(&scratch("web-off"), "", Some("[web]\nfetch = false\n"));
    let assessment = assess(narrowing, &Record::default(), &Constraints::default());
    assert_eq!(assessment.items.len(), 0, "turning fetch off never asks");
}

#[test]
fn a_project_cannot_pick_its_own_judge_without_trust_but_may_turn_it_off() {
    let dir = scratch("judge");
    let rules = Constraints::default();
    let project = repository(&dir, "", Some("judge_model = \"lenient\"\n"));
    let assessment = assess(project, &Record::default(), &rules);
    assert_eq!(assessment.undecided().len(), 1);
    let outcome = assessment.clone().apply(Answer::NoOne);
    assert_eq!(outcome.config.unwrap().judge_model, None);
    assert!(
        outcome.withheld[0].contains("judge_model = \"lenient\""),
        "{:?}",
        outcome.withheld
    );
    let explained = assessment.clone().apply(Answer::Explaining);
    assert!(explained.rows[0][0].contains("judge_model"));
    let trusted = assessment.apply(Answer::Yes).config.unwrap();
    assert_eq!(trusted.judge_model.as_deref(), Some("lenient"));
    let off = repository(&scratch("judge-off"), "", Some("judge_model = \"off\"\n"));
    let assessment = assess(off, &Record::default(), &rules);
    assert_eq!(assessment.items.len(), 0, "never asks");
    let config = assessment.apply(Answer::NoOne).config.unwrap();
    assert_eq!(config.judge_model.as_deref(), Some("off"));
}

#[test]
fn asked_once_then_remembered_until_the_servers_change() {
    let dir = scratch("once");
    let rules = Constraints::default();
    let project = repository(&dir, DOCS, None);
    let mut record = Record::default();
    assert_eq!(undecided(&project, &record, &rules), 1);
    let outcome = assess(project.clone(), &record, &rules).apply(Answer::Yes);
    assert_eq!(outcome.servers.keys().collect::<Vec<_>>(), ["docs"]);
    record.take(Record::deciding(&project, outcome.decisions.unwrap()));
    // Remembered: not asked again.
    assert_eq!(undecided(&project, &record, &rules), 0);
    let again = assess(project, &record, &rules).apply(Answer::NoOne);
    assert_eq!(again.servers.len(), 1, "trusted with no one to ask");
    assert!(again.decisions.is_none(), "nothing new to keep");
    // The repository changes the server's command, and adds another: asked again.
    let changed =
        r#""docs": {"command": "docs-mcp", "args": ["--slow"]}, "extra": {"command": "x"}"#;
    let project = repository(&dir, changed, None);
    let assessment = assess(project, &record, &rules);
    let why: Vec<_> = assessment.undecided().iter().map(|(_, why)| *why).collect();
    assert_eq!(why.len(), 2);
    assert!(why.contains(&assess::Why::Changed) && why.contains(&assess::Why::New));
    insta::assert_snapshot!(
        "question",
        ask::question(&assessment)
            .replace(&assessment.project.root.display().to_string(), "/code/app")
    );
}

#[test]
fn a_no_is_remembered_too_and_with_no_one_there_servers_stay_off() {
    let dir = scratch("no");
    let rules = Constraints::default();
    let project = repository(&dir, DOCS, None);
    let mut record = Record::default();
    let none = assess(project.clone(), &record, &rules).apply(Answer::NoOne);
    assert!(none.servers.is_empty() && none.decisions.is_none());
    assert!(
        none.withheld[0].contains("nobody is here to ask"),
        "{:?}",
        none.withheld
    );
    let no = assess(project.clone(), &record, &rules).apply(Answer::No);
    assert!(no.servers.is_empty());
    record.take(Record::deciding(&project, no.decisions.unwrap()));
    let items = assess(project, &record, &rules).items;
    assert_eq!(items[0].1, Standing::Declined, "not asked again");
}

#[test]
fn an_app_is_shown_each_thing_in_plain_words_and_until_it_answers_they_stay_off() {
    let dir = scratch("plain");
    let rules = Constraints::default();
    let servers = r#""docs": {"command": "npx", "args": ["@acme/mcp"], "env": {"ACME_TOKEN": "${ACME_TOKEN}"}}, "web": {"url": "https://mcp.example.com/mcp", "headers": {"Authorization": "Bearer ${WEB_TOKEN}"}}"#;
    let config = format!(
        "approval = \"auto\"\nsandbox = \"off\"\njudge_model = \"lenient\"\n{GUARD}\n\
         [[hooks.Stop]]\ncommand = \"./done.sh\"\n"
    );
    let project = repository(&dir, servers, Some(&config));
    let assessment = assess(project, &Record::default(), &rules);
    let plain: Vec<(&str, Option<&str>)> = assessment
        .plain()
        .iter()
        .map(|plain| (plain.text, plain.detail.as_deref()))
        .collect();
    assert_eq!(
        plain,
        [
            (
                "Runs a tool server",
                Some("npx @acme/mcp (with ACME_TOKEN)")
            ),
            (
                "Connects to a tool server",
                Some("https://mcp.example.com/mcp (sending Authorization)")
            ),
            ("Runs a hook before each command", Some("./guard.sh")),
            ("Runs a hook when glave finishes a reply", Some("./done.sh")),
            ("Lets glave approve actions without asking", None),
            ("Turns off the sandbox", None),
            ("Chooses which model judges risky actions", Some("lenient")),
        ]
    );
    let later = assessment.apply(Answer::Later);
    assert!(later.servers.is_empty() && later.hooks.is_empty() && later.decisions.is_none());
    assert!(
        later.withheld[0].contains("the person has not trusted this project yet"),
        "{:?}",
        later.withheld
    );
    let config = later.config.unwrap();
    assert_eq!((config.approval, config.sandbox), (None, None));
}

#[test]
fn on_another_person_s_thread_every_server_and_setting_is_off_however_trusted() {
    let dir = scratch("all-off");
    let rules = Constraints::default();
    let project = repository(&dir, DOCS, Some(&format!("approval = \"auto\"\n{GUARD}")));
    let mut record = Record::default();
    let yes = assess(project.clone(), &record, &rules).apply(Answer::Yes);
    record.take(Record::deciding(&project, yes.decisions.unwrap()));
    let off = assess(project, &record, &rules).all_off("not theirs");
    assert_eq!(
        off,
        [
            "MCP server docs (docs-mcp --fast) is off: not theirs",
            "settings (approval = \"auto\") is off: not theirs",
        ],
        "never the hook: the model never calls one"
    );
}

#[test]
fn no_name_or_definition_makes_two_different_asks_read_the_same() {
    let rules = Constraints::default();
    let record = Record::default();
    // One server whose name spells out a second item, then the two items it spells.
    let one = r#""x\u0000{\"command\":\"evil\"}\nmcp:z": {"command": "ls"}"#;
    let two = r#""x": {"command": "evil"}, "z": {"command": "ls"}"#;
    let dir = scratch("collide");
    let first = assess(repository(&dir, one, None), &record, &rules).ask_id();
    let second = assess(repository(&dir, two, None), &record, &rules).ask_id();
    assert_ne!(first, second);
}

#[test]
fn trusting_what_was_shown_is_kept_as_a_yes_and_a_change_since_trusts_nothing() {
    let dir = scratch("asked");
    let rules = Constraints::default();
    let project = repository(&dir.join("repo"), DOCS, Some(GUARD));
    let mut record = Record::default();
    let shown = assess(project.clone(), &record, &rules);
    let asked = shown.ask_id();
    assert_eq!(asked.len(), 64);
    // The repository changes a server after the person was shown it: nothing is trusted.
    let changed = repository(
        &dir.join("repo"),
        r#""docs": {"command": "evil"}"#,
        Some(GUARD),
    );
    assert_eq!(trust_asked(&record, changed.clone(), &rules, &asked), None);
    // What was shown, trusted: kept exactly as the terminal keeps a yes.
    let kept = trust_asked(&record, project.clone(), &rules, &asked).unwrap();
    record.take(kept);
    let mut by_terminal = Record::default();
    let yes = assess(project.clone(), &Record::default(), &rules).apply(Answer::Yes);
    by_terminal.take(Record::deciding(&project, yes.decisions.unwrap()));
    assert_eq!(record, by_terminal);
    let now = assess(project.clone(), &record, &rules);
    assert_eq!(now.undecided().len(), 0);
    let on = now.apply(Answer::Later);
    assert_eq!((on.servers.len(), on.hooks.len()), (1, 1));
    // Answered already: the same ask trusts nothing more.
    assert_eq!(trust_asked(&record, project, &rules, &asked), None);
    // Only what changed is asked about next, under a new name.
    let next = assess(changed, &record, &rules);
    assert_eq!(next.undecided().len(), 1);
    assert_ne!(next.ask_id(), asked);
}

#[test]
fn policy_can_trust_a_project_or_forbid_its_servers() {
    let dir = scratch("policy");
    let project = repository(&dir, DOCS, Some("approval = \"auto\"\n"));
    let record = Record::default();
    let glob = format!("{}/*", project.root.parent().unwrap().display());
    let trusting = constraints(&format!("version = 1\n[projects]\ntrusted = [{glob:?}]\n"));
    assert_eq!(undecided(&project, &record, &trusting), 0);
    let outcome = assess(project.clone(), &record, &trusting).apply(Answer::NoOne);
    assert_eq!(outcome.servers.len(), 1);
    assert_eq!(outcome.config.unwrap().approval, Some(ApprovalMode::Auto));
    assert!(outcome.rows[0][1].contains("trusted by [projects] trusted"));
    // A project outside the globs is asked about as usual.
    let elsewhere = constraints("version = 1\n[projects]\ntrusted = [\"/srv/*\"]\n");
    assert_eq!(undecided(&project, &record, &elsewhere), 2);
    // Forbidding servers wins over a pre-approval and over the person's own yes.
    let forbidding = constraints(&format!(
        "version = 1\n[mcp]\nown = false\n[projects]\ntrusted = [{glob:?}]\n"
    ));
    let outcome = assess(project, &record, &forbidding).apply(Answer::Yes);
    assert!(outcome.servers.is_empty());
    assert!(
        outcome.withheld[0].contains("[mcp] own = false"),
        "{:?}",
        outcome.withheld
    );
    assert!(
        outcome.config.unwrap().approval.is_some(),
        "settings are not servers"
    );
}

#[test]
fn a_moved_clone_is_recognised_by_its_remote() {
    let first = scratch("moved-a");
    let second = scratch("moved-b");
    for dir in [&first, &second] {
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        let config =
            "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@example.com:acme/app.git\n";
        std::fs::write(dir.join(".git/config"), config).unwrap();
    }
    let rules = Constraints::default();
    let project = repository(&first, DOCS, None);
    assert_eq!(
        project.remote.as_deref(),
        Some("git@example.com:acme/app.git")
    );
    let mut record = Record::default();
    let outcome = assess(project.clone(), &record, &rules).apply(Answer::Yes);
    record.take(Record::deciding(&project, outcome.decisions.unwrap()));
    let moved = repository(&second, DOCS, None);
    assert_eq!(
        undecided(&moved, &record, &rules),
        0,
        "same remote, same server"
    );
    let other = repository(&second, r#""docs": {"command": "evil"}"#, None);
    assert_eq!(
        undecided(&other, &record, &rules),
        1,
        "trust is of the content"
    );
}

#[test]
fn resolve_keeps_nothing_when_no_one_is_asked() {
    let dir = scratch("resolve");
    let project = repository(&dir.join("repo"), DOCS, None);
    let rules = Constraints::default();
    let record = Record::default();
    for asking in [Asking::NoOne, Asking::Explaining] {
        let outcome = block_on!(resolve(&record, project.clone(), &rules, asking, no_one)).unwrap();
        assert!(outcome.servers.is_empty());
        assert!(outcome.decisions.is_none());
    }
    let explained = block_on!(resolve(
        &record,
        project,
        &rules,
        Asking::Explaining,
        no_one
    ))
    .unwrap();
    assert!(explained.rows[0][1].contains("glave will ask once"));
}

#[test]
fn a_moved_clone_s_new_decisions_replace_the_ones_its_remote_found() {
    let mut record = Record::default();
    let decided = |path: &str, trusted| Entry {
        path: path.to_owned(),
        remote: Some("git@example.com:acme/app.git".to_owned()),
        items: BTreeMap::from([(
            "mcp:docs".to_owned(),
            Decision {
                definition: "{}".to_owned(),
                trusted,
            },
        )]),
        by: None,
    };
    record.take(decided("/code/app", true));
    record.take(decided("/moved/app", false));
    assert_eq!(record.entries(), [decided("/moved/app", false)]);
    // No items: the project is forgotten.
    let mut forgotten = decided("/moved/app", true);
    forgotten.items.clear();
    record.take(forgotten);
    assert_eq!(record.entries(), []);
}

#[test]
fn an_earlier_trust_file_reads_as_it_was_written_and_another_version_is_refused() {
    let path = Path::new("/home/ada/.glave/trust.toml");
    let text = "version = 1\n\n[[project]]\npath = \"/code/app\"\n\n\
                [project.items.\"mcp:docs\"]\ndefinition = '{\"command\":\"docs-mcp\"}'\n\
                trusted = true\n";
    let entries = Record::earlier(text, path).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path, "/code/app");
    assert!(entries[0].items["mcp:docs"].trusted);
    let error = Record::earlier("version = 2\n", path)
        .unwrap_err()
        .to_string();
    assert!(error.contains("version = 2"), "{error}");
    let _ = ConfigFile::default();
}

#[test]
fn a_mistake_in_a_project_file_stops_glave_naming_it() {
    let dir = scratch("mistake");
    std::fs::write(dir.join(MCP_FILE), r#"{"mcpServers": {"a": {}}}"#).unwrap();
    let error = block_on!(Project::read(&dir, &BTreeMap::new())).unwrap_err();
    let error = error.chain();
    assert!(
        error.contains(".mcp.json") && error.contains("give a command"),
        "{error}"
    );
    std::fs::remove_file(dir.join(MCP_FILE)).unwrap();
    std::fs::create_dir_all(dir.join(".glave")).unwrap();
    std::fs::write(dir.join(".glave/config.toml"), "aproval = \"ask\"\n").unwrap();
    let error = block_on!(Project::read(&dir, &BTreeMap::new())).unwrap_err();
    assert!(error.to_string().contains("aproval"), "{error}");
}

const GUARD: &str = "[[hooks.PreToolUse]]\nmatcher = \"bash\"\ncommand = \"./guard.sh\"\n";

#[test]
fn project_hooks_stay_off_until_trusted_and_the_model_is_not_told() {
    let dir = scratch("hooks-off");
    let rules = Constraints::default();
    let project = repository(&dir, "", Some(&format!("model = \"m\"\n{GUARD}")));
    let assessment = assess(project, &Record::default(), &rules);
    assert_eq!(assessment.undecided().len(), 1);
    let off = assessment.clone().apply(Answer::NoOne);
    assert_eq!(off.hooks.len(), 0);
    assert_eq!(off.withheld.len(), 0, "the model never calls a hook");
    assert!(
        off.hooks_off[0].contains("hook PreToolUse (./guard.sh (for tools matching bash)) is off"),
        "{:?}",
        off.hooks_off
    );
    let config = off.config.unwrap();
    assert_eq!(
        config.model.as_deref(),
        Some("m"),
        "the rest applies at once"
    );
    assert_eq!(config.hooks, None, "hooks never travel as choices");
    let explained = assessment.clone().apply(Answer::Explaining);
    assert!(explained.rows[0][1].contains("glave will ask once"));
    let on = assessment.apply(Answer::Yes);
    assert_eq!(on.hooks.len(), 1);
    assert_eq!(on.hooks[0].command, "./guard.sh");
    assert_eq!(on.config.unwrap().hooks, None);
}

#[test]
fn a_trusted_hook_is_asked_about_again_when_it_changes() {
    let dir = scratch("hooks-change");
    let rules = Constraints::default();
    let project = repository(&dir, "", Some(GUARD));
    let mut record = Record::default();
    let outcome = assess(project.clone(), &record, &rules).apply(Answer::Yes);
    record.take(Record::deciding(&project, outcome.decisions.unwrap()));
    assert_eq!(undecided(&project, &record, &rules), 0, "asked once");
    let again = assess(project, &record, &rules).apply(Answer::NoOne);
    assert_eq!(again.hooks.len(), 1);
    let wider = GUARD.replace("\"bash\"", "\"bash|edit\"");
    let changed = repository(&dir, "", Some(&wider));
    let assessment = assess(changed, &record, &rules);
    let why: Vec<_> = assessment.undecided().iter().map(|(_, why)| *why).collect();
    assert_eq!(why, [assess::Why::Changed]);
    assert_eq!(assessment.apply(Answer::NoOne).hooks.len(), 0);
}

#[test]
fn policy_can_forbid_project_hooks_whatever_the_person_says() {
    let dir = scratch("hooks-forbid");
    let project = repository(&dir, "", Some(GUARD));
    let forbidding = constraints("version = 1\n[hooks]\nown = false\n");
    let outcome = assess(project, &Record::default(), &forbidding).apply(Answer::Yes);
    assert_eq!(outcome.hooks.len(), 0);
    assert!(
        outcome.hooks_off[0].contains("does not allow hooks a project brings"),
        "{:?}",
        outcome.hooks_off
    );
}

#[test]
fn a_hook_written_wrong_stops_glave_naming_the_file() {
    let dir = scratch("hooks-wrong");
    std::fs::create_dir_all(dir.join(".glave")).unwrap();
    for (text, says) in [
        (
            "[[hooks.PreToolUse]]\nmatcher = \"(\"\ncommand = \"x\"\n",
            "regular expression",
        ),
        (
            "[[hooks.Stop]]\nmatcher = \"bash\"\ncommand = \"x\"\n",
            "do not run for a tool",
        ),
        (
            "[[hooks.PreToolUse]]\ncommand = \"x\"\ntype = \"command\"\n",
            "type",
        ),
        ("[[hooks.BeforeTool]]\ncommand = \"x\"\n", "BeforeTool"),
    ] {
        std::fs::write(dir.join(".glave/config.toml"), text).unwrap();
        let error = block_on!(Project::read(&dir, &BTreeMap::new())).unwrap_err();
        let error = error.chain();
        assert!(
            error.contains("config.toml") && error.contains(says),
            "{error}"
        );
    }
}
