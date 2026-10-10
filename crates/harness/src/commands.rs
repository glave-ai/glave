//! Custom commands: a Markdown file whose name is the command, as Claude Code, Codex and
//! OpenCode write them. Typing `/name some words` sends the file's body as the prompt, with
//! `$ARGUMENTS` replaced by the words and `$1` to `$9` by each of them. A command is only text
//! the person sends, so it runs nothing and needs no trust. Optional frontmatter may give a
//! `description` and an `argument-hint`, for listings; other fields are ignored.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::skills::frontmatter::{self, Value};

/// The folders in a project, and in the person's home, that hold commands: glave's own, then
/// Claude Code's.
pub const COMMAND_FOLDERS: [&str; 2] = [".glave/commands", ".claude/commands"];

/// The largest command file read, in bytes; a bigger one is skipped with a warning.
const MAX_FILE: u64 = 256 * 1024;

/// A command the person can type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// What follows the `/`: the file's name without `.md`.
    pub name: String,
    pub description: Option<String>,
    /// What to type after the name, such as `<issue number>`.
    pub argument_hint: Option<String>,
    /// The text sent, before its arguments are put in.
    pub body: String,
    pub file: PathBuf,
}

/// The commands found, by name, and each file skipped with why.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoundCommands {
    pub commands: Vec<Command>,
    pub warnings: Vec<String>,
}

impl Command {
    /// The prompt this command sends with `arguments`: the body with `$ARGUMENTS`, `$1` to
    /// `$9` and `$$` filled in. A body that uses none of them gets the arguments after it,
    /// so none are lost, as Claude Code does.
    pub fn expand(&self, arguments: &str) -> String {
        let arguments = arguments.trim();
        let words: Vec<&str> = arguments.split_whitespace().collect();
        let mut out = String::with_capacity(self.body.len().saturating_add(arguments.len()));
        let mut used = false;
        let mut rest = self.body.as_str();
        while let Some(at) = rest.find('$') {
            out.push_str(rest.get(..at).unwrap_or_default());
            let after = rest.get(at.saturating_add(1)..).unwrap_or_default();
            let (text, taken) = if after.starts_with("ARGUMENTS") {
                used = true;
                (arguments, "ARGUMENTS".len())
            } else if let Some(digit) = after.chars().next().filter(|c| ('1'..='9').contains(c)) {
                used = true;
                let index = digit
                    .to_digit(10)
                    .map_or(0, |d| d as usize)
                    .saturating_sub(1);
                (words.get(index).copied().unwrap_or_default(), 1)
            } else if after.starts_with('$') {
                ("$", 1)
            } else {
                ("$", 0)
            };
            out.push_str(text);
            rest = after.get(taken..).unwrap_or_default();
        }
        out.push_str(rest);
        if !used && !arguments.is_empty() {
            out = format!("{}\n\nARGUMENTS: {arguments}", out.trim_end());
        }
        out
    }
}

/// The command `prompt` invokes, with what follows its name: `/review 12` invokes `review`
/// with `12`. `None` when the prompt names no command in `commands`, so a prompt that only
/// starts with a slash, such as a path, goes to the model as written.
pub fn invoked<'a>(commands: &'a [Command], prompt: &'a str) -> Option<(&'a Command, &'a str)> {
    let rest = prompt.trim_start().strip_prefix('/')?;
    let (name, arguments) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    commands
        .iter()
        .find(|command| command.name == name)
        .map(|command| (command, arguments))
}

/// The folders commands are looked for in, first found first: in each folder from `root` up
/// to its repository's root, the project folders; then `personal`, the person's own. A name
/// is kept by the first folder that has it, as skills are.
pub fn command_folders(root: &Path, personal: &[PathBuf]) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = crate::instructions::project_dirs(root)
        .iter()
        .rev()
        .flat_map(|dir| COMMAND_FOLDERS.map(|folder| dir.join(folder)))
        .collect();
    folders.extend_from_slice(personal);
    folders
}

/// Every command in `folders` (each `*.md` file directly in one), sorted by name. A file
/// that cannot be read, or whose name or frontmatter does not fit, is skipped with a warning.
pub async fn find_commands(folders: &[PathBuf]) -> FoundCommands {
    let mut found = FoundCommands::default();
    let mut names: HashMap<String, PathBuf> = HashMap::new();
    for folder in folders {
        for file in markdown_files(folder, &mut found.warnings).await {
            let command = match read(&file).await {
                Ok(command) => command,
                Err(why) => {
                    found
                        .warnings
                        .push(format!("skipped {}: {why}", file.display()));
                    continue;
                }
            };
            if let Some(kept) = names.get(&command.name) {
                found.warnings.push(format!(
                    "skipped {}: the command /{} in {} comes first",
                    file.display(),
                    command.name,
                    kept.display()
                ));
                continue;
            }
            names.insert(command.name.clone(), file.clone());
            found.commands.push(command);
        }
    }
    found.commands.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// The `.md` files directly in `folder`, in name order.
async fn markdown_files(folder: &Path, warnings: &mut Vec<String>) -> Vec<PathBuf> {
    let mut entries = match tokio::fs::read_dir(folder).await {
        Ok(entries) => entries,
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Vec::new();
        }
        Err(err) => {
            warnings.push(format!("could not read {}: {err}", folder.display()));
            return Vec::new();
        }
    };
    let mut files = Vec::new();
    loop {
        match entries.next_entry().await {
            Ok(Some(entry)) => {
                let path = entry.path();
                let markdown = path.extension().is_some_and(|ext| ext == "md");
                if markdown && tokio::fs::metadata(&path).await.is_ok_and(|m| m.is_file()) {
                    files.push(path);
                }
            }
            Ok(None) => break,
            Err(err) => {
                warnings.push(format!("could not read {}: {err}", folder.display()));
                break;
            }
        }
    }
    files.sort();
    files
}

/// The command in `file`, or why it cannot be one.
async fn read(file: &Path) -> Result<Command, String> {
    let name = file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default()
        .to_owned();
    check_name(&name)?;
    let size = tokio::fs::metadata(file)
        .await
        .map_err(|err| err.to_string())?
        .len();
    if size > MAX_FILE {
        return Err(format!(
            "it is {size} bytes; a command is at most {MAX_FILE}"
        ));
    }
    let text = tokio::fs::read_to_string(file)
        .await
        .map_err(|err| err.to_string())?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let (description, argument_hint, body) = if text.starts_with("---") {
        let document = frontmatter::parse(text)?;
        let field = |key: &str| match document.fields.get(key) {
            Some(Value::Text(text)) => Ok(Some(text.trim().to_owned()).filter(|t| !t.is_empty())),
            Some(Value::Other) => Err(format!("{key} must be text")),
            None => Ok(None),
        };
        (
            field("description")?,
            field("argument-hint")?,
            document.body,
        )
    } else {
        (None, None, text.to_owned())
    };
    Ok(Command {
        name,
        description,
        argument_hint,
        body: body.trim().to_owned(),
        file: file.to_path_buf(),
    })
}

/// A name is letters, digits, `-`, `_` and `.`, so `/name` reads as one word.
fn check_name(name: &str) -> Result<(), String> {
    let allowed = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | '.');
    if name.is_empty() || !name.chars().all(allowed) {
        return Err(format!(
            "its name {name:?} may hold only letters, digits, -, _ and ."
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
