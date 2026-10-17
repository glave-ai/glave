//! What a repository brings that needs trust: its `.mcp.json` servers, the hooks in its
//! `.glave/config.toml`, and the parts of that file that loosen how commands run. Read, never
//! run: reading a file in a repository you have not trusted is safe, starting what it names
//! is not.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use glave_harness::{ApprovalMode, Hook, HookPoint, point_name};
use serde_json::Value;

use crate::policy::{self, ConfigFile, HooksFile, SandboxChoice, WebFile};

/// A project's MCP servers, in Claude Code's format, at the repository's root.
pub const MCP_FILE: &str = ".mcp.json";

/// The id of the settings item, beside `mcp:<server>` ids.
pub const SETTINGS: &str = "settings";

/// The start of a hook's id: `hook:<point>:<n>`, the nth hook at that point.
const HOOK: &str = "hook:";

/// One thing the person is asked to trust.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// `mcp:<server>`, `hook:<point>:<n>` or `settings`.
    pub id: String,
    /// Exactly what is trusted, so a changed definition is a new question.
    pub definition: String,
    /// What it does, in a few words, for the question and for `glave policy explain`.
    pub shows: String,
    /// What it does in plain words, for a person deciding in an app: one line for a server or
    /// a hook, one for each setting.
    pub plain: Vec<Plain>,
}

/// One line of what an item does, in plain words: a sentence, and the thing itself when there
/// is one to show (the command a server or hook runs, an address, a model).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plain {
    pub text: &'static str,
    pub detail: Option<String>,
}

impl Plain {
    fn new(text: &'static str, detail: Option<String>) -> Self {
        Self { text, detail }
    }
}

impl Item {
    /// Whether this is an MCP server, which a policy may forbid.
    pub fn is_server(&self) -> bool {
        self.id.starts_with("mcp:")
    }

    /// Whether this is a hook, which a policy may forbid.
    pub fn is_hook(&self) -> bool {
        self.id.starts_with(HOOK)
    }

    /// The label the person reads: `MCP server docs`, `hook PreToolUse` or `settings`.
    pub fn label(&self) -> String {
        if let Some(name) = self.id.strip_prefix("mcp:") {
            return format!("MCP server {name}");
        }
        match self
            .id
            .strip_prefix(HOOK)
            .and_then(|rest| rest.split_once(':'))
        {
            Some((point, _)) => format!("hook {point}"),
            None => "settings".to_owned(),
        }
    }
}

/// A repository as glave reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// The repository's root with symlinks resolved: the key trust is kept under.
    pub root: PathBuf,
    /// The `origin` remote in `.git/config`, which lets a moved clone be recognised.
    pub remote: Option<String>,
    /// The `.glave/config.toml`, when there is one.
    pub config: Option<ConfigFile>,
    /// The `.mcp.json` servers by name.
    pub servers: BTreeMap<String, Item>,
}

impl Project {
    /// Reads the repository at `root`. A file that does not parse stops glave, naming it, so
    /// a mistake never quietly turns a project's settings off.
    pub async fn read(root: &Path, environment: &BTreeMap<String, String>) -> crate::Result<Self> {
        let root = tokio::fs::canonicalize(root)
            .await
            .map_err(crate::Error::io("resolve", root))?;
        let config_path = root.join(".glave").join(policy::CONFIG);
        let config = match read(&config_path).await? {
            Some(text) => Some(policy::config(&text, &config_path)?),
            None => None,
        };
        let mcp_path = root.join(MCP_FILE);
        let servers = match read(&mcp_path).await? {
            Some(text) => servers(&text, environment, &mcp_path)?,
            None => BTreeMap::new(),
        };
        let remote = remote(&root).await;
        Ok(Self {
            root,
            remote,
            config,
            servers,
        })
    }

    /// Everything here that needs trust: the servers, the hooks, then the settings that
    /// loosen things.
    pub fn items(&self) -> Vec<Item> {
        let settings = self.config.as_ref().and_then(loosening);
        let hooks = self.hooks().into_iter().map(|(item, _)| item);
        self.servers
            .values()
            .cloned()
            .chain(hooks)
            .chain(settings)
            .collect()
    }

    /// The hooks in the project's `config.toml`, each with the item that trusts it. Each is
    /// trusted by its whole definition, so a changed command, matcher or timeout asks again.
    pub fn hooks(&self) -> Vec<(Item, Hook)> {
        let hooks = self
            .config
            .as_ref()
            .and_then(|config| config.hooks.as_ref())
            .map(HooksFile::hooks)
            .unwrap_or_default();
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        hooks
            .into_iter()
            .map(|hook| {
                let point = point_name(hook.point);
                let count = counts.entry(point).or_default();
                *count = count.saturating_add(1);
                let definition = serde_json::json!({
                    "event": point,
                    "command": hook.command,
                    "matcher": hook.matcher,
                    "timeout": hook.timeout_secs,
                });
                let shows = match &hook.matcher {
                    Some(matcher) => format!("{} (for tools matching {matcher})", hook.command),
                    None => hook.command.clone(),
                };
                let plain = Plain::new(hook_text(&hook), Some(hook.command.clone()));
                let item = Item {
                    id: format!("{HOOK}{point}:{count}"),
                    definition: definition.to_string(),
                    shows,
                    plain: vec![plain],
                };
                (item, hook)
            })
            .collect()
    }

    /// The config as it applies without trust: choices that only tighten or pick a model,
    /// and no hooks.
    pub fn config_without_trust(&self) -> Option<ConfigFile> {
        self.config.as_ref().map(|config| ConfigFile {
            approval: config.approval.filter(|a| *a != ApprovalMode::Auto),
            sandbox: config.sandbox.filter(|s| *s != SandboxChoice::Off),
            judge_model: config.judge_model.clone().filter(|j| j.trim() == "off"),
            web: config.web.as_ref().map(WebFile::without_trust),
            hooks: None,
            ..config.clone()
        })
    }
}

