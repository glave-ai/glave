//! The isolation suite's table: every method of the session API with the params it is called
//! with, each id of someone's written as a placeholder (`@thread`) for the sweep to fill in
//! with another person's id, or with one nobody has. A method missing here, or a param naming
//! something of someone's that no template fills, fails the suite, so a method is covered the
//! moment it is added to the protocol's table.

use serde_json::{Value, json};

/// What an id names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Kind {
    Thread,
    Project,
    Approval,
    Question,
    /// What a thread's project asks to be trusted.
    Ask,
    /// A command a thread keeps running in the background.
    Command,
    Schedule,
    Memory,
    /// A model account.
    Provider,
    /// A git host's account.
    Credential,
    /// A sign-in to GitHub in the browser, under way.
    Flow,
    /// A sign-in link's code. Whoever holds it signs in as the person on that computer, so
    /// within one organization it proves nothing: it is checked across organizations only.
    Code,
    /// A browser's session, which a caller presents rather than names (`@caller` in a
    /// template).
    Session,
    /// A running computer to start a thread on.
    Computer,
    /// Files kept for a message, as `turn/start` names them: a list of one, by its content's
    /// hash.
    Files,
}

impl Kind {
    pub(super) const ALL: [Self; 15] = [
        Self::Thread,
        Self::Project,
        Self::Approval,
        Self::Question,
        Self::Ask,
        Self::Command,
        Self::Schedule,
        Self::Memory,
        Self::Provider,
        Self::Credential,
        Self::Flow,
        Self::Code,
        Self::Session,
        Self::Computer,
        Self::Files,
    ];

    /// How a template writes it.
    pub(super) fn placeholder(self) -> &'static str {
        match self {
            Self::Thread => "@thread",
            Self::Project => "@project",
            Self::Approval => "@approval",
            Self::Question => "@question",
            Self::Ask => "@ask",
            Self::Command => "@command",
            Self::Schedule => "@schedule",
            Self::Memory => "@memory",
            Self::Provider => "@provider",
            Self::Credential => "@credential",
            Self::Flow => "@flow",
            Self::Code => "@code",
            Self::Session => "@session",
            Self::Computer => "@computer",
            Self::Files => "@files",
        }
    }

    /// The kind a template's string stands for, if it is a placeholder.
    pub(super) fn of(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.placeholder() == text)
    }

    /// The protocol type of a param that names one: each such param must be a placeholder.
    pub(super) fn schema_type(self) -> Option<&'static str> {
        match self {
            Self::Thread => Some("ThreadId"),
            Self::Project => Some("ProjectId"),
            Self::Approval | Self::Question => Some("CallId"),
            Self::Command => Some("CommandId"),
            Self::Schedule => Some("ScheduleId"),
            Self::Memory => Some("MemoryId"),
            Self::Provider => Some("ProviderId"),
            Self::Credential => Some("CredentialId"),
            Self::Computer => Some("ComputerId"),
            Self::Ask | Self::Flow | Self::Code | Self::Session | Self::Files => None,
        }
    }

    /// One nobody has: what an id of someone's must be answered like.
    pub(super) fn nobodys(self) -> Value {
        // UUID v7s no clock or counter in a test reaches.
        let uuid = |last: u64| format!("0192f5c0-0000-7000-8000-{last:012x}");
        if self == Self::Files {
            return files(&"e".repeat(64));
        }
        Value::String(match self {
            Self::Thread => uuid(0xdead_0001),
            Self::Project => uuid(0xdead_0002),
            Self::Approval | Self::Question => "call_nobody".to_owned(),
            Self::Ask => "nobody's ask".to_owned(),
            Self::Command => uuid(0xdead_0003),
            Self::Schedule => uuid(0xdead_0004),
            Self::Memory => uuid(0xdead_0005),
            Self::Provider => uuid(0xdead_0006),
            Self::Credential => uuid(0xdead_0007),
            Self::Flow => "flow-nobody".to_owned(),
            Self::Code => "0".repeat(64),
            Self::Session => uuid(0xdead_0008),
            Self::Computer => uuid(0xdead_0009),
            Self::Files => String::new(),
        })
    }
}

/// `turn/start`'s files: one, its content hashing to `sha256`, named as every person's is.
pub(super) fn files(sha256: &str) -> Value {
    json!([{"sha256": sha256, "name": "notes.txt"}])
}

/// Kinds no one in the suite's world holds, and why: their templates are still called with
/// ids nobody has, and checked to name no one's data, but never with another's.
pub(super) const NOT_HELD: &[(Kind, &str)] = &[
    (
        Kind::Ask,
        "a project asks only from a folder of its own with something to trust",
    ),
    (
        Kind::Command,
        "no command is left running in the background",
    ),
    (
        Kind::Computer,
        "threads on a computer of one's own are not available yet",
    ),
];

