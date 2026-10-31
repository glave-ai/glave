//! The computer protocol on the wire: JSON-RPC 2.0, one message per line, as the session API
//! frames it on stdio. Requests run concurrently; a command's output arrives as
//! `command/output` notifications before its response, and `command/cancel` stops it. While a
//! command offered credentials runs, the computer asks the caller for one with a request of
//! its own, `credentials/request`, which the caller answers on the same connection.

use glave_ports::computer::{
    Command, CommandId, ComputerError, ComputerId, CredentialAsk, CredentialHost, Environment,
    FileWrite, GitCredential, Glob, Listing, Save, Search, Sources, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The longest message accepted, in bytes: a whole file read or written, as base64, fits.
pub(crate) const MAX_LINE: u64 = 32 * 1024 * 1024;

/// The protocol version spoken; it changes whenever a message changes shape. `2026-10-10`
/// added `files/stat`, `files/glob`, `files/search`, ignore-aware listings and the sandbox on
/// `command/run`: a server that predates it would run a sandboxed command unconfined.
/// `2026-10-10.1` added the credential on `workspace/open`. `2026-10-11` added `protect` on
/// each file of `files/write`: the computer refuses a protected path on the path it opens. A
/// server that predates it would ignore the field and write, so the client refuses such a
/// server before sending it anything. The protected list is the one in `glave-ports`, the same
/// on both sides; changing it changes the protocol. `2026-10-15` added `credentials` on
/// `command/run` and the computer's own `credentials/request`. `2026-10-16` added the
/// `project` sources of `workspace/open`, a new project's folder. `2026-10-17` added commands
/// in the background: `command/start`, `command/output`, `command/stop` and `command/list`,
/// `on_timeout` on a command, and the `backgrounded` and `idle` endings. `2026-10-28` added
/// `command/ended`, how a command in the background ended, waited for without reading it.
pub(crate) const PROTOCOL: &str = "2026-10-28";

/// The notification that carries a command's output.
pub(crate) const OUTPUT: &str = "command/output";

/// The request a computer sends its caller for a credential, while a command runs.
pub(crate) const ASK: &str = "credentials/request";

/// Every request, by method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub(crate) enum Request {
    /// Over a network transport, the first message: the token the harness side minted.
    #[serde(rename = "connection/authenticate")]
    Authenticate { token: String },
    /// What the server is, and the protocol versions it speaks.
    #[serde(rename = "server/discover")]
    Discover {},
    #[serde(rename = "computer/create")]
    Create { environment: Environment },
    #[serde(rename = "computer/status")]
    Status { computer: ComputerId },
    #[serde(rename = "computer/destroy")]
    Destroy { computer: ComputerId },
    #[serde(rename = "computer/worktrees")]
    Worktrees { computer: ComputerId },
    #[serde(rename = "computer/free_worktree")]
    FreeWorktree { computer: ComputerId, path: String },
    /// `credential`, when given, is kept in memory by the computer for fetching and saving.
    #[serde(rename = "workspace/open")]
    Open {
        computer: ComputerId,
        sources: Sources,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        credential: Option<GitCredential>,
    },
    #[serde(rename = "workspace/close")]
    Close { workspace: WorkspaceId },
    /// `run` names this run in its output notifications, in `command/cancel` and in the
    /// computer's `credentials/request`, which it sends only for the hosts `credentials` names.
    #[serde(rename = "command/run")]
    Run {
        workspace: WorkspaceId,
        command: Command,
        run: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        credentials: Vec<CredentialHost>,
    },
    #[serde(rename = "command/cancel")]
    Cancel { run: String },
    /// Answered with the `command` id it runs as in the background.
    #[serde(rename = "command/start")]
    Start {
        workspace: WorkspaceId,
        command: Command,
    },
    /// Waits up to `wait_ms` for the command to end before answering what it printed.
    #[serde(rename = "command/output")]
    Output {
        workspace: WorkspaceId,
        command: CommandId,
        #[serde(default)]
        wait_ms: u64,
    },
    /// Waits up to `wait_ms` for the command to end, answering how it `ended` (null while it
    /// runs), without reading it.
    #[serde(rename = "command/ended")]
    Ended {
        workspace: WorkspaceId,
        command: CommandId,
        #[serde(default)]
        wait_ms: u64,
    },
    #[serde(rename = "command/stop")]
    Stop {
        workspace: WorkspaceId,
        command: CommandId,
    },
    /// Answered with the workspace's `commands` in the background.
    #[serde(rename = "command/list")]
    Commands { workspace: WorkspaceId },
    #[serde(rename = "files/read")]
    Read {
        workspace: WorkspaceId,
        path: String,
    },
    #[serde(rename = "files/write")]
    Write {
        workspace: WorkspaceId,
        files: Vec<FileWrite>,
    },
    #[serde(rename = "files/list")]
    List {
        workspace: WorkspaceId,
        listing: Listing,
    },
    #[serde(rename = "files/stat")]
    Stat {
        workspace: WorkspaceId,
        path: String,
    },
    #[serde(rename = "files/glob")]
    Glob { workspace: WorkspaceId, glob: Glob },
    #[serde(rename = "files/search")]
    Search {
        workspace: WorkspaceId,
        search: Search,
    },
    #[serde(rename = "workspace/save")]
    Save { workspace: WorkspaceId, save: Save },
}

