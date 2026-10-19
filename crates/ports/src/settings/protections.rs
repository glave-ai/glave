//! How a project's settings change the protections its threads' commands run under: the mode
//! they ask in, and the sandbox they run in. The harness applies them; project trust decides them.

use glave_event::ApprovalMode;

use crate::computer::{Sandbox, SandboxLevel};

/// How a project's settings change the protections its threads' commands run under.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Protections {
    /// The mode its threads run in when they chose none: its own choice, `auto` once trusted;
    /// `ask` when the project turned the sandbox off with auto chosen nowhere; none to leave
    /// the harness's. Always set where its commands run unconfined.
    pub approval: Option<ApprovalMode>,
    /// Whether its threads' commands run without the sandbox, once trusted.
    pub unconfined: bool,
    /// A narrower sandbox than the harness's for its threads' commands.
    pub level: Option<SandboxLevel>,
    /// Whether its threads' sandboxed commands go without the network.
    pub offline: bool,
}

impl Protections {
    /// The mode a thread of the project runs in: its own choice (`chosen`), else the
    /// project's, else the harness's (`own`). Where the project runs commands unconfined
    /// without auto chosen, it asks whatever the thread chose: its auto was chosen behind the
    /// sandbox, and commands run unconfined with nothing asked only when auto is chosen for
    /// that.
    pub fn mode(self, chosen: Option<ApprovalMode>, own: ApprovalMode) -> ApprovalMode {
        if self.unconfined && self.approval != Some(ApprovalMode::Auto) {
            return ApprovalMode::Ask;
        }
        chosen.or(self.approval).unwrap_or(own)
    }

    /// The sandbox its threads' commands run in, where the harness's is `own`: none where it
    /// turned the sandbox off, else `own` narrowed as it asks.
    pub fn sandbox(self, own: Option<&Sandbox>) -> Option<Sandbox> {
        if self.unconfined {
            return None;
        }
        own.map(|own| Sandbox {
            level: self.level.unwrap_or(own.level),
            network: own.network && !self.offline,
            ..own.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thread_never_runs_in_auto_where_its_project_unconfines_it_without_auto() {
        use ApprovalMode::{Ask, Auto};
        let none = Protections::default();
        assert_eq!(none.mode(None, Auto), Auto);
        assert_eq!(none.mode(Some(Ask), Auto), Ask, "its own choice");
        let auto = Protections {
            approval: Some(Auto),
            ..Protections::default()
        };
        assert_eq!(auto.mode(None, Ask), Auto, "the project's");
        assert_eq!(auto.mode(Some(Ask), Ask), Ask, "its own, nearer");
        let off = Protections {
            approval: Some(Ask),
            unconfined: true,
            ..Protections::default()
        };
        assert_eq!(
            off.mode(Some(Auto), Auto),
            Ask,
            "auto chosen behind the sandbox"
        );
        let both = Protections {
            approval: Some(Auto),
            unconfined: true,
            ..Protections::default()
        };
        assert_eq!(both.mode(None, Ask), Auto);
        assert_eq!(both.mode(Some(Ask), Ask), Ask);
    }

    #[test]
    fn a_project_narrows_its_threads_sandbox_or_turns_it_off_and_never_widens_it() {
        let own = Sandbox {
            level: SandboxLevel::Project,
            network: true,
            home: None,
            glave_home: None,
        };
        assert_eq!(
            Protections::default().sandbox(Some(&own)),
            Some(own.clone())
        );
        let narrowed = Protections {
            level: Some(SandboxLevel::ReadOnly),
            offline: true,
            ..Protections::default()
        };
        let sandbox = narrowed.sandbox(Some(&own)).unwrap();
        assert_eq!(
            (sandbox.level, sandbox.network),
            (SandboxLevel::ReadOnly, false)
        );
        assert_eq!(narrowed.sandbox(None), None, "no sandbox here to narrow");
        let off = Protections {
            unconfined: true,
            ..Protections::default()
        };
        assert_eq!(off.sandbox(Some(&own)), None);
    }
}