/// The params every method is called with. A template with no placeholder acts only on the
/// caller's own, and is called once by each person before the sweep.
#[expect(
    clippy::too_many_lines,
    reason = "one entry per method: the suite's one table"
)]
pub(super) fn table() -> Vec<(&'static str, Vec<Value>)> {
    let on_thread = || vec![json!({"thread": "@thread"})];
    let cron = json!({"cron": "0 9 * * *"});
    vec![
        ("server/discover", vec![json!({})]),
        // Whether the server's own computer stays awake: no one's id to name.
        ("awake/get", vec![json!({})]),
        ("awake/set", vec![json!({"on": false})]),
        // The caller's own language: no one else's to name.
        ("language/get", vec![json!({})]),
        (
            "language/set",
            vec![json!({"language": "en-XA"}), json!({})],
        ),
        // How the caller's own threads are grouped: no one else's to name.
        ("grouping/get", vec![json!({})]),
        ("grouping/set", vec![json!({"grouping": "status"})]),
        ("signin/link", vec![json!({})]),
        ("signin/redeem", vec![json!({"code": "@code"})]),
        ("signin/end", vec![json!({"@caller": "@session"})]),
        // Ends every session but the caller's own: there is no one else's to name.
        ("signin/end_others", vec![json!({})]),
        ("signin/setup", vec![json!({})]),
        // Passkeys are the person's own, on their own server: a request or a passkey is named
        // by an id only its server's browsers ledger knows.
        ("passkey/create_options", vec![json!({"code": "@code"})]),
        (
            "passkey/create",
            vec![json!({"ceremony": "x", "credential": {}})],
        ),
        ("passkey/add_options", vec![json!({})]),
        (
            "passkey/add",
            vec![json!({"ceremony": "x", "credential": {}})],
        ),
        ("passkey/get_options", vec![json!({})]),
        (
            "passkey/get",
            vec![json!({"ceremony": "x", "credential": {}})],
        ),
        ("passkey/list", vec![json!({})]),
        ("passkey/remove", vec![json!({"passkey": "x"})]),
        ("model/list", vec![json!({})]),
        (
            "thread/start",
            vec![
                json!({}),
                json!({"project": "@project"}),
                json!({"project": "@project", "background": true}),
                json!({"computer": "@computer"}),
            ],
        ),
        (
            "thread/list",
            vec![
                json!({}),
                json!({"show": "archived"}),
                json!({"show": "deleted"}),
                json!({"project": "@project"}),
                json!({"project": "@project", "show": "archived"}),
                json!({"project": "@project", "show": "deleted"}),
            ],
        ),
        (
            "thread/rename",
            vec![json!({"thread": "@thread", "title": "mine now"})],
        ),
        ("thread/archive", on_thread()),
        ("thread/unarchive", on_thread()),
        ("thread/delete", on_thread()),
        ("thread/get", on_thread()),
        (
            "thread/events",
            vec![
                json!({"thread": "@thread"}),
                json!({"thread": "@thread", "after": 1, "limit": 1}),
            ],
        ),
        (
            "thread/watch",
            vec![
                json!({"thread": "@thread"}),
                json!({"thread": "@thread", "after": 1}),
            ],
        ),
        ("thread/unwatch", on_thread()),
        // A watch of every project the caller owns names no one's id and never ends: who
        // hears of what there is `list_watching.rs`'s to show.
        (
            "thread/list/watch",
            vec![
                json!({"project": "@project"}),
                json!({"project": "@project", "after": "2026-10-01T00:00:00.000Z"}),
            ],
        ),
        ("thread/list/unwatch", vec![json!({"project": "@project"})]),
        (
            "turn/start",
            vec![
                json!({"thread": "@thread", "prompt": "hello"}),
                json!({"thread": "@thread", "prompt": "hello", "files": "@files"}),
                json!({"thread": "@thread", "prompt": "hello", "background": true, "budget": 1000}),
            ],
        ),
        ("turn/cancel", on_thread()),
        (
            "file/upload",
            vec![
                json!({"name": "notes.txt", "media_type": "text/plain", "data": "aGk="}),
                json!({"project": "@project", "name": "notes.txt", "media_type": "text/plain", "data": "aGk="}),
            ],
        ),
        ("thread/compact", on_thread()),
        (
            "approval/decide",
            vec![json!({"thread": "@thread", "approval": "@approval", "decision": "allow_once"})],
        ),
        (
            "question/answer",
            vec![json!({"thread": "@thread", "question": "@question", "answers": ["yes"]})],
        ),
        (
            "trust/answer",
            vec![json!({"thread": "@thread", "ask": "@ask", "answer": "trust"})],
        ),
        (
            "goal/start",
            vec![
                json!({"thread": "@thread"}),
                json!({"thread": "@thread", "objective": "win"}),
                json!({"thread": "@thread", "objective": "win", "background": true}),
            ],
        ),
        ("goal/stop", on_thread()),
        ("goal/get", on_thread()),
        ("tasks/list", vec![json!({})]),
        ("tasks/stop", on_thread()),
        ("command/list", on_thread()),
        (
            "command/stop",
            vec![json!({"thread": "@thread", "command": "@command"})],
        ),
        (
            "schedule/create",
            vec![
                json!({"when": cron, "prompt": "triage", "budget": 1000}),
                json!({"project": "@project", "when": cron, "prompt": "triage", "budget": 1000}),
            ],
        ),
        ("schedule/list", vec![json!({})]),
        ("schedule/delete", vec![json!({"schedule": "@schedule"})]),
        ("project/list", vec![json!({})]),
        ("project/get", vec![json!({"project": "@project"})]),
        ("project/find", vec![json!({"path": "/nowhere/at/all"})]),
        ("project/create", vec![json!({"name": "probe"})]),
        (
            "project/rename",
            vec![json!({"project": "@project", "name": "mine now"})],
        ),
        (
            "project/folder/set",
            vec![json!({"project": "@project", "path": "/nowhere/at/all"})],
        ),
        ("project/remove", vec![json!({"project": "@project"})]),
        ("folder/list", vec![json!({})]),
        (
            "memory/list",
            vec![
                json!({}),
                json!({"scope": "personal"}),
                json!({"project": "@project"}),
                json!({"project": "@project", "scope": "personal"}),
            ],
        ),
        (
            "memory/save",
            vec![
                json!({"text": "probe", "scope": "project", "project": "@project"}),
                json!({"text": "probe", "scope": "personal", "project": "@project"}),
            ],
        ),
        (
            "memory/edit",
            vec![
                json!({"memory": "@memory", "text": "changed"}),
                json!({"memory": "@memory", "text": "changed", "project": "@project"}),
            ],
        ),
        (
            "memory/move",
            vec![
                json!({"memory": "@memory", "scope": "personal"}),
                json!({"memory": "@memory", "scope": "project"}),
                json!({"memory": "@memory", "scope": "personal", "project": "@project"}),
                json!({"memory": "@memory", "scope": "project", "project": "@project"}),
            ],
        ),
        (
            "memory/forget",
            vec![
                json!({"memory": "@memory"}),
                json!({"memory": "@memory", "project": "@project"}),
                json!({"all": true, "scope": "project", "project": "@project"}),
                json!({"all": true, "scope": "personal", "project": "@project"}),
            ],
        ),
        (
            "memory/restore",
            vec![
                json!({"memory": "@memory"}),
                json!({"memory": "@memory", "project": "@project"}),
            ],
        ),
        // A key the fake service refuses: whether the caller may connect is what is checked.
        (
            "provider/connect",
            vec![json!({"kind": "openai", "key": "a-key-it-refuses"})],
        ),
        ("provider/list", vec![json!({})]),
        ("provider/refresh", vec![json!({})]),
        ("provider/remove", vec![json!({"provider": "@provider"})]),
        (
            "credential/connect",
            vec![json!({"url": "https://github.com", "token": "a-token-it-refuses"})],
        ),
        ("credential/list", vec![json!({})]),
        (
            "credential/remove",
            vec![json!({"credential": "@credential"})],
        ),
        ("credential/device/start", vec![json!({})]),
        ("credential/device/poll", vec![json!({"flow": "@flow"})]),
        (
            "connection/start",
            vec![json!({"server": "https://mcp.example.com/mcp"})],
        ),
        ("connection/poll", vec![json!({"flow": "@flow"})]),
        ("connection/list", vec![json!({})]),
        (
            "connection/remove",
            vec![json!({"server": "https://mcp.example.com/mcp"})],
        ),
        ("search/get", vec![json!({}), json!({"usage": true})]),
        ("search/set", vec![json!({"on": true})]),
    ]
}

/// The placeholders in `template`, by param.
pub(super) fn placeholders(template: &Value) -> Vec<(String, Kind)> {
    template
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(param, value)| Some((param.clone(), Kind::of(value.as_str()?)?)))
        .collect()
}
