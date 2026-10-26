//! What the agent may do in a thread: the mode it runs with, as its project changes it, and
//! the sandbox that mode comes to on this computer (`specs/approvals.md`, "Modes"). The
//! harness applies them; project trust decides what a project changes.

use glave_event::Permissions;

use crate::computer::{Sandbox, SandboxLevel};

/// How a project's settings change what its threads may do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Protections {
    /// The mode its threads run with when they chose none: the project's own, where it
    /// narrows the harness's or, trusted, gives full access; none to leave the harness's.
    pub permissions: Option<Permissions>,
    /// Whether its threads' sandboxed commands go without the network.
    pub offline: bool,
}

impl Protections {
    /// The mode a thread of the project runs with: its own choice (`chosen`), else the
    /// project's, else the harness's (`own`).
    pub fn permissions(self, chosen: Option<Permissions>, own: Permissions) -> Permissions {
        chosen.or(self.permissions).unwrap_or(own)
    }

    /// The sandbox a thread's commands run in with `permissions`, where the most a sandbox
    /// here allows is `here` (none where none can run): read-only for Plan and Ask first, the
    /// project for Auto where that can run, and none for Full access. Without the network where
    /// the project takes it.
    pub fn sandbox(self, permissions: Permissions, here: Option<&Sandbox>) -> Option<Sandbox> {
        let here = here?;
        let level = match permissions {
            Permissions::Plan | Permissions::Ask => SandboxLevel::ReadOnly,
            Permissions::Auto if here.level == SandboxLevel::Project => SandboxLevel::Project,
            Permissions::Auto | Permissions::FullAccess => return None,
        };
        Some(Sandbox {
            level,
            network: here.network && !self.offline,
            ..here.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Permissions::{Ask, Auto, FullAccess, Plan};

    #[test]
    fn a_thread_s_own_choice_is_nearest_then_its_project_s_then_the_harness_s() {
        let none = Protections::default();
        assert_eq!(none.permissions(None, Auto), Auto);
        assert_eq!(none.permissions(Some(Ask), Auto), Ask);
        let project = Protections {
            permissions: Some(Ask),
            ..Protections::default()
        };
        assert_eq!(project.permissions(None, FullAccess), Ask);
        assert_eq!(project.permissions(Some(FullAccess), Auto), FullAccess);
    }

    #[test]
    fn each_mode_comes_to_its_own_sandbox_and_never_one_that_cannot_run() {
        let here = Sandbox {
            level: SandboxLevel::Project,
            network: true,
            home: None,
            glave_home: None,
        };
        let level = |protections: Protections, mode, here: Option<&Sandbox>| {
            protections
                .sandbox(mode, here)
                .map(|sandbox| (sandbox.level, sandbox.network))
        };
        let none = Protections::default();
        for cautious in [Plan, Ask] {
            assert_eq!(
                level(none, cautious, Some(&here)),
                Some((SandboxLevel::ReadOnly, true))
            );
        }
        assert_eq!(
            level(none, Auto, Some(&here)),
            Some((SandboxLevel::Project, true))
        );
        assert_eq!(level(none, FullAccess, Some(&here)), None);
        assert_eq!(level(none, Ask, None), None, "none runs here");
        let offline = Protections {
            offline: true,
            ..Protections::default()
        };
        assert_eq!(
            level(offline, Auto, Some(&here)),
            Some((SandboxLevel::Project, false))
        );
        let read_only_here = Sandbox {
            level: SandboxLevel::ReadOnly,
            ..here
        };
        assert_eq!(
            level(none, Ask, Some(&read_only_here)),
            Some((SandboxLevel::ReadOnly, true))
        );
        assert_eq!(
            level(none, Auto, Some(&read_only_here)),
            None,
            "Auto never runs where its commands could not write the project"
        );
    }
}
