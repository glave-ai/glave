//! `install`: adds a skill or a tool server where the person chooses, or takes one out, when
//! they ask the agent to ("make that a skill", "connect to Linear", "disconnect it")
//! (`specs/model.md`, "Configuration is files"). The model's own tools can't: the sandbox keeps
//! them out of glave's folder and off a project's `.mcp.json`. So glave asks the person once,
//! as a service asks (`specs/approvals.md`, "A service asks"), showing what will change, and
//! where it goes: this project's files, which anyone who gets the project gets too, or their
//! own, for all their projects. What changes counts from their next message; a sign-in stays
//! theirs either way. Claude Code asks the same of `claude mcp add` with `--scope project` or
//! `user`, and keeps skills in a project's folder or the person's.

mod places;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glave_event::{Arguments, ServiceAnswer, ServiceField, ServiceFieldKind};
use glave_ports::computer::Sources;
use glave_ports::{
    Assessment, BoxFuture, Called, Cancellation, Effect, ServiceAsks, ServiceGiven, ToolError,
    ToolInfo, ToolOutput, ToolSource, ToolSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::tools::workspace::Workspace;
use places::Place;

/// The tool's name.
pub(crate) const INSTALL: &str = "install";

/// Who asks: glave itself, a name no connected service may ask under.
pub const ASKED_BY_GLAVE: &str = "glave";

/// Where the answer goes, said for glave's own question in place of a service's.
pub(crate) const GLAVE_NOTE: &str =
    "Nothing changes until you choose. The model is told what you chose.";

/// The field the person answers: where it goes.
const WHERE: &str = "place";

/// The longest SKILL.md it adds.
const MOST_SKILL: usize = 256 * 1024;

/// The longest name a server is given.
const MOST_NAME: usize = 64;

/// What the model is told where no one can choose.
const NO_ONE: &str = "Not done: no one is here to choose where it goes. Tell the person what \
     you would add, and they can ask you again.";

/// What the model is told in plan mode.
const PLANNING: &str = "Not done in plan mode: nothing is added while you plan. Put it in the \
     plan instead.";

/// The person's own folder, where `install` puts what is for all their projects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Own {
    /// glave's folder, which holds their `skills` and `mcp.json`.
    pub folder: PathBuf,
    /// Whether they may add tool servers of their own: not where a policy forbids it.
    pub servers: bool,
}

/// The `install` tool for one thread: its project's folder, and the person's own.
pub(crate) struct Install {
    pub(crate) workspace: Arc<Workspace>,
    pub(crate) own: Option<Own>,
}

/// What a call changes, as the person is asked about it and it is kept while they answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub(crate) enum Change {
    AddSkill {
        name: String,
        description: String,
        text: String,
    },
    AddServer {
        name: String,
        server: Value,
    },
    RemoveSkill {
        name: String,
    },
    RemoveServer {
        name: String,
    },
}

impl Change {
    fn name(&self) -> &str {
        match self {
            Self::AddSkill { name, .. }
            | Self::AddServer { name, .. }
            | Self::RemoveSkill { name }
            | Self::RemoveServer { name } => name,
        }
    }

    fn adds(&self) -> bool {
        matches!(self, Self::AddSkill { .. } | Self::AddServer { .. })
    }

    fn is_skill(&self) -> bool {
        matches!(self, Self::AddSkill { .. } | Self::RemoveSkill { .. })
    }
}

/// A call's arguments, as the model gives them.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asked {
    kind: Kind,
    name: String,
    #[serde(default)]
    skill: Option<String>,
    #[serde(default)]
    server: Option<Value>,
    #[serde(default)]
    remove: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Skill,
    Server,
}

