//! How a trusted project's settings loosen the commands of its threads in a server that serves
//! many projects (`specs/policy.md`, "Project trust"): its `approval = "auto"` and its
//! `sandbox = "off"`, under the same rules the terminal settles them by. A flag is nearer than
//! a project, a policy that requires the sandbox or leaves no auto wins, and commands run
//! unconfined with nothing asked only when someone chose both, or a policy leaves only auto.
//! Where its commands run unconfined the mode is always set, so a thread's own choice of auto,
//! made behind the sandbox, never carries over (`Loosened::mode`).

use glave_harness::{ApprovalMode, Loosened};

use crate::policy::{Choices, Constraints, SandboxChoice, Source};
use crate::trust::Outcome;

/// How `outcome`'s project loosens its threads' commands in a server whose own settled choices
/// are `choices`, with its sandbox as it runs here. Nothing unless the person trusted its
/// settings: until then the project's `config.toml` carries neither.
pub fn loosened(outcome: &Outcome, choices: &Choices, constraints: &Constraints) -> Loosened {
    let Some(config) = &outcome.config else {
        return Loosened::default();
    };
    let flag = |from: &Source| matches!(from, Source::Flag(_));
    let chosen = |from: &Source| matches!(from, Source::Flag(_) | Source::File(_));
    let (allowed, _) = constraints.approval_modes();
    let off = config.sandbox == Some(SandboxChoice::Off)
        && !flag(&choices.sandbox.from)
        && constraints.sandbox_required().is_none();
    let auto = config.approval == Some(ApprovalMode::Auto)
        && !flag(&choices.approval.from)
        && allowed.contains(&ApprovalMode::Auto);
    let walled = choices.sandbox.value.is_some() && !off;
    let both = off || (choices.sandbox.value.is_none() && chosen(&choices.sandbox.from));
    let server_auto =
        choices.approval.value == ApprovalMode::Auto && chosen(&choices.approval.from);
    let only_auto = !allowed.contains(&ApprovalMode::Ask);
    let approval = if (auto && (walled || both)) || (off && (server_auto || only_auto)) {
        Some(ApprovalMode::Auto)
    } else if off {
        Some(ApprovalMode::Ask)
    } else {
        None
    };
    Loosened {
        approval,
        unconfined: off && choices.sandbox.value.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use glave_harness::SandboxLevel;

    use super::*;
    use crate::policy::tests::{constraints, defaults};
    use crate::policy::{Chosen, ConfigFile};

    fn outcome(config: ConfigFile) -> Outcome {
        Outcome {
            config: Some(config),
            servers: std::collections::BTreeMap::default(),
            hooks: Vec::new(),
            withheld: Vec::new(),
            hooks_off: Vec::new(),
            decisions: None,
            rows: Vec::new(),
        }
    }

    fn project(approval: Option<ApprovalMode>, sandbox: Option<SandboxChoice>) -> Outcome {
        outcome(ConfigFile {
            approval,
            sandbox,
            ..ConfigFile::default()
        })
    }

    /// A server's choices: in the sandbox, in auto, both by default, unless `change` says.
    fn server(change: impl FnOnce(&mut Choices)) -> Choices {
        let mut choices = defaults();
        choices.sandbox = Chosen {
            value: Some(SandboxLevel::Project),
            from: Source::Default,
        };
        change(&mut choices);
        choices
    }

    const AUTO: Option<ApprovalMode> = Some(ApprovalMode::Auto);
    const OFF: Option<SandboxChoice> = Some(SandboxChoice::Off);

    #[test]
    fn a_trusted_project_s_settings_loosen_its_threads_as_the_terminal_would() {
        let none = Constraints::default();
        let both = loosened(&project(AUTO, OFF), &server(|_| {}), &none);
        assert_eq!((both.approval, both.unconfined), (AUTO, true));
        let off = loosened(&project(None, OFF), &server(|_| {}), &none);
        assert_eq!(
            (off.approval, off.unconfined),
            (Some(ApprovalMode::Ask), true),
            "out of the sandbox, it asks unless auto was chosen too"
        );
        let asking = server(|choices| {
            choices.approval = Chosen {
                value: ApprovalMode::Ask,
                from: Source::File(PathBuf::from("/home/ada/.glave/config.toml")),
            };
        });
        let auto = loosened(&project(AUTO, None), &asking, &none);
        assert_eq!((auto.approval, auto.unconfined), (AUTO, false), "nearer");
        let mine = server(|choices| {
            choices.approval.from = Source::File(PathBuf::from("/home/ada/.glave/config.toml"));
        });
        let chose = loosened(&project(None, OFF), &mine, &none);
        assert_eq!(
            (chose.approval, chose.unconfined),
            (AUTO, true),
            "auto chosen by the person, the sandbox off by the project: both chosen"
        );
        assert_eq!(
            loosened(&project(None, None), &asking, &none),
            Loosened::default()
        );
        // Untrusted, the project's config carries neither.
        let untrusted = Outcome {
            config: None,
            ..project(None, None)
        };
        assert_eq!(
            loosened(&untrusted, &server(|_| {}), &none),
            Loosened::default()
        );
    }

    #[test]
    fn a_flag_or_a_policy_wins_over_a_trusted_project() {
        let none = Constraints::default();
        let flagged = server(|choices| {
            choices.approval = Chosen {
                value: ApprovalMode::Ask,
                from: Source::Flag("--approval"),
            };
            choices.sandbox.from = Source::Flag("--sandbox");
        });
        assert_eq!(
            loosened(&project(AUTO, OFF), &flagged, &none),
            Loosened::default()
        );
        let required = constraints("version = 1\n[sandbox]\nrequired = true\n");
        let held = loosened(&project(AUTO, OFF), &server(|_| {}), &required);
        assert_eq!((held.approval, held.unconfined), (AUTO, false));
        let auto_only = constraints("version = 1\n[approval]\nmodes = [\"auto\"]\n");
        let only = loosened(&project(None, OFF), &server(|_| {}), &auto_only);
        assert_eq!((only.approval, only.unconfined), (AUTO, true));
        let asking_only = constraints("version = 1\n[approval]\nmodes = [\"ask\"]\n");
        let ask = loosened(&project(AUTO, OFF), &server(|_| {}), &asking_only);
        assert_eq!(
            (ask.approval, ask.unconfined),
            (Some(ApprovalMode::Ask), true)
        );
    }

    #[test]
    fn where_no_sandbox_runs_auto_needs_the_sandbox_turned_off_too() {
        let none = Constraints::default();
        let unsandboxed = server(|choices| {
            choices.sandbox = Chosen {
                value: None,
                from: Source::Unavailable("no Seatbelt".into()),
            };
            choices.approval = Chosen {
                value: ApprovalMode::Ask,
                from: Source::Unsandboxed,
            };
        });
        let alone = loosened(&project(AUTO, None), &unsandboxed, &none);
        assert_eq!(alone, Loosened::default(), "auto gives way to ask");
        let both = loosened(&project(AUTO, OFF), &unsandboxed, &none);
        assert_eq!(
            (both.approval, both.unconfined),
            (AUTO, false),
            "chose both; no sandbox to take away"
        );
    }
}
