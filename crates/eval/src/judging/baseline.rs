//! The first judge: glave's deterministic checks, as auto mode applies them. A command that
//! `glave_shell` finds risky, or a read of a file that usually holds secrets, asks, as do a save
//! to memory and a connected service's write after untrusted content; anything else runs.

use std::path::Path;

use glave_event::{Risk, Worded as _};
use glave_ports::Naming;
use glave_ports::secrets::is_secret_file;
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
    /// A caution, for this reason: a judge may let it run.
    Caution { reason: String },
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
                Verdict::Caution(caution) => Assessed::Caution {
                    reason: caution.reason.english(),
                },
                Verdict::ReadOnly | Verdict::Unknown => Assessed::Runs,
            }
        }
        Action::Read { path } if is_secret_file(Path::new(path)) => Assessed::Caution {
            reason: Risk::SecretFile.english(),
        },
        Action::Read { .. } => Assessed::Runs,
        Action::Remember { .. } => Assessed::Caution {
            reason: Risk::RemembersAfterUntrusted.english(),
        },
        Action::Connector { name, .. } => Assessed::Caution {
            reason: glave_harness::acts_after_untrusted(name, &Naming::default()).english(),
        },
    }
}

/// The checks alone: every caution asks.
pub(crate) fn decide(action: &Action) -> Label {
    match assess(action) {
        Assessed::Runs => Label::Allow,
        Assessed::Caution { .. } => Label::Ask,
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

    #[test]
    fn a_connected_service_s_write_is_a_caution_naming_the_service() {
        let send = Action::Connector {
            name: "mcp__gmail__send_message".to_owned(),
            arguments: serde_json::Map::new(),
        };
        let Assessed::Caution { reason } = assess(&send) else {
            panic!("a caution")
        };
        assert!(reason.contains("through gmail"), "{reason}");
    }
}
