//! How a project's settings change what its threads may do in a server that serves many
//! projects (`specs/policy.md`, "Project trust"): its mode, and its network. What only
//! narrows applies at once: a mode no wider than the server's own, and `network = false`.
//! Full access needs the person's trust, and the policy must leave it. A flag is nearer than a
//! project, as in the terminal, where the same rule settles a project's mode
//! ([`crate::policy::project_permissions`]).

use glave_ports::settings::Protections;

use crate::policy::{Choices, Constraints, Source, project_permissions};
use crate::trust::Outcome;

/// How `outcome`'s project changes what its threads may do in a server whose own settled
/// choices are `choices`. Until the person trusts its settings, the project's `config.toml`
/// carries only what narrows.
pub fn protections(outcome: &Outcome, choices: &Choices, constraints: &Constraints) -> Protections {
    let Some(config) = &outcome.config else {
        return Protections::default();
    };
    let flag = |from: &Source| matches!(from, Source::Flag(_));
    let (highest, _) = constraints.highest_permissions();
    let permissions = project_permissions(config.permissions, choices.permissions.value, highest)
        .filter(|_| !flag(&choices.permissions.from));
    Protections {
        permissions,
        offline: config.network == Some(false) && !flag(&choices.network.from),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use glave_event::Permissions::{self, Ask, Auto, FullAccess};

    use super::*;
    use crate::policy::tests::{constraints, defaults};
    use crate::policy::{Chosen, ConfigFile};

    fn project(permissions: Option<Permissions>, network: Option<bool>) -> Outcome {
        Outcome {
            config: Some(ConfigFile {
                permissions,
                network,
                ..ConfigFile::default()
            }),
            servers: std::collections::BTreeMap::default(),
            hooks: Vec::new(),
            withheld: Vec::new(),
            hooks_off: Vec::new(),
            decisions: None,
            rows: Vec::new(),
        }
    }

    /// A server's choices: Auto by default, unless `change` says.
    fn server(change: impl FnOnce(&mut Choices)) -> Choices {
        let mut choices = defaults();
        change(&mut choices);
        choices
    }

    fn chose(mode: Permissions, from: Source) -> impl FnOnce(&mut Choices) {
        move |choices| choices.permissions = Chosen { value: mode, from }
    }

    fn mine() -> Source {
        Source::File(PathBuf::from("/home/ada/.glave/config.toml"))
    }

    fn mode(outcome: &Outcome, choices: &Choices, rules: &Constraints) -> Option<Permissions> {
        protections(outcome, choices, rules).permissions
    }

    #[test]
    fn a_project_narrows_its_threads_at_once_and_never_widens_them_without_full_access() {
        let none = Constraints::default();
        assert_eq!(
            mode(&project(Some(Ask), None), &server(|_| {}), &none),
            Some(Ask)
        );
        assert_eq!(
            mode(
                &project(Some(Auto), None),
                &server(chose(FullAccess, mine())),
                &none
            ),
            Some(Auto),
            "narrower than the person's"
        );
        assert_eq!(
            mode(
                &project(Some(Auto), None),
                &server(chose(Ask, mine())),
                &none
            ),
            None,
            "wider than the person's: ignored"
        );
        assert_eq!(
            mode(
                &project(Some(FullAccess), None),
                &server(chose(Ask, mine())),
                &none
            ),
            Some(FullAccess),
            "trusted, it is there at all, and nearer than the person's file"
        );
        let untrusted = Outcome {
            config: None,
            ..project(Some(Ask), Some(false))
        };
        assert_eq!(
            protections(&untrusted, &server(|_| {}), &none),
            Protections::default()
        );
    }

    #[test]
    fn a_flag_or_a_policy_wins_over_a_trusted_project() {
        let none = Constraints::default();
        let flagged = server(|choices| {
            choices.permissions.from = Source::Flag("--permissions");
            choices.network.from = Source::Flag("--network or --no-network");
        });
        assert_eq!(
            protections(&project(Some(Ask), Some(false)), &flagged, &none),
            Protections::default(),
            "a flag is nearer"
        );
        let capped = constraints("version = 1\n[permissions]\nhighest = \"auto\"\n");
        assert_eq!(
            mode(&project(Some(FullAccess), None), &server(|_| {}), &capped),
            Some(Auto)
        );
        let offline = constraints("version = 1\n[sandbox]\nnetwork = false\n");
        assert_eq!(
            mode(&project(Some(FullAccess), None), &server(|_| {}), &offline),
            Some(Auto),
            "out of the sandbox it would have the network"
        );
        assert_eq!(
            protections(&project(None, Some(false)), &server(|_| {}), &none),
            Protections {
                permissions: None,
                offline: true,
            }
        );
    }
}
