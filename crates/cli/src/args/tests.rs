use super::*;

fn parse_words(words: &[&str]) -> Result<Args, String> {
    parse(words.iter().map(|w| (*w).to_owned()))
}

#[test]
fn prompt_words_join_and_flags_parse() {
    let args = parse_words(&["--approval", "ask", "fix", "the", "--model", "m", "test"]).unwrap();
    assert_eq!(args.approval, Some(ApprovalMode::Ask));
    assert_eq!(args.model.as_deref(), Some("m"));
    assert_eq!(args.prompt, "fix the test");
}

#[test]
fn the_sandbox_takes_a_level_and_optional_network() {
    let serve = |words: &[&str]| parse_words(&[&["serve"], words].concat());
    let args = serve(&["--sandbox", "project", "--network"]).unwrap();
    assert_eq!(args.sandbox, Some(SandboxChoice::Project));
    assert_eq!(args.network, Some(true));
    let offline = serve(&["--no-network"]).unwrap();
    assert_eq!(offline.network, Some(false));
    assert_eq!(serve(&[]).unwrap().network, None);
    let off = serve(&["--sandbox", "off"]).unwrap();
    assert_eq!(off.sandbox, Some(SandboxChoice::Off));
    assert!(serve(&["--sandbox", "everything"]).is_err());
    assert!(serve(&["--sandbox"]).is_err());
    assert!(
        serve(&["--sandbox", "off", "--network"]).is_err(),
        "network needs a sandbox"
    );
    assert!(
        serve(&["--sandbox", "off", "--no-network"]).is_err(),
        "no network needs a sandbox"
    );
    assert!(
        serve(&["--network"]).is_ok(),
        "a config file may choose the sandbox"
    );
}

#[test]
fn how_glave_runs_is_the_server_s_to_choose_never_one_turn_s() {
    for flags in [
        &["--sandbox", "off"][..],
        &["--network"],
        &["--no-network"],
        &["--on", "cloud"],
        &["--max-steps", "3"],
    ] {
        let refused = parse_words(&[flags, &["go"]].concat()).unwrap_err();
        assert!(
            refused.contains("set it in config.toml"),
            "{flags:?}: {refused}"
        );
        let task = parse_words(&[flags, &["-b", "go"]].concat()).unwrap_err();
        assert!(task.contains("not one turn"), "{flags:?}: {task}");
    }
    let explained = parse_words(&["policy", "explain", "--sandbox", "read-only"]).unwrap();
    assert_eq!(explained.sandbox, Some(SandboxChoice::ReadOnly));
}

#[test]
fn a_worktree_is_asked_for_by_flag_and_off_otherwise() {
    assert!(parse_words(&["--worktree", "go"]).unwrap().worktree);
    assert!(!parse_words(&["go"]).unwrap().worktree);
}

#[test]
fn serve_is_a_subcommand_only_as_the_first_word() {
    let serve = parse_words(&["serve", "--stdio", "--approval", "auto"]).unwrap();
    assert_eq!(serve.action, Action::Serve);
    assert_eq!(serve.approval, Some(ApprovalMode::Auto));
    let person = parse_words(&["serve"]).unwrap();
    assert!(person.person_s_server() && !person.own_session);
    assert!(
        parse_words(&["serve", "--own-session"])
            .unwrap()
            .own_session
    );
    assert!(parse_words(&["serve", "--stdio", "--own-session"]).is_err());
    assert!(!serve.person_s_server(), "a program's own server");
    assert!(
        parse_words(&["--http", "8080"]).is_err(),
        "only serve takes --http"
    );
    assert!(parse_words(&["serve", "--stdio", "hello"]).is_err());
    assert!(
        parse_words(&["--stdio"]).is_err(),
        "only serve takes --stdio"
    );
    let prompt = parse_words(&["--", "serve", "the", "page"]).unwrap();
    assert_eq!(
        (prompt.action, prompt.prompt.as_str()),
        (Action::Run, "serve the page")
    );
}

#[test]
fn serving_over_http_needs_sign_in() {
    let https = [
        "serve",
        "--http",
        "8080",
        "--issuer",
        "https://id.example.com",
        "--audience",
        "glave",
    ];
    let serve = parse_words(&https).unwrap();
    assert_eq!(serve.action, Action::Serve);
    assert_eq!(serve.http, Some("127.0.0.1:8080".parse().unwrap()));
    assert_eq!(serve.issuers, ["https://id.example.com"]);
    assert_eq!(serve.audience.as_deref(), Some("glave"));
    let anywhere = parse_words(&[
        "serve",
        "--http",
        "0.0.0.0:9",
        "--issuer",
        "i",
        "--audience",
        "a",
    ]);
    assert_eq!(anywhere.unwrap().http, Some("0.0.0.0:9".parse().unwrap()));
    // Without OIDC, only a browser on this computer signs in, with the local token.
    let web = parse_words(&["serve", "--http", "8080", "--web", "dist/app"]).unwrap();
    assert_eq!(web.web, Some(PathBuf::from("dist/app")));
    assert!(parse_words(&["serve", "--stdio", "--web", "dist/app"]).is_err());
    assert!(parse_words(&["serve", "--http", "8080", "--web"]).is_err());
    let local = parse_words(&["serve", "--http", "8080"]).unwrap();
    assert_eq!(local.http, Some("127.0.0.1:8080".parse().unwrap()));
    assert!(parse_words(&["serve", "--http", "[::1]:8080"]).is_ok());
    let exposed = parse_words(&["serve", "--http", "0.0.0.0:9"]).unwrap_err();
    assert!(exposed.contains("--issuer"), "{exposed}");
    let half = parse_words(&["serve", "--http", "8080", "--issuer", "i"]).unwrap_err();
    assert!(half.contains("go together"), "{half}");
    assert!(parse_words(&["serve", "--http", "8080", "--stdio"]).is_err());
    assert!(parse_words(&["serve", "--stdio", "--audience", "a"]).is_err());
    assert!(parse_words(&["serve", "--http", "nowhere"]).is_err());
}

