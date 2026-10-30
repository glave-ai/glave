//! AGENTS.md: the instructions a project gives the agents that work in it. They are read when a
//! thread starts and sent as the first part of its first message, the way Codex and Claude Code
//! send theirs. They ride in a user message, not the system prompt: they come from the
//! repository, so they steer the model without outranking glave's own instructions, and
//! approvals still gate every action. A subfolder's follow the first tool result that reaches
//! it (`nested`). On a person's own computer they are read there, through the computer port
//! ([`agents_md_on`]), never from the server's disk.

mod nested;

use std::path::{Path, PathBuf};

use glave_event::{InstructionsFile, Part};
use glave_ports::Message;
use glave_ports::computer::ComputerError;

use crate::tools::workspace::Workspace;

/// The file agents look for.
pub const AGENTS_MD: &str = "AGENTS.md";

/// The most instruction text sent, as Codex caps it: what a thread opens with and what its
/// subfolders add (`nested`) together.
const MAX_BYTES: usize = 32 * 1024;

/// What follows instructions cut short at [`MAX_BYTES`].
const CUT_SHORT: &str = "\n[cut short: instructions are limited to 32 KiB]";

/// What starts a block of instructions, before the folder they are for.
const HEADING: &str = "# AGENTS.md instructions for ";

/// The person's own instructions on their own computer, in their home there: glave's folder
/// in it, as on a laptop.
const PERSONAL: &str = ".glave/AGENTS.md";

/// The instructions for a workspace, ready to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instructions {
    /// The message part the model sees.
    pub text: String,
    /// The files it came from, in the order they appear.
    pub files: Vec<PathBuf>,
}

/// Reads the person's own instructions (`user_file`, when it exists), then every AGENTS.md from
/// the repository's root down to `root`, so the most specific come last. Without a repository
/// (no `.git` above `root`), only `root` is looked in. `None` when there are no instructions.
pub async fn agents_md(
    root: &Path,
    user_file: Option<&Path>,
) -> std::io::Result<Option<Instructions>> {
    let mut candidates: Vec<PathBuf> = user_file.map(Path::to_path_buf).into_iter().collect();
    candidates.extend(project_dirs(root).iter().map(|dir| dir.join(AGENTS_MD)));
    let mut read = Sections::default();
    for path in candidates {
        if read.full() {
            break;
        }
        let text = match tokio::fs::read_to_string(&path).await {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => {
                return Err(std::io::Error::new(
                    err.kind(),
                    format!("could not read {}: {err}", path.display()),
                ));
            }
        };
        let canonical = tokio::fs::canonicalize(&path).await?;
        read.add(&path, canonical, &text);
    }
    Ok(read.done(root))
}

/// The instructions `messages` opened with, as they were sent: the first part of the first
/// message, when it is a block of them.
pub(crate) fn opened_with(messages: &[Message]) -> Option<String> {
    let Some(Message::User { parts }) = messages.first() else {
        return None;
    };
    match parts.first() {
        Some(Part::Text { text }) if text.starts_with(HEADING) => Some(text.clone()),
        _ => None,
    }
}

/// What [`agents_md`] reads for a thread working at `root` on a computer reached through the
/// port, read there through `home`, the person's home on it, which `root` is inside: their own
/// instructions (`.glave/AGENTS.md` in the home), then every AGENTS.md from the repository's
/// root down to `root`, never above the home, file by file as they are sent ([`compose`]).
pub(crate) async fn agents_md_on(
    home: &Workspace,
    root: &Path,
) -> Result<Vec<InstructionsFile>, ComputerError> {
    let mut files = vec![home.root().join(PERSONAL)];
    files.extend(
        dirs_on(home, root)
            .await?
            .iter()
            .map(|dir| dir.join(AGENTS_MD)),
    );
    read_on(home, files).await
}

/// The person's own instructions on their own computer, read there through `home`, their home
/// on it, as [`agents_md_on`] reads them: what a chat opens with, as on a laptop.
pub(crate) async fn personal_on(home: &Workspace) -> Result<Vec<InstructionsFile>, ComputerError> {
    read_on(home, vec![home.root().join(PERSONAL)]).await
}

