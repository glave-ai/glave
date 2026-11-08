//! What `command/list` answers beside the commands: the previews of what they serve.

use glave_ports::computer::CommandId;
use serde::{Deserialize, Serialize};

/// A page a command in the background serves, at its own address on this computer, never
/// glave's (`specs/session.md`, "Previews"): opened in a browser here, or in a frame of the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Preview {
    /// The command that serves it, as `command/list` lists it.
    pub command: CommandId,
    /// The port it listens on.
    pub port: u16,
    /// Where a browser on this computer opens it: `http://127.0.0.1:<port>/`, or
    /// `http://[::1]:<port>/` for a server that listens on IPv6 alone.
    pub url: String,
}