#[test]
fn approval_modes_parse() {
    assert_eq!(parse_words(&["hi"]).unwrap().approval, None);
    let ask = parse_words(&["--approval", "ask", "hi"]).unwrap();
    assert_eq!(ask.approval, Some(ApprovalMode::Ask));
    assert!(parse_words(&["--approval", "bypass"]).is_err());
    assert!(parse_words(&["--approval", "edits"]).is_err(), "retired");
    assert!(
        parse_words(&["--yes", "hi"]).is_err(),
        "auto is the default"
    );
    assert!(parse_words(&["--approval"]).is_err());
}

#[test]
fn max_steps_takes_a_count_of_at_least_one() {
    assert_eq!(parse_words(&["hi"]).unwrap().max_steps, None);
    let serve = parse_words(&["serve", "--stdio", "--max-steps", "1"]).unwrap();
    assert_eq!((serve.action, serve.max_steps), (Action::Serve, Some(1)));
    for wrong in ["0", "-3", "ten", "2.5", ""] {
        let error = parse_words(&["serve", "--max-steps", wrong]).unwrap_err();
        assert!(error.contains("use 1 or more"), "{wrong}: {error}");
    }
    assert!(
        parse_words(&["serve", "--max-steps"])
            .unwrap_err()
            .contains("needs a number")
    );
}

#[test]
fn skills_is_a_command_only_as_the_first_word() {
    assert_eq!(parse_words(&["skills"]).unwrap().action, Action::Skills);
    assert!(parse_words(&["skills", "now"]).is_err());
    assert_eq!(parse_words(&["commands"]).unwrap().action, Action::Commands);
    assert_eq!(parse_words(&["app"]).unwrap().action, Action::App);
    assert!(parse_words(&["app", "now"]).is_err());
    assert_eq!(
        parse_words(&["open", "the", "app"]).unwrap().action,
        Action::Run
    );
    assert!(parse_words(&["commands", "now"]).is_err());
    let prompt = parse_words(&["list", "skills"]).unwrap();
    assert_eq!(prompt.action, Action::Run);
    assert_eq!(prompt.prompt, "list skills");
}

#[test]
fn policy_explain_is_a_command_that_takes_the_choice_flags() {
    let explain = parse_words(&["policy", "explain", "--approval", "ask"]).unwrap();
    assert_eq!(explain.action, Action::Explain);
    assert_eq!(explain.approval, Some(ApprovalMode::Ask));
    assert!(parse_words(&["policy"]).is_err());
    assert!(parse_words(&["policy", "show"]).is_err());
    assert!(parse_words(&["policy", "explain", "why"]).is_err());
    let prompt = parse_words(&["explain", "policy"]).unwrap();
    assert_eq!(prompt.action, Action::Run);
}

#[test]
fn a_goal_takes_an_objective_and_optionally_a_budget() {
    let goal = parse_words(&["--goal", "make cargo test pass"]).unwrap();
    assert_eq!(goal.goal.as_deref(), Some("make cargo test pass"));
    assert_eq!(goal.goal_budget, None);
    let budgeted = parse_words(&["--goal", "x", "--goal-budget", "50000"]).unwrap();
    assert_eq!(budgeted.goal_budget, Some(50_000));
    let check = parse_words(&["--goal", "x", "--check", "cargo test"]);
    assert!(check.is_err(), "there is no check command");
    assert!(parse_words(&["--goal-budget", "5", "hi"]).is_err());
    assert!(parse_words(&["--goal", "x", "--goal-budget", "0"]).is_err());
    assert!(parse_words(&["--goal", "x", "and", "a", "prompt"]).is_err());
    assert!(parse_words(&["--goal", " "]).is_err());
    assert!(parse_words(&["serve", "--stdio", "--goal", "x"]).is_err());
}

#[test]
fn background_takes_a_prompt_or_a_goal_and_a_budget_only_for_a_turn() {
    let task = parse_words(&["-b", "--budget", "5000", "tidy", "up"]).unwrap();
    assert!(task.background);
    assert_eq!((task.budget, task.prompt.as_str()), (Some(5000), "tidy up"));
    let goal = parse_words(&["--background", "--goal", "pass", "--goal-budget", "9"]).unwrap();
    assert!(goal.background && goal.goal.is_some());
    assert!(parse_words(&["-b"]).is_err(), "nothing to do");
    assert!(
        parse_words(&["--budget", "5", "hi"]).is_err(),
        "only for a task"
    );
    assert!(parse_words(&["-b", "--goal", "x", "--budget", "5"]).is_err());
    assert!(parse_words(&["-b", "--continue", "hi"]).is_err());
    assert!(parse_words(&["-b", "--budget", "0", "hi"]).is_err());
    assert!(parse_words(&["-b", "--sandbox", "off", "hi"]).is_err());
    assert!(parse_words(&["-b", "--max-steps", "3", "hi"]).is_err());
    assert!(
        parse_words(&["serve", "--socket"]).is_err(),
        "the folder's task host is gone"
    );
}

#[test]
fn errors_are_explained() {
    assert!(parse_words(&["--model"]).is_err());
    assert!(parse_words(&["--bogus"]).is_err());
    assert_eq!(parse_words(&["--", "--yes"]).unwrap().prompt, "--yes");
}
