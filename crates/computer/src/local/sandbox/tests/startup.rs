//! What runs before a sandboxed command's shell says it started (`command/started.rs`): the
//! person's `BASH_ENV` and the functions their environment exports. However they keep the line
//! from coming, the command is never run again, and its ending is its own.

use std::path::Path;

use glave_ports::block_on;
use glave_ports::computer::Ending;

use super::{computers_saying, enforced, open_app, run, scratch};

/// Says what the shell was given and how it runs, so a failure shows it, appends a line to
/// `ran.txt` in the project, and says it is done.
const COUNTED: &str = "echo \"BASH_ENV=${BASH_ENV-} bash=$BASH_VERSION flags=$- \
    options=$SHELLOPTS ids=$(id -u):$(id -g)\"; echo ran >> ran.txt; echo done";

/// How many times the command ran in the project at `app`.
fn runs(app: &Path) -> usize {
    std::fs::read_to_string(app.join("ran.txt"))
        .unwrap_or_default()
        .lines()
        .count()
}

/// [`COUNTED`] in the sandbox, with `environment` in the computer's own; how it ended, what it
/// said and how many times it ran.
fn counted(name: &str, environment: &[(&str, &str)], bash_env: Option<&str>) -> Counted {
    let dir = scratch(name);
    let app = dir.join("app");
    let mut environment = environment.to_vec();
    let file = app.join("env.sh");
    let file = file.to_string_lossy().into_owned();
    if let Some(text) = bash_env {
        std::fs::write(&file, text).unwrap();
        environment.push(("BASH_ENV", &file));
    }
    block_on!(async {
        let (computers, sandbox) = computers_saying(&dir, &environment);
        let there = open_app(&computers, &dir).await;
        let (ending, said) = run(&computers, there, &sandbox, COUNTED).await;
        Counted {
            ending,
            said,
            runs: runs(&app),
            env_ran: std::fs::read_to_string(app.join("env-ran.txt"))
                .unwrap_or_default()
                .lines()
                .count(),
        }
    })
}

struct Counted {
    ending: Ending,
    said: String,
    runs: usize,
    env_ran: usize,
}

#[test]
fn a_bash_env_that_sends_the_shell_s_errors_away_leaves_the_command_run_once() {
    if !enforced() {
        return;
    }
    let ran = counted("bash-env-away", &[], Some("exec 2>/dev/null\n"));
    assert_eq!(ran.ending, Ending::Exited { code: 0 }, "{}", ran.said);
    assert!(ran.said.contains("done"), "{}", ran.said);
    assert_eq!(ran.runs, 1, "{}", ran.said);
}

#[test]
fn a_bash_env_that_ends_the_shell_runs_once_and_its_ending_is_the_command_s() {
    if !enforced() {
        return;
    }
    let ran = counted(
        "bash-env-exits",
        &[],
        Some("echo env >> env-ran.txt; exit 3\n"),
    );
    assert_eq!(ran.ending, Ending::Exited { code: 3 }, "{}", ran.said);
    assert_eq!((ran.env_ran, ran.runs), (1, 0), "{}", ran.said);
}

#[test]
fn exported_functions_never_run_a_command_again() {
    if !enforced() {
        return;
    }
    // One standing in for `printf`, which the line does not use, and one for `builtin`,
    // which keeps the line from coming.
    for (function, name) in [
        ("BASH_FUNC_printf%%", "printf"),
        ("BASH_FUNC_builtin%%", "builtin"),
    ] {
        let ran = counted(
            &format!("functions-{name}"),
            &[(function, "() { :; }")],
            None,
        );
        assert_eq!(
            ran.ending,
            Ending::Exited { code: 0 },
            "{name}: {}",
            ran.said
        );
        assert!(ran.said.contains("done"), "{name}: {}", ran.said);
        assert_eq!(ran.runs, 1, "{name}: {}", ran.said);
    }
}