pub(crate) fn spec() -> ToolSpec {
    ToolSpec {
        name: INSTALL.to_owned(),
        description: "Add a skill or a tool server (MCP) when the person asks you to (\"make \
                      that a skill\", \"connect to Linear\"), or take one out (\"disconnect \
                      it\"). glave asks them where it goes, this project or all their projects, \
                      and shows them what changes; it counts from their next message. Use this, \
                      never write or bash, for skills and tool servers."
            .to_owned(),
        parameters: json!({"type": "object", "properties": {
            "kind": {"type": "string", "enum": ["skill", "server"]},
            "name": {"type": "string", "description": "The skill's name (lowercase letters, digits and hyphens), or the server's"},
            "skill": {"type": "string", "description": "To add a skill: its whole SKILL.md, frontmatter with name and description first"},
            "server": {"type": "object", "description": "To add a server: its entry as .mcp.json has it, such as {\"type\": \"http\", \"url\": \"https://…\"} or {\"command\": \"npx\", \"args\": […]}"},
            "remove": {"type": "boolean", "description": "Take it out instead"}
        }, "required": ["kind", "name"]}),
    }
}

/// What a call asks to change, or why it can't.
pub(crate) fn change(arguments: &Arguments) -> Result<Change, String> {
    let Arguments::Json(map) = arguments else {
        return Err("The arguments were not valid JSON.".to_owned());
    };
    let asked: Asked = serde_json::from_value(Value::Object(map.clone()))
        .map_err(|err| format!("The arguments were not understood: {err}."))?;
    let name = asked.name.trim().to_owned();
    match (asked.kind, asked.remove) {
        (Kind::Skill, true) => {
            crate::skills::check_name(&name, &name).map_err(|why| format!("The skill's {why}."))?;
            Ok(Change::RemoveSkill { name })
        }
        (Kind::Server, true) => {
            check_server_name(&name)?;
            Ok(Change::RemoveServer { name })
        }
        (Kind::Skill, false) => {
            let text = asked
                .skill
                .ok_or("Give the skill's whole SKILL.md as skill.")?;
            if text.len() > MOST_SKILL {
                return Err(format!("The skill is over {} KiB.", MOST_SKILL / 1024));
            }
            let file = Path::new(&name).join(crate::skills::SKILL_MD);
            let skill = crate::skills::parsed(&text, &file)
                .map_err(|why| format!("The skill can't be added: {why}."))?
                .ok_or("The skill asks not to be offered to the model, so it would do nothing.")?;
            Ok(Change::AddSkill {
                name,
                description: skill.description,
                text,
            })
        }
        (Kind::Server, false) => {
            check_server_name(&name)?;
            let server = asked
                .server
                .ok_or("Give the server's entry, as .mcp.json has it, as server.")?;
            let file = json!({"mcpServers": {&name: &server}}).to_string();
            glave_ports::settings::mcp::parse_project(&file, &BTreeMap::new())
                .map_err(|err| format!("The server can't be added: {err}."))?;
            Ok(Change::AddServer { name, server })
        }
    }
}

/// A server's name is 1 to 64 letters, digits, `-` and `_`, as its tools are named after it.
fn check_server_name(name: &str) -> Result<(), String> {
    let allowed = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    if name.is_empty() || name.len() > MOST_NAME || !name.chars().all(allowed) {
        return Err(format!(
            "The server's name must be 1 to {MOST_NAME} letters, digits, - and _."
        ));
    }
    Ok(())
}

/// What changes, in a line the person reads before they choose.
fn message(change: &Change) -> String {
    let said = match change {
        Change::AddSkill {
            name,
            description,
            text,
        } => format!(
            "Add the skill “{name}” ({} lines): {description}",
            text.lines().count()
        ),
        Change::AddServer { name, server } => format!("Connect “{name}”, {}", reach(server)),
        Change::RemoveSkill { name } => format!("Remove the skill “{name}”."),
        Change::RemoveServer { name } => format!("Disconnect “{name}”."),
    };
    let line: Vec<&str> = said.split_whitespace().collect();
    line.join(" ").chars().filter(|c| !c.is_control()).collect()
}

