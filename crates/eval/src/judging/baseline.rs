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

/// What the deterministic checks make of an action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Assessed {
    /// No caution: it runs.
    Runs,
    /// A firm caution: it always asks.
    Asks,
    /// A soft caution, for this reason: a judge may let it run.
    Soft { reason: String },
}

pub(crate) fn assess(action: &Action) -> Assessed {
    match action {
        Action::Bash { command } => {
            let context = Context {
                project: Path::new(PROJECT),
                home: Some(Path::new(HOME)),
                temp: None,
                secret: &is_secret_file,
            };
            match judge(command, &context) {
                Verdict::Caution(caution) if caution.kind.firm() => Assessed::Asks,
                Verdict::Caution(caution) => Assessed::Soft {
                    reason: caution.reason,
                },
                Verdict::ReadOnly | Verdict::Unknown => Assessed::Runs,
            }
        }
        Action::Read { path } if is_secret_file(Path::new(path)) => Assessed::Asks,
        Action::Read { .. } => Assessed::Runs,
        Action::Remember { .. } => Assessed::Soft {
            reason: glave_harness::REMEMBER_CAUTION.to_owned(),
        },
    }
}

/// The checks alone: every caution asks.
pub(crate) fn decide(action: &Action) -> Label {
    match assess(action) {
        Assessed::Runs => Label::Allow,
        Assessed::Asks | Assessed::Soft { .. } => Label::Ask,
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