impl Request {
    /// The method this request names, for errors that say which call failed.
    pub(crate) fn method(&self) -> &'static str {
        match self {
            Self::Authenticate { .. } => "connection/authenticate",
            Self::Discover {} => "server/discover",
            Self::Create { .. } => "computer/create",
            Self::Status { .. } => "computer/status",
            Self::Destroy { .. } => "computer/destroy",
            Self::Worktrees { .. } => "computer/worktrees",
            Self::FreeWorktree { .. } => "computer/free_worktree",
            Self::Open { .. } => "workspace/open",
            Self::Close { .. } => "workspace/close",
            Self::Run { .. } => "command/run",
            Self::Cancel { .. } => "command/cancel",
            Self::Start { .. } => "command/start",
            Self::Output { .. } => "command/output",
            Self::Ended { .. } => "command/ended",
            Self::Stop { .. } => "command/stop",
            Self::Commands { .. } => "command/list",
            Self::Read { .. } => "files/read",
            Self::Write { .. } => "files/write",
            Self::List { .. } => "files/list",
            Self::Stat { .. } => "files/stat",
            Self::Glob { .. } => "files/glob",
            Self::Search { .. } => "files/search",
            Self::Save { .. } => "workspace/save",
        }
    }
}

/// Every method a request may name.
const METHODS: &[&str] = &[
    "connection/authenticate",
    "server/discover",
    "computer/create",
    "computer/status",
    "computer/destroy",
    "computer/worktrees",
    "computer/free_worktree",
    "workspace/open",
    "workspace/close",
    "command/run",
    "command/cancel",
    "command/start",
    "command/output",
    "command/ended",
    "command/stop",
    "command/list",
    "files/read",
    "files/write",
    "files/list",
    "files/stat",
    "files/glob",
    "files/search",
    "workspace/save",
];

/// A command's output, as it arrives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Chunk {
    pub(crate) run: String,
    pub(crate) text: String,
}

/// What `credentials/request` asks: a credential for the run's command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Asked {
    pub(crate) run: String,
    pub(crate) ask: CredentialAsk,
}

/// What the caller answers it: a credential, or none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Given {
    #[serde(default)]
    pub(crate) credential: Option<GitCredential>,
}

/// JSON-RPC's error codes, and one for a computer's own refusals and failures, whose
/// [`ComputerError`] travels in `data`.
pub(crate) mod code {
    pub(crate) const PARSE: i64 = -32700;
    pub(crate) const INVALID_REQUEST: i64 = -32600;
    pub(crate) const NO_METHOD: i64 = -32601;
    pub(crate) const INVALID_PARAMS: i64 = -32602;
    pub(crate) const COMPUTER: i64 = -32000;
    pub(crate) const UNAUTHENTICATED: i64 = -32001;
}

