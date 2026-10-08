//! The four tools: read, write, edit, bash. Their descriptions and schemas are kept short and
//! plain so smaller models use them well.

mod bash;
mod edit;
mod read;
pub(crate) mod workspace;
mod write;

use glave_event::Arguments;
use glave_ports::ToolSpec;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use workspace::Workspace;

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
    Read,
    Write,
    Edit,
    Bash,
}

impl Tool {
    pub const ALL: [Self; 4] = [Self::Read, Self::Write, Self::Edit, Self::Bash];

    /// Finds a tool by name, forgiving case and stray whitespace.
    pub fn named(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "edit" => Some(Self::Edit),
            "bash" => Some(Self::Bash),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Edit => "edit",
            Self::Bash => "bash",
        }
    }

    /// Whether the tool can change anything, and so needs the person's approval.
    pub fn changes_things(self) -> bool {
        !matches!(self, Self::Read)
    }

    fn spec(self) -> ToolSpec {
        let (description, parameters) = match self {
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
                 timeout 120 seconds, at most 600.",
                json!({"type": "object", "properties": {
                    "command": {"type": "string"},
                    "timeout_secs": {"type": "integer", "minimum": 1, "maximum": 600}
                }, "required": ["command"]}),
            ),
        };
        ToolSpec {
            name: self.name().to_owned(),
            description: description.to_owned(),
            parameters,
        }
    }

    /// Runs the tool with the model's arguments. Every failure becomes an error the model can
    /// read and act on; nothing here aborts the turn.
    pub(crate) async fn run(self, arguments: &Arguments, workspace: &Workspace) -> ToolOutcome {
        match self {
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
                Ok(args) => bash::bash(workspace, args).await,
                Err(outcome) => outcome,
            },
        }
    }
}

/// The specs offered to the model, in a stable order.
pub fn specs() -> Vec<ToolSpec> {
    Tool::ALL.iter().map(|tool| tool.spec()).collect()
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
