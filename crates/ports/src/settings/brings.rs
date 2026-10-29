//! What a project brings that waits for the person's trust, in plain words, one table: glave's
//! config reads a project and says each thing with it, and the session writes it again in the
//! caller's language (`specs/session.md`, "Languages").

use glave_event::worded;

worded! {
    /// One thing a project brings, as the person deciding reads it.
    Brings {
        // Tool servers.
        RunsServer = "brings-runs-server" "Runs a tool server",
        ConnectsServer = "brings-connects-server" "Connects to a tool server",

        // Hooks.
        HookBeforeEachCommand = "brings-hook-before-each-command" "Runs a hook before each command",
        HookBeforeEachAction = "brings-hook-before-each-action" "Runs a hook before each action",
        HookBeforeSome = "brings-hook-before-some" "Runs a hook before some actions",
        HookAfterEachCommand = "brings-hook-after-each-command" "Runs a hook after each command",
        HookAfterEachAction = "brings-hook-after-each-action" "Runs a hook after each action",
        HookAfterSome = "brings-hook-after-some" "Runs a hook after some actions",
        HookOnMessage = "brings-hook-on-message" "Runs a hook when you send a message",
        HookOnReply = "brings-hook-on-reply" "Runs a hook when glave finishes a reply",

        // Settings that loosen how commands run.
        FullAccess = "brings-full-access" "Lets glave work outside the sandbox without asking",
        Judges = "brings-judges" "Chooses which model judges risky actions",
        PrivateNetworks = "brings-private-networks" "Lets glave fetch pages on private networks",
        Sites = "brings-sites" "Lets glave fetch these sites, even on a private network",
    }
}
