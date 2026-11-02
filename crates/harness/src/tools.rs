//! The tools: search, glob, read, write, edit, bash, and command_output for commands bash keeps
//! running in the background. Their descriptions and schemas are kept short and plain so
//! smaller models use them well.

mod background;
mod bash;
mod captured;
mod edit;
mod glob;
mod read;
mod search;
pub(crate) mod workspace;
mod write;

pub(crate) use background::named as command_named;
pub(crate) use bash::outside_sandbox;
pub(crate) use captured::Captured;

use glave_event::{Arguments, ToolKind};
use glave_ports::{ToolFile, ToolSpec};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use workspace::Workspace;

use crate::background::Commands;
use crate::sandbox::Asks;
use glave_ports::Cancellation;

/// What a tool call produced, as the model will see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    pub is_error: bool,
    pub text: String,
    /// A file it hands the model after the text, such as an image `read` read.
    pub file: Option<ToolFile>,
}

impl ToolOutcome {
    pub(crate) fn ok(text: impl Into<String>) -> Self {
        Self {
            is_error: false,
            text: text.into(),
            file: None,
        }
    }

    pub(crate) fn error(text: impl Into<String>) -> Self {
        Self {
            is_error: true,
            text: text.into(),
            file: None,
        }
    }
}

/// The tools the harness offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    Search,
    Glob,
    Read,
    Write,
    Edit,
    Bash,
    CommandOutput,
}

impl Tool {
    pub const ALL: [Self; 7] = [
        Self::Search,
        Self::Glob,
        Self::Read,
        Self::Write,
        Self::Edit,
        Self::Bash,
        Self::CommandOutput,
    ];

    /// What kind of work it does.
    pub(crate) fn kind(self) -> ToolKind {
        match self {
            Self::Search | Self::Glob => ToolKind::Search,
            Self::Read => ToolKind::Read,
            Self::Write | Self::Edit => ToolKind::Edit,
            Self::Bash | Self::CommandOutput => ToolKind::Command,
        }
    }

