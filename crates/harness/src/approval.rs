//! Asking the person before anything changes (writes, edits and commands) and before the
//! model sees a file that usually holds secrets.

use std::future::{Future, ready};
use std::path::Path;

use glave_event::Arguments;

use crate::secrets::is_secret_file;
use crate::tools::Tool;
use crate::tools::workspace::Workspace;

/// What the harness wants to do, in words the person can judge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub tool: Tool,
    pub summary: String,
    /// The model would see a file that usually holds secrets, such as `.env`. With no one to
    /// ask, these requests are declined.
    pub secret: bool,
}

/// The person's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    AllowOnce,
    /// Allow this tool for the rest of the thread.
    AllowForThread,
    Deny,
}

/// Decides whether an action may go ahead. The CLI asks in the terminal; tests and `--yes`
/// allow everything.
pub trait Approver: Send + Sync {
    fn decide(&self, request: &ApprovalRequest) -> impl Future<Output = Decision> + Send;
}

/// Allows every action without asking, except showing the model a secret file: with no one
/// to ask, that is declined.
#[derive(Debug, Clone, Copy, Default)]
pub struct AllowAll;

impl Approver for AllowAll {
    fn decide(&self, request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        ready(if request.secret {
            Decision::Deny
        } else {
            Decision::AllowOnce
        })
    }
}

/// Whether this call would show the model a secret file, judged by the path given and by the
/// file it leads to, so a symlink with an innocent name is caught too.
pub(crate) async fn reads_secret(workspace: &Workspace, tool: Tool, arguments: &Arguments) -> bool {
    if tool != Tool::Read {
        return false;
    }
    let Some(path) = field(arguments, "path") else {
        return false;
    };
    is_secret_file(Path::new(&path))
        || workspace
            .resolve(&path)
            .await
            .is_ok_and(|resolved| is_secret_file(&resolved))
}

fn field(arguments: &Arguments, name: &str) -> Option<String> {
    match arguments {
        Arguments::Json(map) => map.get(name).and_then(|v| v.as_str()).map(str::to_owned),
        Arguments::Malformed(_) => None,
    }
}

/// A one-line description of a tool call, such as `edit src/main.rs` or `bash: cargo test`.
pub(crate) fn summarize(tool: Tool, arguments: &Arguments) -> String {
    let field = |name: &str| field(arguments, name);
    let path = field("path").unwrap_or_else(|| "?".into());
    match tool {
        Tool::Search => format!("search {}", field("pattern").unwrap_or_else(|| "?".into())),
        Tool::Glob => format!("glob {}", field("pattern").unwrap_or_else(|| "?".into())),
        Tool::Read => format!("read {path}"),
        Tool::Write => {
            let lines = field("content").map_or(0, |content| content.lines().count());
            format!("write {path} ({lines} lines)")
        }
        Tool::Edit => format!("edit {path}"),
        Tool::Bash => format!("bash: {}", field("command").unwrap_or_else(|| "?".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn read(path: &str) -> Arguments {
        Arguments::Json(serde_json::from_value(serde_json::json!({ "path": path })).unwrap())
    }

    #[test]
    fn secret_files_need_approval_even_behind_a_symlink() {
        let dir = std::env::temp_dir().join(format!("glave-secret-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".env"), "TOKEN=x\n").unwrap();
        std::fs::write(dir.join("README.md"), "hi\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join(".env"), dir.join("notes.txt")).unwrap();
        block_on(async {
            let workspace = Workspace::open(&dir, []).await.unwrap();
            assert!(reads_secret(&workspace, Tool::Read, &read(".env")).await);
            assert!(!reads_secret(&workspace, Tool::Read, &read("README.md")).await);
            #[cfg(unix)]
            assert!(reads_secret(&workspace, Tool::Read, &read("notes.txt")).await);
            assert!(!reads_secret(&workspace, Tool::Bash, &read(".env")).await);
        });
    }

    #[test]
    fn with_no_one_to_ask_secrets_are_declined() {
        let request = |secret| ApprovalRequest {
            tool: Tool::Read,
            summary: "read .env".into(),
            secret,
        };
        assert_eq!(block_on(AllowAll.decide(&request(true))), Decision::Deny);
        assert_eq!(
            block_on(AllowAll.decide(&request(false))),
            Decision::AllowOnce
        );
    }
}
