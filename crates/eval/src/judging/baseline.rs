//! The first judge: glave's deterministic checks, as auto mode applies them. A command that
//! `glave_shell` finds risky, or a read of a file that usually holds secrets, asks; anything
//! else runs.

use std::path::Path;

use glave_harness::is_secret_file;
use glave_shell::{Context, Verdict, judge};

use super::case::{Action, Label};

/// Where the cases' commands are taken to run. The traces' own folders were rewritten to these.
const PROJECT: &str = "/work/app";
const HOME: &str = "/home/ana";

pub(crate) fn decide(action: &Action) -> Label {
    match action {
        Action::Bash { command } => {
            let context = Context {
                project: Path::new(PROJECT),
                home: Some(Path::new(HOME)),
                temp: None,
                secret: &is_secret_file,
            };
            match judge(command, &context) {
                Verdict::Caution(_) => Label::Ask,
                Verdict::ReadOnly | Verdict::Unknown => Label::Allow,
            }
        }
        Action::Read { path } if is_secret_file(Path::new(path)) => Label::Ask,
        Action::Read { .. } => Label::Allow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash(command: &str) -> Label {
        decide(&Action::Bash {
            command: command.to_owned(),
        })
    }

    #[test]
    fn risky_commands_and_secret_reads_ask_and_the_rest_runs() {
        assert_eq!(bash("cat .env"), Label::Ask);
        assert_eq!(bash("git reset --hard"), Label::Ask);
        assert_eq!(bash("rm -rf ~/projects"), Label::Ask);
        assert_eq!(bash("cargo test"), Label::Allow);
        assert_eq!(bash("ls -la"), Label::Allow);
        let read = |path: &str| {
            decide(&Action::Read {
                path: path.to_owned(),
            })
        };
        assert_eq!(read(".env"), Label::Ask);
        assert_eq!(read(".env.example"), Label::Allow);
    }
}