/// How a server is reached, as the person is shown it: its address and the headers it sends,
/// or the program it runs and what is set for it.
fn reach(server: &Value) -> String {
    if let Some(url) = server.get("url").and_then(Value::as_str) {
        let sends = match named(server, "headers") {
            headers if headers.is_empty() => String::new(),
            headers => format!(", sending {headers}"),
        };
        return format!("at {url}{sends}. Signing in to it stays yours either way.");
    }
    let mut runs: Vec<&str> = server
        .get("command")
        .and_then(Value::as_str)
        .into_iter()
        .collect();
    let args = server.get("args").and_then(Value::as_array);
    runs.extend(args.into_iter().flatten().filter_map(Value::as_str));
    let with = match named(server, "env") {
        set if set.is_empty() => String::new(),
        set => format!(", with {set} set"),
    };
    format!("which runs {}{with}.", runs.join(" "))
}

/// The names in `server`'s `key` (its headers, or what it sets), each with the variables its
/// value is filled from, so a secret it sends is seen: `Authorization from ${GITHUB_TOKEN}`.
/// A value written out is never shown.
fn named(server: &Value, key: &str) -> String {
    let Some(map) = server.get(key).and_then(Value::as_object) else {
        return String::new();
    };
    let each = map.iter().map(|(name, value)| {
        let text = value.as_str().unwrap_or_default();
        let filled: Vec<&str> = text
            .match_indices("${")
            .filter_map(|(at, _)| {
                text.get(at..)
                    .and_then(|rest| rest.split_inclusive('}').next())
            })
            .collect();
        if filled.is_empty() {
            name.clone()
        } else {
            format!("{name} from {}", filled.join(" "))
        }
    });
    each.collect::<Vec<_>>().join(", ")
}

impl Install {
    /// Where `change` may go: this project, unless the thread works in a chat's scratch folder;
    /// the person's own, where they have a folder here (and may add servers, for a server).
    /// Taking one out, only where it is.
    async fn places(&self, change: &Change) -> Result<Vec<Place>, String> {
        let mut places = Vec::new();
        if !matches!(self.workspace.info().sources, Sources::Scratch { .. }) {
            places.push(Place::Project);
        }
        if self
            .own
            .as_ref()
            .is_some_and(|own| change.is_skill() || own.servers)
        {
            places.push(Place::Everywhere);
        }
        if change.adds() {
            return Ok(places);
        }
        let mut there = Vec::new();
        for place in places {
            if places::holds(&self.workspace, self.own.as_ref(), place, change).await? {
                there.push(place);
            }
        }
        // A project's skill is a file the model's own tools change, with the project's others.
        if matches!(change, Change::RemoveSkill { .. }) && there.contains(&Place::Project) {
            there.retain(|place| *place != Place::Project);
            if there.is_empty() {
                return Err(format!(
                    "The skill “{}” is in this project's files: delete its folder with your own \
                     tools, so it shows with the project's other changes.",
                    change.name()
                ));
            }
        }
        Ok(there)
    }

    /// What the person is asked about `change`, or why nothing can be asked.
    async fn ask(&self, change: &Change) -> Result<ServiceAsks, String> {
        let places = self.places(change).await?;
        if places.is_empty() {
            let name = change.name();
            return Err(match change {
                Change::RemoveSkill { .. } => {
                    format!(
                        "There is no skill “{name}” glave added in this project or the person's own."
                    )
                }
                Change::RemoveServer { .. } => {
                    format!(
                        "There is no server “{name}” in this project's .mcp.json or the person's own."
                    )
                }
                _ => "Nothing can be added here: this is a chat, and the person has no folder of \
                      their own on this server."
                    .to_owned(),
            });
        }
        let label = if change.adds() {
            "Where should it go?"
        } else {
            "Where should it come out of?"
        };
        let field = ServiceField {
            name: WHERE.to_owned(),
            label: label.to_owned(),
            description: None,
            required: true,
            kind: ServiceFieldKind::Choice {
                options: places.iter().map(|place| place.option()).collect(),
                multiple: false,
                default: Vec::new(),
            },
        };
        let mut replaces = Vec::new();
        if change.adds() {
            for place in &places {
                if places::holds(&self.workspace, self.own.as_ref(), *place, change).await? {
                    replaces.push(place.whose());
                }
            }
        }
        let message = match replaces.as_slice() {
            [] => message(change),
            whose => format!(
                "{} It replaces the one by that name in {}.",
                message(change),
                whose.join(" and in ")
            ),
        };
        let state = serde_json::to_value(change).map_err(|err| err.to_string())?;
        Ok(ServiceAsks {
            service: ASKED_BY_GLAVE.to_owned(),
            message,
            fields: vec![field],
            state,
        })
    }

