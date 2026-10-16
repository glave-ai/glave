//! AGENTS.md: the instructions a project gives the agents that work in it. They are read when a
//! thread starts and sent as the first part of its first message, the way Codex and Claude Code
//! send theirs. They ride in a user message, not the system prompt: they come from the
//! repository, so they steer the model without outranking glave's own instructions, and
//! approvals still gate every action.

use std::path::{Path, PathBuf};

use glave_ports::BoxFuture;

/// What a thread working in a folder of its own opens with, as the host reads it: the
/// person's own instructions, the folder's AGENTS.md, and what its project brings that stays
/// off and why. Each thread reads its own, so a thread in one project never opens with
/// another's. Without one, every thread opens with the settings' instructions.
pub trait FolderInstructions: Send + Sync {
    /// The instructions for a thread working at `root`, in the project whose folder is
    /// `project`; `None` for `project` when the folder is a scratch folder of the thread's
    /// own.
    fn for_folder<'a>(
        &'a self,
        root: &'a Path,
        project: Option<&'a Path>,
    ) -> BoxFuture<'a, Option<String>>;
}

/// The file agents look for.
pub const AGENTS_MD: &str = "AGENTS.md";

/// The most instruction text sent, as Codex caps it.
const MAX_BYTES: usize = 32 * 1024;

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
    let mut files = Vec::new();
    let mut sections = Vec::new();
    let mut remaining = MAX_BYTES;
    for path in candidates {
        if remaining == 0 {
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
        if text.trim().is_empty() || files.contains(&canonical) {
            continue;
        }
        let (text, cut) = within(text.trim(), remaining);
        remaining = remaining.saturating_sub(text.len());
        let note = if cut {
            "\n[cut short: instructions are limited to 32 KiB]"
        } else {
            ""
        };
        sections.push(format!("From {}:\n{text}{note}", path.display()));
        files.push(canonical);
    }
    if sections.is_empty() {
        return Ok(None);
    }
    Ok(Some(Instructions {
        text: format!(
            "# AGENTS.md instructions for {}\n\n<INSTRUCTIONS>\n{}\n</INSTRUCTIONS>",
            root.display(),
            sections.join("\n\n")
        ),
        files,
    }))
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

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("glave-agents-md-{}", std::process::id()))
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(dir).unwrap()
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
        let mine = folder("home").join(AGENTS_MD);
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
