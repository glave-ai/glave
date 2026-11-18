use std::cell::Cell;
use std::process::Stdio;

use glave_ports::block_on;
use glave_ports::testing::CountingEntropy;
use glave_process::{Child, Processes, Program};

use super::*;

const BUBBLEWRAP: Refuses = Refuses {
    saying: "bwrap: ",
    passing: true,
};

/// What bubblewrap says when a place it hides went away as it mounted over it.
const GONE: &str = "bwrap: Can't bind mount /oldroot/e on /newroot/tmp/emacs1001: Unable to \
                    mount source on destination: No such file or directory";

/// `script` run by `sh` as a sandbox's wrapper would, with `args` as `$1`…, its error output
/// glave's.
async fn wrapper(script: &str, args: &[&str]) -> Child {
    let mut program = Program::new("sh");
    program
        .args(["-c", script, "sh"])
        .args(args)
        .no_input()
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    Processes::unrecorded().spawn(&mut program).await.unwrap()
}

/// How `child`'s start comes out, waiting at most `within`, for a launch whose line is `line`.
async fn start(child: &mut Child, line: &Line, refuses: Refuses, within: Duration) -> Start {
    let mut output = child.stderr.take().unwrap();
    wait(&mut output, line, refuses, within).await
}

#[test]
fn the_line_is_heard_after_more_than_is_kept_and_written_in_parts() {
    let line = Line::new(&CountingEntropy::starting_at(7)).unwrap();
    let whole = line.0.trim_end();
    let (first, rest) = whole.split_at(whole.len() / 2);
    let script = "head -c 5000 /dev/zero | tr '\\0' x >&2; printf '%s' \"$1\" >&2; \
                  printf '%s\\n' \"$2\" >&2; exec sleep 30";
    block_on!(async {
        let mut child = wrapper(script, &[first, rest]).await;
        let started = start(&mut child, &line, BUBBLEWRAP, WITHIN).await;
        assert_eq!(started, Start::Runs);
        assert!(child.try_wait().unwrap().is_none(), "still runs");
        child.stop().await;
    });
}

#[test]
fn bubblewrap_s_own_error_means_the_command_never_ran() {
    let line = Line::new(&CountingEntropy::starting_at(1)).unwrap();
    // Another launch's line, as a name bubblewrap echoes could carry: only this one's says
    // the command runs.
    let other = Line::new(&CountingEntropy::starting_at(2)).unwrap();
    block_on!(async {
        let script = "echo \"$1\" >&2; printf '%s' \"$2\" >&2; exit 1";
        let mut child = wrapper(script, &[GONE, &other.0]).await;
        let started = start(&mut child, &line, BUBBLEWRAP, WITHIN).await;
        let said = format!("{GONE}\n{}", other.0.trim_end());
        assert_eq!(started, Start::NeverRan { said, again: true });
        child.stop().await;
    });
}

#[test]
fn an_end_without_the_wrapper_s_own_words_is_the_command_s() {
    let line = Line::new(&CountingEntropy::starting_at(3)).unwrap();
    block_on!(async {
        // As a `BASH_ENV` that warns, then ends the shell before the line.
        let mut child = wrapper("echo 'a warning' >&2; exit 3", &[]).await;
        let started = start(&mut child, &line, BUBBLEWRAP, WITHIN).await;
        assert_eq!(started, Start::Runs);
        child.stop().await;
    });
}

#[test]
fn a_late_line_leaves_the_command_running() {
    let line = Line::new(&CountingEntropy::starting_at(4)).unwrap();
    block_on!(async {
        let mut child = wrapper("exec sleep 30", &[]).await;
        let within = Duration::from_millis(50);
        assert_eq!(
            start(&mut child, &line, BUBBLEWRAP, within).await,
            Start::Runs
        );
        assert!(
            child.try_wait().unwrap().is_none(),
            "never stopped as never started"
        );
        child.stop().await;
    });
}

/// Tries that come out as `outcomes` say, one after another, and how many were made.
fn tries(outcomes: Vec<Tried<u32>>) -> (Result<u32, ComputerError>, usize) {
    let made = Cell::new(0_usize);
    let mut outcomes = outcomes.into_iter();
    let tried = block_on!(until_started(|| {
        made.set(made.get() + 1);
        let next = outcomes.next().expect("no more tries than scripted");
        std::future::ready(Ok(next))
    }));
    (tried, made.get())
}

fn gone(n: u32) -> Tried<u32> {
    Tried::NeverRan {
        said: format!("bwrap: gone {n}"),
        again: true,
    }
}

#[test]
fn a_sandbox_bubblewrap_could_not_start_is_made_again_until_one_starts_the_command() {
    let (tried, made) = tries(vec![gone(1), gone(2), Tried::Runs(3)]);
    assert_eq!(tried.unwrap(), 3);
    assert_eq!(made, 3);
}

#[test]
fn bubblewrap_is_tried_three_times_at_most_and_the_failure_says_what_it_said_last() {
    let (tried, made) = tries(vec![gone(1), gone(2), gone(3), Tried::Runs(4)]);
    assert_eq!(made, SANDBOXES);
    let error = tried.unwrap_err();
    assert_eq!(
        error.message(),
        "the sandbox could not start the command: bwrap: gone 3"
    );
}

#[test]
fn a_command_that_runs_or_may_have_is_never_tried_again() {
    let (tried, made) = tries(vec![Tried::Runs(1), Tried::Runs(2)]);
    assert_eq!((tried.unwrap(), made), (1, 1));
}

#[test]
fn another_wrapper_s_refusal_fails_at_once_saying_why() {
    let refused = Tried::NeverRan {
        said: "glave sandbox: Landlock is off in this kernel".into(),
        again: false,
    };
    let (tried, made) = tries(vec![refused, Tried::Runs(2)]);
    assert_eq!(made, 1);
    assert_eq!(
        tried.unwrap_err().message(),
        "the sandbox could not start the command: glave sandbox: Landlock is off in this kernel"
    );
    let silent = Tried::NeverRan {
        said: String::new(),
        again: false,
    };
    let (tried, _) = tries(vec![silent]);
    assert_eq!(
        tried.unwrap_err().message(),
        "the sandbox could not start the command, and did not say why"
    );
}

#[test]
fn a_try_that_fails_ends_the_trying() {
    let made = Cell::new(0_usize);
    let tried: Result<u32, ComputerError> = block_on!(until_started(|| {
        made.set(made.get() + 1);
        std::future::ready(Err(ComputerError::Failed("no temp folder".into())))
    }));
    assert_eq!(tried.unwrap_err().message(), "no temp folder");
    assert_eq!(made.get(), 1);
}
