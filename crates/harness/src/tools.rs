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

use glave_event::Arguments;
use glave_ports::ToolSpec;
use glave_ports::computer::Sandbox;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use workspace::Workspace;

use crate::background::Commands;
use glave_ports::Cancellation;

/// What a tool call produced, as the model will see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutcome {
    pub is_error: bool,
    pub text: String,
}

impl ToolOutcome {
    pub(crate) fn ok(text: impl Into<String>) -> Self {
        Self {
            is_error: false,
            text: text.into(),
        }
    }

    pub(crate) fn error(text: impl Into<String>) -> Self {
        Self {
            is_error: true,
            text: text.into(),
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
            Self::Bash => (
                "Run a bash command at the project root and see its output and exit code. Each \
                 call starts fresh: cd does not carry over, so use `cd dir && command`. Default \
                 timeout 120 seconds, at most 600; a command still running then keeps running \
                 in the background. For a server, a watcher or a long build, set background: \
                 it starts there at once and keeps running, and command_output shows what it \
                 prints or stops it. Commands run in a sandbox. If it blocked a command the \
                 task needs, run that same command again with outside_sandbox and a reason: the \
                 person is asked to allow it.",
                json!({"type": "object", "properties": {
                    "command": {"type": "string"},
                    "timeout_secs": {"type": "integer", "minimum": 1, "maximum": 600},
                    "background": {"type": "boolean", "description": "Start it in the background and answer at once with its number"},
                    "outside_sandbox": {"type": "boolean", "description": "Run outside the sandbox. Only after the sandbox blocked this same command; the person is asked every time"},
                    "reason": {"type": "string", "description": "With outside_sandbox: why the command needs it, in one sentence for the person"}
                }, "required": ["command"]}),
            ),
            Self::CommandOutput => background::spec(),
        };
        ToolSpec {
            name: self.name().to_owned(),
            description: description.to_owned(),
            parameters,
        }
    }

    /// The tool as it is offered where commands run in `sandbox`: `bash` takes `network`, for
    /// asking to run one command with it, only where the sandbox has none.
    pub(crate) fn spec_in(self, sandbox: Option<&Sandbox>) -> ToolSpec {
        let mut spec = self.spec();
        if self != Self::Bash || sandbox.is_none_or(|sandbox| sandbox.network) {
            return spec;
        }
        spec.description.push_str(
            " The sandbox has no network here: if a command the task needs must reach the \
             internet, run it again with network and a reason instead; the person is asked to \
             allow it, and it stays in the sandbox otherwise.",
        );
        if let Some(properties) = spec
            .parameters
            .get_mut("properties")
            .and_then(Value::as_object_mut)
        {
            properties.insert(
                "network".to_owned(),
                json!({"type": "boolean", "description": "Run with the internet, in the sandbox otherwise. Only after this same command failed without it; the person is asked every time"}),
            );
            properties.insert(
                "reason".to_owned(),
                json!({"type": "string", "description": "With outside_sandbox or network: why the command needs it, in one sentence for the person"}),
            );
        }
        spec
    }

    /// Runs the tool with the model's arguments, keeping the thread's commands in the
    /// background in `commands`. Every failure becomes an error the model can read and act on;
    /// nothing here aborts the turn. A command in the foreground stops when `cancel` fires.
    pub(crate) async fn run(
        self,
        arguments: &Arguments,
        (workspace, commands): (&Workspace, &Commands),
        cancel: &dyn Cancellation,
    ) -> ToolOutcome {
        match self {
            Self::Search => match parse(self, arguments) {
                Ok(args) => search::search(workspace, args).await,
                Err(outcome) => outcome,
            },
            Self::Glob => match parse(self, arguments) {
                Ok(args) => glob::glob(workspace, args).await,
                Err(outcome) => outcome,
            },
            Self::Read => match parse(self, arguments) {
                Ok(args) => read::read(workspace, args).await,
                Err(outcome) => outcome,
            },
            Self::Write => match parse(self, arguments) {
                Ok(args) => write::write(workspace, args).await,
                Err(outcome) => outcome,
            },
            Self::Edit => match parse(self, arguments) {
                Ok(args) => edit::edit(workspace, args).await,
                Err(outcome) => outcome,
            },
            Self::Bash => match parse(self, arguments) {
                Ok(args) => bash::bash(workspace, commands, args, cancel).await,
                Err(outcome) => outcome,
            },
            Self::CommandOutput => match parse(self, arguments) {
                Ok(args) => background::command_output(commands, args, cancel).await,
                Err(outcome) => outcome,
            },
        }
    }
}

fn parse<T: DeserializeOwned>(tool: Tool, arguments: &Arguments) -> Result<T, ToolOutcome> {
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
        ToolOutcome::error(format!(
            "The arguments for {} are invalid: {e}. Expected: {}",
            tool.name(),
            tool.spec().parameters
        ))
    })
}

#[cfg(test)]
mod tests;