/// A request line: its id and request, or the id (if known) and why it is not one.
pub(crate) fn parse(line: &str) -> Result<(Option<Value>, Request), (Option<Value>, Value)> {
    let message: Value = serde_json::from_str(line).map_err(|error| {
        (
            None,
            failure(code::PARSE, &format!("not JSON-RPC: {error}")),
        )
    })?;
    let id = message.get("id").cloned();
    if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err((
            id,
            failure(code::INVALID_REQUEST, "expected \"jsonrpc\": \"2.0\""),
        ));
    }
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return Err((id, failure(code::INVALID_REQUEST, "no method")));
    };
    if !METHODS.contains(&method) {
        let message = format!("no method {method:?}");
        return Err((id, failure(code::NO_METHOD, &message)));
    }
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    serde_json::from_value(json!({"method": method, "params": params}))
        .map(|request| (id.clone(), request))
        .map_err(|error| {
            (
                id,
                failure(code::INVALID_PARAMS, &format!("{method}: {error}")),
            )
        })
}

/// A JSON-RPC error object.
pub(crate) fn failure(code: i64, message: &str) -> Value {
    json!({"code": code, "message": message})
}

/// A computer's error as a JSON-RPC error object, the error itself in `data`.
pub(crate) fn computer_failure(error: &ComputerError) -> Value {
    json!({"code": code::COMPUTER, "message": error.to_string(), "data": error})
}

/// The response to request `id`.
pub(crate) fn respond(id: Option<&Value>, answered: Result<Value, Value>) -> String {
    match answered {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
    }
    .to_string()
}

/// A notification: a message with a method and no id.
pub(crate) fn notify(method: &str, params: &impl Serialize) -> String {
    json!({"jsonrpc": "2.0", "method": method, "params": params}).to_string()
}

/// A response the other side sent to a request of this side's: its id and its result or
/// error. `None` for any other message.
pub(crate) fn response(line: &str) -> Option<(u64, Result<Value, Value>)> {
    let message: Value = serde_json::from_str(line).ok()?;
    if message.get("method").is_some() || message.get("jsonrpc")? != "2.0" {
        return None;
    }
    let id = message.get("id")?.as_u64()?;
    match (message.get("result"), message.get("error")) {
        (Some(result), None) => Some((id, Ok(result.clone()))),
        (None, Some(error)) => Some((id, Err(error.clone()))),
        _ => None,
    }
}

/// The computer's own `credentials/request`, with id `id`.
pub(crate) fn ask(id: u64, asked: &Asked) -> String {
    json!({"jsonrpc": "2.0", "id": id, "method": ASK, "params": asked}).to_string()
}

/// A request message.
pub(crate) fn request(id: u64, request: &Request) -> String {
    let mut message = serde_json::to_value(request).unwrap_or(Value::Null);
    if let Value::Object(fields) = &mut message {
        fields.insert("jsonrpc".into(), "2.0".into());
        fields.insert("id".into(), id.into());
    }
    message.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip_and_mistakes_get_standard_codes() {
        let run = Request::Run {
            workspace: WorkspaceId::from_uuid(uuid::Uuid::nil()),
            command: Command::new("ls"),
            run: "r1".into(),
            credentials: Vec::new(),
        };
        let line = request(7, &run);
        assert!(line.contains(r#""method":"command/run""#), "{line}");
        assert_eq!(run.method(), "command/run");
        assert!(METHODS.contains(&Request::Discover {}.method()));
        assert_eq!(parse(&line).unwrap(), (Some(json!(7)), run));
        let code = |line: &str| parse(line).err().map(|(_, error)| error["code"].clone());
        assert_eq!(code("nope"), Some(json!(code::PARSE)));
        assert_eq!(
            code(r#"{"id":1,"method":"computer/status"}"#),
            Some(json!(code::INVALID_REQUEST))
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":1,"method":"computer/explode"}"#),
            Some(json!(code::NO_METHOD))
        );
        assert_eq!(
            code(r#"{"jsonrpc":"2.0","id":1,"method":"computer/status","params":{}}"#),
            Some(json!(code::INVALID_PARAMS))
        );
        let error = ComputerError::NotFound("no workspace".into());
        assert_eq!(
            respond(Some(&json!(1)), Err(computer_failure(&error))),
            r#"{"error":{"code":-32000,"data":{"kind":"not_found","message":"no workspace"},"message":"not found: no workspace"},"id":1,"jsonrpc":"2.0"}"#
        );
    }
}
