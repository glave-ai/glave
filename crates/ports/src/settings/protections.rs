//! What the agent may do in a thread: the preset it runs with, as its project changes it, and
//! the sandbox that preset comes to on this computer (`specs/approvals.md`, "Permissions"). The
//! harness applies them; project trust decides what a project changes.

use glave_event::Permissions;

use crate::computer::{Sandbox, SandboxLevel};

/// How a project's settings change what its threads may do.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Protections {
    /// The preset its threads run with when they chose none: the project's own, where it
    /// narrows the harness's or, trusted, gives full access; none to leave the harness's.
    pub permissions: Option<Permissions>,
    /// Whether its threads' sandboxed commands go without the network.
    pub offline: bool,
}

impl Protections {
    /// The preset a thread of the project runs with: its own choice (`chosen`), else the
    /// project's, else the harness's (`own`).
    pub fn permissions(self, chosen: Option<Permissions>, own: Permissions) -> Permissions {
        chosen.or(self.permissions).unwrap_or(own)
    }

    /// The sandbox a thread's commands run in with `permissions`, where the most a sandbox
    /// here allows is `here` (none where none can run): read-only for Read-only, the project
    /// for Workspace where that can run, and none for Full access. Without the network where
    /// the project takes it.
    pub fn sandbox(self, permissions: Permissions, here: Option<&Sandbox>) -> Option<Sandbox> {
        let here = here?;
        let level = match permissions {
            Permissions::ReadOnly => SandboxLevel::ReadOnly,
            Permissions::Workspace if here.level == SandboxLevel::Project => SandboxLevel::Project,
            Permissions::Workspace | Permissions::FullAccess => return None,
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
    use Permissions::{FullAccess, ReadOnly, Workspace};

    #[test]
    fn a_thread_s_own_choice_is_nearest_then_its_project_s_then_the_harness_s() {
        let none = Protections::default();
        assert_eq!(none.permissions(None, Workspace), Workspace);
        assert_eq!(none.permissions(Some(ReadOnly), Workspace), ReadOnly);
        let project = Protections {
            permissions: Some(ReadOnly),
            ..Protections::default()
        };
        assert_eq!(project.permissions(None, FullAccess), ReadOnly);
        assert_eq!(project.permissions(Some(FullAccess), Workspace), FullAccess);
    }

    #[test]
    fn each_preset_comes_to_its_own_sandbox_and_never_one_that_cannot_run() {
        let here = Sandbox {
            level: SandboxLevel::Project,
            network: true,
            home: None,
            glave_home: None,
        };
        let level = |protections: Protections, preset, here: Option<&Sandbox>| {
            protections
                .sandbox(preset, here)
                .map(|sandbox| (sandbox.level, sandbox.network))
        };
        let none = Protections::default();
        assert_eq!(
            level(none, ReadOnly, Some(&here)),
            Some((SandboxLevel::ReadOnly, true))
        );
        assert_eq!(
            level(none, Workspace, Some(&here)),
            Some((SandboxLevel::Project, true))
        );
        assert_eq!(level(none, FullAccess, Some(&here)), None);
        assert_eq!(level(none, ReadOnly, None), None, "none runs here");
        let offline = Protections {
            offline: true,
            ..Protections::default()
        };
        assert_eq!(
            level(offline, Workspace, Some(&here)),
            Some((SandboxLevel::Project, false))
        );
        let read_only_here = Sandbox {
            level: SandboxLevel::ReadOnly,
            ..here
        };
        assert_eq!(
            level(none, ReadOnly, Some(&read_only_here)),
            Some((SandboxLevel::ReadOnly, true))
        );
        assert_eq!(
            level(none, Workspace, Some(&read_only_here)),
            None,
            "Workspace never runs where its commands could not write the project"
        );
    }
}
