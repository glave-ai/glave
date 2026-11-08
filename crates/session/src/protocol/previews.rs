//! What `command/list` answers beside the commands: the previews of what they serve.

use glave_ports::computer::CommandId;
use serde::{Deserialize, Serialize};

/// A page a command in the background serves, shown at an address of its own on this
/// computer, never glave's own (`specs/session.md`, "Previews"): opened in a browser here, or
/// in a frame of the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Preview {
    /// The command that serves it, as `command/list` lists it.
    pub command: CommandId,
    /// The port the command listens on, on its computer.
    pub port: u16,
    /// Where a browser on this computer opens it: `http://127.0.0.1:<its own port>/`.
    pub url: String,
}