/// The settings item for the choices in `config` that loosen how commands run: approval
/// `auto`, the sandbox `off`, `[web]` reaching further than the default, and a judge model (a
/// repo must not pick who judges its own actions; `off` only makes glave ask more). Everything
/// else only tightens, or picks a model.
fn loosening(config: &ConfigFile) -> Option<Item> {
    let mut parts = Vec::new();
    let mut plain = Vec::new();
    if config.approval == Some(ApprovalMode::Auto) {
        parts.push("approval = \"auto\"");
        plain.push(Plain::new(APPROVES, None));
    }
    if config.sandbox == Some(SandboxChoice::Off) {
        parts.push("sandbox = \"off\"");
        plain.push(Plain::new(UNSANDBOXED, None));
    }
    let judge = config
        .judge_model
        .as_deref()
        .filter(|judge| judge.trim() != "off");
    let judge_part = judge.map(|judge| format!("judge_model = {judge:?}"));
    if let Some(judge) = judge {
        plain.push(Plain::new(JUDGES, Some(judge.to_owned())));
    }
    let web_part = config.web.as_ref().and_then(WebFile::loosening);
    if let Some(web) = &config.web {
        plain.extend(web_plain(web));
    }
    (!parts.is_empty() || judge_part.is_some() || web_part.is_some()).then(|| {
        let text = parts
            .iter()
            .map(|part| (*part).to_owned())
            .chain(judge_part)
            .chain(web_part)
            .collect::<Vec<_>>()
            .join(", ");
        Item {
            id: SETTINGS.to_owned(),
            definition: text.clone(),
            shows: text,
            plain,
        }
    })
}

/// What `approval = "auto"` does, in plain words.
const APPROVES: &str = "Lets glave approve actions without asking";
/// What `sandbox = "off"` does.
const UNSANDBOXED: &str = "Turns off the sandbox";
/// What a `judge_model` other than `off` does.
const JUDGES: &str = "Chooses which model judges risky actions";

/// What a `[web]` that reaches further than glave's default does, in plain words.
fn web_plain(web: &WebFile) -> Vec<Plain> {
    let mut plain = Vec::new();
    if web.private_addresses == Some(true) {
        plain.push(Plain::new(
            "Lets glave fetch pages on private networks",
            None,
        ));
    }
    if !web.allow.is_empty() {
        plain.push(Plain::new(
            "Lets glave fetch these sites, even on a private network",
            Some(web.allow.join(", ")),
        ));
    }
    plain
}

/// What a hook does, in plain words: when it runs. A matcher of `bash` alone runs it around
/// each command; any other matcher, around some actions.
fn hook_text(hook: &Hook) -> &'static str {
    let matcher = hook.matcher.as_deref().map(str::trim);
    let every = matcher.is_none_or(|matcher| matcher.is_empty() || matcher == "*");
    let commands = matcher == Some("bash");
    match (hook.point, every, commands) {
        (HookPoint::PreToolUse, _, true) => "Runs a hook before each command",
        (HookPoint::PreToolUse, true, _) => "Runs a hook before each action",
        (HookPoint::PreToolUse, ..) => "Runs a hook before some actions",
        (HookPoint::PostToolUse, _, true) => "Runs a hook after each command",
        (HookPoint::PostToolUse, true, _) => "Runs a hook after each action",
        (HookPoint::PostToolUse, ..) => "Runs a hook after some actions",
        (HookPoint::UserPromptSubmit, ..) => "Runs a hook when you send a message",
        (HookPoint::Stop, ..) => "Runs a hook when glave finishes a reply",
    }
}

/// The servers in `.mcp.json`'s `text`, checked as glave would read them.
fn servers(
    text: &str,
    environment: &BTreeMap<String, String>,
    path: &Path,
) -> crate::Result<BTreeMap<String, Item>> {
    glave_mcp::parse(text, environment)
        .map_err(|error| crate::Error::within(format!("in {}", path.display()), error))?;
    let file: Value = serde_json::from_str(text).map_err(|error| {
        crate::Error::within(format!("{} is not valid JSON", path.display()), error)
    })?;
    let found = file.get("mcpServers").and_then(Value::as_object);
    Ok(found
        .into_iter()
        .flatten()
        .map(|(name, definition)| {
            let remote = definition.get("url").is_some_and(Value::is_string);
            let text = if remote {
                "Connects to a tool server"
            } else {
                "Runs a tool server"
            };
            let item = Item {
                id: format!("mcp:{name}"),
                definition: definition.to_string(),
                shows: shows(definition),
                plain: vec![Plain::new(text, Some(shows(definition)))],
            };
            (name.clone(), item)
        })
        .collect())
}

/// What a server runs or where it connects, as written (variables unfilled).
fn shows(definition: &Value) -> String {
    let text = |key: &str| definition.get(key).and_then(Value::as_str);
    if let Some(url) = text("url") {
        return url.to_owned();
    }
    let args = definition
        .get("args")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str);
    let words: Vec<&str> = text("command").into_iter().chain(args).collect();
    words.join(" ")
}

/// The `origin` remote's url, read from `.git/config` without running git (which a
/// repository's own config can make run programs).
pub async fn remote(root: &Path) -> Option<String> {
    let text = tokio::fs::read_to_string(root.join(".git").join("config"))
        .await
        .ok()?;
    glave_ports::origin_remote(&text)
}

async fn read(path: &Path) -> crate::Result<Option<String>> {
    match tokio::fs::read_to_string(path).await {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(crate::Error::io("read", path)(error)),
    }
}
