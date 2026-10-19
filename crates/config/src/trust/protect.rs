//! How a project's settings change the protections of its threads' commands in a server that
//! serves many projects (`specs/policy.md`, "Project trust"). What only tightens applies at
//! once: its `approval = "ask"`, `sandbox = "read-only"` and `network = false`. What loosens
//! needs the person's trust, its `approval = "auto"` and its `sandbox = "off"`, under the same
//! rules the terminal settles them by. A flag is nearer than a project, a policy that requires
//! the sandbox or leaves no auto wins, and commands run unconfined with nothing asked only when
//! someone chose both, or a policy leaves only auto. Where its commands run unconfined the mode
//! is always set, so a thread's own choice of auto, made behind the sandbox, never carries over
//! (`Protections::mode`).

use glave_event::ApprovalMode;
use glave_ports::computer::SandboxLevel;
use glave_ports::settings::Protections;

use crate::policy::{Choices, Constraints, SandboxChoice, Source};
use crate::trust::Outcome;

/// How `outcome`'s project changes its threads' protections in a server whose own settled
/// choices are `choices`, with its sandbox as it runs here. Until the person trusts its
/// settings, the project's `config.toml` carries only what tightens.
pub fn protections(outcome: &Outcome, choices: &Choices, constraints: &Constraints) -> Protections {
    let Some(config) = &outcome.config else {
        return Protections::default();
    };
    let flag = |from: &Source| matches!(from, Source::Flag(_));
    let chosen = |from: &Source| matches!(from, Source::Flag(_) | Source::File(_));
    let (allowed, _) = constraints.approval_modes();
    let off = config.sandbox == Some(SandboxChoice::Off)
        && !flag(&choices.sandbox.from)
        && constraints.sandbox_required().is_none();
    let mode = |mode| {
        config.approval == Some(mode) && !flag(&choices.approval.from) && allowed.contains(&mode)
    };
    let walled = choices.sandbox.value.is_some() && !off;
    let both = off || (choices.sandbox.value.is_none() && chosen(&choices.sandbox.from));
    let server_auto =
        choices.approval.value == ApprovalMode::Auto && chosen(&choices.approval.from);
    let only_auto = !allowed.contains(&ApprovalMode::Ask);
    let approval = if mode(ApprovalMode::Ask) {
        Some(ApprovalMode::Ask)
    } else if (mode(ApprovalMode::Auto) && (walled || both)) || (off && (server_auto || only_auto))
    {
        Some(ApprovalMode::Auto)
    } else if off {
        Some(ApprovalMode::Ask)
    } else {
        None
    };
    let read_only = config.sandbox == Some(SandboxChoice::ReadOnly) && !flag(&choices.sandbox.from);
    Protections {
        approval,
        unconfined: off && choices.sandbox.value.is_some(),
        level: read_only.then_some(SandboxLevel::ReadOnly),
        offline: config.network == Some(false) && !flag(&choices.network.from),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

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

    /// What `protections` changes of approval and the sandbox.
    fn loosened(
        outcome: &Outcome,
        choices: &Choices,
        rules: &Constraints,
    ) -> (Option<ApprovalMode>, bool) {
        let changed = protections(outcome, choices, rules);
        (changed.approval, changed.unconfined)
    }

    const AUTO: Option<ApprovalMode> = Some(ApprovalMode::Auto);
    const ASK: Option<ApprovalMode> = Some(ApprovalMode::Ask);
    const OFF: Option<SandboxChoice> = Some(SandboxChoice::Off);

    #[test]
    fn a_trusted_project_s_settings_loosen_its_threads_as_the_terminal_would() {
        let none = Constraints::default();
        assert_eq!(
            loosened(&project(AUTO, OFF), &server(|_| {}), &none),
            (AUTO, true)
        );
        assert_eq!(
            loosened(&project(None, OFF), &server(|_| {}), &none),
            (ASK, true),
            "out of the sandbox, it asks unless auto was chosen too"
        );
        let asking = server(|choices| {
            choices.approval = Chosen {
                value: ApprovalMode::Ask,
                from: Source::File(PathBuf::from("/home/ada/.glave/config.toml")),
            };
        });
        assert_eq!(
            loosened(&project(AUTO, None), &asking, &none),
            (AUTO, false),
            "nearer"
        );
        let mine = server(|choices| {
            choices.approval.from = Source::File(PathBuf::from("/home/ada/.glave/config.toml"));
        });
        assert_eq!(
            loosened(&project(None, OFF), &mine, &none),
            (AUTO, true),
            "auto chosen by the person, the sandbox off by the project: both chosen"
        );
        assert_eq!(
            protections(&project(None, None), &asking, &none),
            Protections::default()
        );
        // Untrusted, the project's config carries neither.
        let untrusted = Outcome {
            config: None,
            ..project(None, None)
        };
        assert_eq!(
            protections(&untrusted, &server(|_| {}), &none),
            Protections::default()
        );
    }

    #[test]
    fn what_only_tightens_applies_and_the_project_s_own_ask_is_nearer_than_the_person_s_auto() {
        let none = Constraints::default();
        let tight = outcome(ConfigFile {
            approval: ASK,
            sandbox: Some(SandboxChoice::ReadOnly),
            network: Some(false),
            ..ConfigFile::default()
        });
        let mine = server(|choices| {
            choices.approval.from = Source::File(PathBuf::from("/home/ada/.glave/config.toml"));
        });
        assert_eq!(
            protections(&tight, &mine, &none),
            Protections {
                approval: ASK,
                unconfined: false,
                level: Some(SandboxLevel::ReadOnly),
                offline: true,
            }
        );
        let flagged = server(|choices| {
            choices.approval.from = Source::Flag("--approval");
            choices.sandbox.from = Source::Flag("--sandbox");
            choices.network.from = Source::Flag("--network or --no-network");
        });
        assert_eq!(
            protections(&tight, &flagged, &none),
            Protections::default(),
            "a flag is nearer"
        );
        let auto_only = constraints("version = 1\n[approval]\nmodes = [\"auto\"]\n");
        assert_eq!(protections(&tight, &mine, &auto_only).approval, None);
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
            protections(&project(AUTO, OFF), &flagged, &none),
            Protections::default()
        );
        let required = constraints("version = 1\n[sandbox]\nrequired = true\n");
        assert_eq!(
            loosened(&project(AUTO, OFF), &server(|_| {}), &required),
            (AUTO, false)
        );
        let auto_only = constraints("version = 1\n[approval]\nmodes = [\"auto\"]\n");
        assert_eq!(
            loosened(&project(None, OFF), &server(|_| {}), &auto_only),
            (AUTO, true)
        );
        let asking_only = constraints("version = 1\n[approval]\nmodes = [\"ask\"]\n");
        assert_eq!(
            loosened(&project(AUTO, OFF), &server(|_| {}), &asking_only),
            (ASK, true)
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
        assert_eq!(
            protections(&project(AUTO, None), &unsandboxed, &none),
            Protections::default(),
            "auto gives way to ask"
        );
        assert_eq!(
            loosened(&project(AUTO, OFF), &unsandboxed, &none),
            (AUTO, false),
            "chose both; no sandbox to take away"
        );
    }
}