/// `files`, read through `home`, in order, within [`MAX_BYTES`] together. A file that is not
/// there, or that the computer refuses to read, is left out; any other failure (the computer or
/// its home gone) fails the read, so nothing half read is taken for it.
async fn read_on(
    home: &Workspace,
    files: Vec<PathBuf>,
) -> Result<Vec<InstructionsFile>, ComputerError> {
    let base = home.root();
    let mut read = Sections::default();
    for file in files {
        if read.full() {
            break;
        }
        let Some(path) = inside(base, &file) else {
            continue;
        };
        if !there(home, &path).await? {
            continue;
        }
        match home.read(&path).await {
            Ok(bytes) => read.add(
                &file,
                file.clone(),
                &String::from_utf8_lossy(bytes.as_slice()),
            ),
            Err(ComputerError::Refused(_)) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(read.blocks)
}

/// The folders [`project_dirs`] looks in, found through `home`: from the repository's root
/// down to `root`, or `root` alone, never above the home.
async fn dirs_on(home: &Workspace, root: &Path) -> Result<Vec<PathBuf>, ComputerError> {
    let base = home.root();
    let mut dirs = Vec::new();
    for dir in root.ancestors().take_while(|dir| dir.starts_with(base)) {
        dirs.push(dir.to_path_buf());
        let Some(git) = inside(base, &dir.join(".git")) else {
            continue;
        };
        if there(home, &git).await? {
            dirs.reverse();
            return Ok(dirs);
        }
    }
    Ok(vec![root.to_path_buf()])
}

/// Whether something is at `path` in `home`, one the computer lets glave see. A path with
/// nothing there answers so, so one not found means the home itself is gone: an error, never
/// taken for an empty folder.
async fn there(home: &Workspace, path: &str) -> Result<bool, ComputerError> {
    match home.computers().stat(home.info().id, path).await {
        Ok(stat) => Ok(stat.kind.is_some()),
        Err(ComputerError::Refused(_)) => Ok(false),
        Err(error) => Err(error),
    }
}

/// `path` as the home at `base` names it: relative to it.
fn inside(base: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(base).ok()?;
    Some(relative.to_string_lossy().into_owned())
}

/// Instructions as they are read, file by file, within [`MAX_BYTES`] together.
struct Sections {
    /// The files read, as each is known, so none is sent twice.
    files: Vec<PathBuf>,
    blocks: Vec<InstructionsFile>,
    remaining: usize,
}

impl Default for Sections {
    fn default() -> Self {
        Self {
            files: Vec::new(),
            blocks: Vec::new(),
            remaining: MAX_BYTES,
        }
    }
}

impl Sections {
    /// Whether no more fits.
    fn full(&self) -> bool {
        self.remaining == 0
    }

    /// Adds `text`, read from `path`, known as `file`, unless it is blank or `file` was added
    /// already.
    fn add(&mut self, path: &Path, file: PathBuf, text: &str) {
        if text.trim().is_empty() || self.files.contains(&file) {
            return;
        }
        let (text, cut) = within(text.trim(), self.remaining);
        self.remaining = self.remaining.saturating_sub(text.len());
        let note = if cut { CUT_SHORT } else { "" };
        self.blocks.push(InstructionsFile {
            path: path.display().to_string(),
            text: format!("{text}{note}"),
        });
        self.files.push(file);
    }

    /// The instructions for a workspace at `root`, or `None` when nothing was read.
    fn done(self, root: &Path) -> Option<Instructions> {
        Some(Instructions {
            text: compose(root, &self.blocks)?,
            files: self.files,
        })
    }
}

/// The instructions for a workspace at `root` made of `files`, as the model is sent them; `None`
/// when there are none.
pub(crate) fn compose(root: &Path, files: &[InstructionsFile]) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    let blocks: Vec<String> = files
        .iter()
        .map(|file| format!("From {}:\n{}", file.path, file.text))
        .collect();
    Some(format!(
        "{HEADING}{}\n\n<INSTRUCTIONS>\n{}\n</INSTRUCTIONS>",
        root.display(),
        blocks.join("\n\n")
    ))
}

/// The folders to look in: from the repository's root down to `root`, or `root` alone.
pub(crate) fn project_dirs(root: &Path) -> Vec<PathBuf> {
    let repository = root
        .ancestors()
        .find(|dir| dir.join(".git").exists())
        .map(Path::to_path_buf);
    let mut dirs: Vec<PathBuf> = match repository {
        Some(top) => root
            .ancestors()
            .take_while(|dir| dir.starts_with(&top))
            .map(Path::to_path_buf)
            .collect(),
        None => vec![root.to_path_buf()],
    };
    dirs.reverse();
    dirs
}

/// `text` cut to at most `limit` bytes on a character boundary, and whether it was cut.
pub(crate) fn within(text: &str, limit: usize) -> (&str, bool) {
    if text.len() <= limit {
        return (text, false);
    }
    let end = (0..=limit)
        .rev()
        .find(|end| text.is_char_boundary(*end))
        .unwrap_or(0);
    (text.get(..end).unwrap_or_default(), true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glave_ports::testing::TempDir;

    fn folder(name: &str) -> TempDir {
        TempDir::new(&format!("agents-md-{name}")).unwrap()
    }

    fn load(root: &Path, user_file: Option<&Path>) -> Option<Instructions> {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(agents_md(root, user_file))
            .unwrap()
    }

    #[test]
    fn a_repository_is_read_from_its_root_down_and_the_person_first() {
        let top = folder("repo");
        let inner = top.join("crates/app");
        std::fs::create_dir_all(top.join(".git")).unwrap();
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(top.join(AGENTS_MD), "Use tabs.\n").unwrap();
        std::fs::write(top.join("crates").join(AGENTS_MD), "  \n").unwrap();
        std::fs::write(inner.join(AGENTS_MD), "Run make test.\n").unwrap();
        let home = folder("home");
        let mine = home.join(AGENTS_MD);
        std::fs::write(&mine, "Be brief.").unwrap();
        let loaded = load(&inner, Some(&mine)).unwrap();
        assert_eq!(
            loaded.files,
            [mine.clone(), top.join(AGENTS_MD), inner.join(AGENTS_MD)],
            "blank files are skipped"
        );
        let text = loaded.text;
        assert!(
            text.starts_with(&format!("# AGENTS.md instructions for {}", inner.display())),
            "{text}"
        );
        let order = ["Be brief.", "Use tabs.", "Run make test."].map(|line| text.find(line));
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{text}");
        assert!(text.ends_with("Run make test.\n</INSTRUCTIONS>"), "{text}");
    }

    #[test]
    fn without_a_repository_only_the_folder_itself_is_read() {
        let outer = folder("plain");
        let inner = outer.join("project");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(outer.join(AGENTS_MD), "Not ours.").unwrap();
        assert_eq!(load(&inner, None), None);
        std::fs::write(inner.join(AGENTS_MD), "Ours.").unwrap();
        assert_eq!(load(&inner, None).unwrap().files, [inner.join(AGENTS_MD)]);
    }

    #[test]
    fn a_home_its_computer_no_longer_has_is_a_failed_read_not_an_empty_one() {
        use glave_computer::LocalComputers;
        use glave_ports::computer::{Computers as _, Environment, Sources};
        use std::sync::Arc;

        let root = folder("gone-home");
        let read = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                let computers = LocalComputers::new(root.join("copies"), std::env::vars_os());
                let computer = computers.create(&Environment::host()).await.unwrap();
                let sources = Sources::Folder {
                    path: root.to_string_lossy().into_owned(),
                };
                let info = computers.open(computer.id, &sources).await.unwrap();
                computers.close(info.id).await.unwrap();
                let home = Workspace::new(Arc::new(computers), info);
                personal_on(&home).await
            });
        assert!(read.is_err(), "{read:?}");
    }

    #[test]
    fn missing_files_mean_no_instructions() {
        let root = folder("none");
        assert_eq!(load(&root, Some(&root.join("nope.md"))), None);
    }

    #[test]
    fn the_same_file_is_sent_once() {
        let root = folder("same");
        std::fs::write(root.join(AGENTS_MD), "Once.").unwrap();
        let loaded = load(&root, Some(&root.join(AGENTS_MD))).unwrap();
        assert_eq!(loaded.text.matches("Once.").count(), 1);
    }

    #[test]
    fn instructions_stop_at_32_kib() {
        let root = folder("long");
        std::fs::write(root.join(AGENTS_MD), "é".repeat(20_000)).unwrap();
        let text = load(&root, None).unwrap().text;
        assert!(text.contains("[cut short: instructions are limited to 32 KiB]"));
        assert!(text.len() < MAX_BYTES + 512, "{}", text.len());
        assert_eq!(within("héllo", 2), ("h", true), "never splits a character");
        assert_eq!(within("hello", 9), ("hello", false));
    }
}