    /// Finds a tool by name, forgiving case and stray whitespace.
    pub fn named(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "search" => Some(Self::Search),
            "glob" => Some(Self::Glob),
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "edit" => Some(Self::Edit),
            "bash" => Some(Self::Bash),
            "command_output" => Some(Self::CommandOutput),
            _ => None,
        }
    }

    /// The file a call of `tool` with `arguments` changes when it succeeds: a `write`'s or an
    /// `edit`'s, as it names it; none for any other call, or one that names no file.
    pub fn changes<'a>(tool: &str, arguments: &'a Arguments) -> Option<&'a str> {
        if !matches!(Self::named(tool)?, Self::Write | Self::Edit) {
            return None;
        }
        Self::touches(tool, arguments)
    }

    /// The path a call of `tool` with `arguments` reads or changes: a `read`'s, a `write`'s or
    /// an `edit`'s, as it names it; none for any other call, or one that names no path.
    pub(crate) fn touches<'a>(tool: &str, arguments: &'a Arguments) -> Option<&'a str> {
        if !matches!(Self::named(tool)?, Self::Read | Self::Write | Self::Edit) {
            return None;
        }
        let Arguments::Json(map) = arguments else {
            return None;
        };
        map.get("path")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Search => "search",
            Self::Glob => "glob",
            Self::Read => "read",
            Self::Write => "write",
            Self::Edit => "edit",
            Self::Bash => "bash",
            Self::CommandOutput => "command_output",
        }
    }

    pub(crate) fn spec(self) -> ToolSpec {
        let (description, parameters) = match self {
            Self::Search => (
                "Search file contents for a regular expression, like ripgrep. Shows matching \
                 lines as path:line:text, at most 100. Skips files git ignores, binary files and \
                 files that hold secrets. Use this instead of grep in bash.",
                json!({"type": "object", "properties": {
                    "pattern": {"type": "string", "description": "Regular expression, or plain text"},
                    "path": {"type": "string", "description": "Folder or file to search; default the whole project"},
                    "glob": {"type": "string", "description": "Only files matching this, such as *.rs or src/**/*.ts"},
                    "ignore_case": {"type": "boolean"}
                }, "required": ["pattern"]}),
            ),
            Self::Glob => (
                "Find files by name or path, newest first, at most 100. *.rs matches file names \
                 in any folder; src/**/*.rs matches paths. Skips files git ignores.",
                json!({"type": "object", "properties": {
                    "pattern": {"type": "string", "description": "Such as *.rs, **/test_*.py or src/**/*.ts"},
                    "path": {"type": "string", "description": "Folder to look in; default the whole project"}
                }, "required": ["pattern"]}),
            ),
            Self::Read => (
                "Read a file, shown with line numbers, or list a folder. Reads up to 2000 lines; \
                 use offset and limit for more.",
                json!({"type": "object", "properties": {
                    "path": {"type": "string", "description": "File or folder, relative to the project"},
                    "offset": {"type": "integer", "minimum": 1, "description": "First line to read"},
                    "limit": {"type": "integer", "minimum": 1, "description": "How many lines"}
                }, "required": ["path"]}),
            ),
            Self::Write => (
                "Create a file, or replace one you have read. Creates missing folders. To change \
                 part of a file, use edit.",
                json!({"type": "object", "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string", "description": "The whole new file"}
                }, "required": ["path", "content"]}),
            ),
            Self::Edit => (
                "Replace text in a file you have read. old_string must match the file exactly and \
                 only once; include a few surrounding lines to make it unique. Do not include the \
                 line numbers from read.",
                json!({"type": "object", "properties": {
                    "path": {"type": "string"},
                    "old_string": {"type": "string", "description": "Text to replace"},
                    "new_string": {"type": "string", "description": "Replacement text"},
                    "replace_all": {"type": "boolean", "description": "Replace every occurrence"}
                }, "required": ["path", "old_string", "new_string"]}),
            ),
            Self::Bash => return bash::spec(Asks::Outside),
            Self::CommandOutput => background::spec(),
        };
        ToolSpec {
            name: self.name().to_owned(),
            description: description.to_owned(),
            parameters,
        }
    }

    /// The tool as it is offered on `workspace`: `bash` as its commands may ask to leave the
    /// sandbox there (`Asks`).
    pub(crate) fn spec_in(self, workspace: &Workspace) -> ToolSpec {
        match self {
            Self::Bash => bash::spec(Asks::of(workspace)),
            _ => self.spec(),
        }
    }

    /// Runs the tool with the model's arguments, keeping the thread's commands in the
    /// background in `commands`, its output within `room` bytes (`room::tool_room`). Every
    /// failure becomes an error the model can read and act on; nothing here aborts the turn. A
    /// command in the foreground stops when `cancel` fires.
    pub(crate) async fn run(
        self,
        arguments: &Arguments,
        (workspace, commands): (&Workspace, &Commands),
        room: usize,
        cancel: &dyn Cancellation,
    ) -> ToolOutcome {
        match self {
            Self::Search => match parse(self, arguments, Some(workspace)) {
                Ok(args) => search::search(workspace, args).await,
                Err(outcome) => outcome,
            },
            Self::Glob => match parse(self, arguments, Some(workspace)) {
                Ok(args) => glob::glob(workspace, args).await,
                Err(outcome) => outcome,
            },
            Self::Read => match parse(self, arguments, Some(workspace)) {
                Ok(args) => read::read(workspace, args, room).await,
                Err(outcome) => outcome,
            },
            Self::Write => match parse(self, arguments, Some(workspace)) {
                Ok(args) => write::write(workspace, args).await,
                Err(outcome) => outcome,
            },
            Self::Edit => match parse(self, arguments, Some(workspace)) {
                Ok(args) => edit::edit(workspace, args).await,
                Err(outcome) => outcome,
            },
            Self::Bash => match parse(self, arguments, Some(workspace)) {
                Ok(args) => bash::bash(workspace, commands, args, room, cancel).await,
                Err(outcome) => outcome,
            },
            Self::CommandOutput => match parse(self, arguments, Some(workspace)) {
                Ok(args) => {
                    let hidden = workspace.hidden();
                    background::command_output(commands, args, (room, hidden), cancel).await
                }
                Err(outcome) => outcome,
            },
        }
    }
}

/// The arguments of a call of `tool`, or an error the model can act on, which shows the
/// parameters the tool takes as it is offered on `workspace`, when there is one.
fn parse<T: DeserializeOwned>(
    tool: Tool,
    arguments: &Arguments,
    workspace: Option<&Workspace>,
) -> Result<T, ToolOutcome> {
    let map = match arguments {
        Arguments::Json(map) => map.clone(),
        Arguments::Malformed(raw) => {
            return Err(ToolOutcome::error(format!(
                "The arguments for {} are not valid JSON: {}",
                tool.name(),
                raw.chars().take(200).collect::<String>()
            )));
        }
    };
    serde_json::from_value(Value::Object(map)).map_err(|e| {
        let offered = workspace.map_or_else(|| tool.spec(), |workspace| tool.spec_in(workspace));
        ToolOutcome::error(format!(
            "The arguments for {} are invalid: {e}. Expected: {}",
            tool.name(),
            offered.parameters
        ))
    })
}

#[cfg(test)]
mod tests;