    /// What came of the person's `answer` to where `change` goes.
    async fn answered(&self, change: &Change, answer: ServiceAnswer) -> ToolOutput {
        let ServiceAnswer::Answered { values } = answer else {
            return ToolOutput::text(true, "The person said no, so nothing changed.");
        };
        let picked = values.get(WHERE).and_then(Value::as_str);
        let Some(place) = picked.and_then(Place::from_value) else {
            return ToolOutput::text(true, "The person chose no place, so nothing changed.");
        };
        match self.places(change).await {
            Ok(places) if places.contains(&place) => {}
            Ok(_) => {
                return ToolOutput::text(true, "That place no longer fits, so nothing changed.");
            }
            Err(why) => return ToolOutput::text(true, why),
        }
        match places::apply(&self.workspace, self.own.as_ref(), place, change).await {
            Ok(done) => ToolOutput::text(false, done),
            Err(why) => ToolOutput::text(true, why),
        }
    }

    /// What a call comes to with `given`, the person's answer when there is one.
    async fn called(&self, arguments: &Arguments, given: Option<ServiceGiven>) -> Called {
        let change = match &given {
            Some(given) => serde_json::from_value(given.state.clone()).map_err(|e| e.to_string()),
            None => change(arguments),
        };
        let change = match change {
            Ok(change) => change,
            Err(why) => return Called::Done(ToolOutput::text(true, why)),
        };
        if self.workspace.plans() {
            return Called::Done(ToolOutput::text(true, PLANNING));
        }
        match given {
            Some(given) => Called::Done(self.answered(&change, given.answer).await),
            None => match self.ask(&change).await {
                Ok(asks) => Called::Asks(asks),
                Err(why) => Called::Done(ToolOutput::text(true, why)),
            },
        }
    }
}

impl ToolSource for Install {
    fn tools(&self) -> BoxFuture<'_, Result<Vec<ToolInfo>, ToolError>> {
        let info = ToolInfo {
            spec: spec(),
            effect: Effect::Changes,
            parallel: false,
        };
        Box::pin(std::future::ready(Ok(vec![info])))
    }

    /// It asks the person itself, so nothing asks before it.
    fn assess<'a>(&'a self, _: &'a str, _: &'a Arguments) -> BoxFuture<'a, Assessment> {
        Box::pin(std::future::ready(Assessment::Reads))
    }

    fn describe(&self, _: &str, arguments: &Arguments) -> String {
        match change(arguments) {
            Ok(Change::AddSkill { name, .. }) => format!("install the skill {name}"),
            Ok(Change::AddServer { name, .. }) => format!("install the server {name}"),
            Ok(Change::RemoveSkill { name }) => format!("remove the skill {name}"),
            Ok(Change::RemoveServer { name }) => format!("remove the server {name}"),
            Err(_) => INSTALL.to_owned(),
        }
    }

    fn call<'a>(
        &'a self,
        _: &'a str,
        _: &'a Arguments,
        _: &'a dyn Cancellation,
    ) -> BoxFuture<'a, ToolOutput> {
        Box::pin(std::future::ready(ToolOutput::text(true, NO_ONE)))
    }

    fn call_asking<'a>(
        &'a self,
        _: &'a str,
        arguments: &'a Arguments,
        given: Option<ServiceGiven>,
        _: &'a dyn Cancellation,
    ) -> BoxFuture<'a, Called> {
        Box::pin(self.called(arguments, given))
    }
}

#[cfg(test)]
mod tests;
