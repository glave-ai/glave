//! Files a call made for the person (`specs/session.md`, "Files it made"). A file counts as made
//! when a `write` or a command named it, it is a document, a spreadsheet, a deck, a PDF or an
//! e-book by the end of its name (`glave_event::FileKind::made_as`), it is in the thread's
//! folder and in no hidden folder there, and the call left it different from what was there
//! before. Its bytes are read through the computer port as the call ends and kept for the
//! thread, sealed, so it outlives the computer; the model never sees them.

use glave_event::{Arguments, FileKind, FileMade};
use glave_ports::computer::{EntryKind, Stat};
use glave_ports::{BlobStore, EventLog, ModelProvider, Pieces, ToolFile, ToolOutput};

use crate::approval::Approver;
use crate::files::MAX_FILE;
use crate::harness::{Harness, Thread};
use crate::tools::Tool;
use crate::tools::workspace::Workspace;

/// The most files one call makes for the person: past them, the rest are left on the computer.
const MOST: usize = 8;

/// The paths a call of `tool` may make a file at: the one a `write` names, and each word of a
/// command that names a file that would count as made (`glave doc write budget.xlsx`,
/// `> table.csv`). None for any other tool.
pub(crate) fn named(tool: Tool, arguments: &Arguments) -> Vec<String> {
    let Arguments::Json(map) = arguments else {
        return Vec::new();
    };
    let text = |key: &str| map.get(key).and_then(|value| value.as_str());
    let mut paths: Vec<String> = match tool {
        Tool::Write => text("path").map(str::to_owned).into_iter().collect(),
        Tool::Bash => text("command").map(words).unwrap_or_default(),
        _ => Vec::new(),
    };
    paths.retain(|path| FileKind::made_as(path).is_some());
    paths.dedup();
    paths.truncate(MOST);
    paths
}

/// The words of `command`, as a shell splits them, near enough: quotes keep a name with spaces
/// whole, and an option's value after `=` is a word of its own (`--out=a.pdf`).
fn words(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quote: Option<char> = None;
    for c in command.chars() {
        match quote {
            Some(open) if c == open => quote = None,
            None if c == '\'' || c == '"' => quote = Some(c),
            None if c.is_whitespace() || ";|&<>()=`".contains(c) => {
                words.push(std::mem::take(&mut word));
            }
            Some(_) | None => word.push(c),
        }
    }
    words.push(word);
    words.retain(|word| !word.is_empty());
    words
}

/// What is at each of `paths` before the call, by the path the model gave: a path the computer
/// refuses (outside the thread's folder) is left out, since nothing made there counts.
pub(crate) async fn before(workspace: &Workspace, paths: &[String]) -> Vec<(String, Stat)> {
    let mut found = Vec::with_capacity(paths.len());
    for path in paths {
        if let Ok(stat) = workspace.stat(path).await {
            found.push((path.clone(), stat));
        }
    }
    found
}

/// The files the call made at what was `before`: each one now a file, no larger than a thread
/// keeps, in no hidden folder, and different from what was there, read whole.
pub(crate) async fn made(workspace: &Workspace, before: Vec<(String, Stat)>) -> Vec<ToolFile> {
    let mut made: Vec<ToolFile> = Vec::new();
    for (path, was) in before {
        let Ok(now) = workspace.stat(&path).await else {
            continue;
        };
        let changed = now.kind != was.kind || now.size != was.size || now.digest != was.digest;
        let fits = now.kind == Some(EntryKind::File) && now.size <= MAX_FILE;
        let again = made.iter().any(|file| file.name == now.path);
        if !changed || !fits || again || hidden(&now.path) {
            continue;
        }
        let Some(media_type) = FileKind::made_as(&now.path) else {
            continue;
        };
        if let Ok(bytes) = workspace.read(&now.path).await {
            made.push(ToolFile {
                name: now.path,
                media_type: media_type.to_owned(),
                bytes: bytes.into_vec(),
            });
        }
    }
    made
}

/// Whether `path`, relative to the thread's folder, is in a hidden folder or is hidden itself:
/// scratch, such as glave's own folder or a cache, never something made for the person.
fn hidden(path: &str) -> bool {
    path.split('/')
        .any(|name| name.starts_with('.') && name != ".")
}

impl<P, L, B, A> Harness<P, L, B, A>
where
    P: ModelProvider,
    L: EventLog,
    B: BlobStore,
    A: Approver,
{
    /// `output` with the files it made kept for `thread` and named in its `structured` result;
    /// one that cannot be kept stays on the computer alone.
    pub(crate) async fn keep_made_files(
        &self,
        thread: &Thread,
        mut output: ToolOutput,
    ) -> ToolOutput {
        let mut kept = Vec::new();
        for file in std::mem::take(&mut output.made) {
            let put = self.blobs.put(
                &thread.scope,
                Some(thread.id),
                &file.media_type,
                MAX_FILE,
                Box::new(Pieces::from(file.bytes)),
            );
            match put.await {
                Ok(blob) => kept.push(FileMade {
                    path: file.name,
                    blob,
                }),
                Err(error) => tracing::warn!(%error, "a file the call made could not be kept"),
            }
        }
        output.structured = FileMade::structured(output.structured.take(), &kept);
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bash(command: &str) -> Vec<String> {
        let arguments = Arguments::Json(
            serde_json::json!({ "command": command })
                .as_object()
                .cloned()
                .unwrap_or_default(),
        );
        named(Tool::Bash, &arguments)
    }

    #[test]
    fn a_command_names_the_files_it_may_make() {
        assert_eq!(
            bash("glave doc write budget.xlsx <<'EOF'\n{}\nEOF"),
            ["budget.xlsx"]
        );
        assert_eq!(
            bash("cd out && glave doc pack work \"Trip plan.docx\" --replace"),
            ["Trip plan.docx"]
        );
        assert_eq!(
            bash("python3 make.py --out=deck.pptx>log.txt"),
            ["deck.pptx"]
        );
        assert_eq!(
            bash("glave doc merge all.pdf a.pdf b.pdf"),
            ["all.pdf", "a.pdf", "b.pdf"],
            "each is stat'd before and after; only what changed counts"
        );
        assert_eq!(bash("cargo test && cat notes.md"), Vec::<String>::new());
    }

    #[test]
    fn a_write_names_its_path_when_it_would_count() {
        let write = |path: &str| {
            let arguments = serde_json::json!({ "path": path, "content": "a,b" });
            named(
                Tool::Write,
                &Arguments::Json(arguments.as_object().cloned().unwrap_or_default()),
            )
        };
        assert_eq!(write("table.csv"), ["table.csv"]);
        assert_eq!(write("src/main.rs"), Vec::<String>::new());
        let read = Arguments::Json(
            serde_json::json!({ "path": "a.pdf" })
                .as_object()
                .cloned()
                .unwrap_or_default(),
        );
        assert_eq!(named(Tool::Read, &read), Vec::<String>::new());
    }

    #[test]
    fn what_is_hidden_is_scratch() {
        assert!(hidden(".glave/out.pdf") && hidden("work/.cache/a.xlsx") && hidden(".a.pdf"));
        assert!(!hidden("out/report.pdf") && !hidden("report.pdf"));
    }
}
