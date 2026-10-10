//! Asking the person before acting: how much to ask (the approval mode), and always before the
//! model sees a file that usually holds secrets. See `specs/approvals.md`.

use std::future::{Future, ready};
use std::path::Path;

use glave_event::{Arguments, CallId, PrincipalId, ThreadId};
use glave_ports::Effect;
use serde::Serialize;

use crate::question::QuestionRequest;
use crate::secrets::is_secret_file;
use crate::tools::Tool;
use crate::tools::workspace::Workspace;

/// What the harness wants to do, in words the person can judge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    /// The thread asking; with `call_id`, enough for anyone to answer it later.
    pub thread: ThreadId,
    /// The call waiting, which is also the approval's id.
    pub call_id: CallId,
    /// The tool's name.
    pub tool: String,
    /// What the tool does.
    pub effect: Effect,
    pub summary: String,
    /// The model would see a file that usually holds secrets, such as `.env`. With no one to
    /// ask, these requests are declined.
    pub secret: bool,
    /// Why the call needs the person whatever the mode, when it does: "it deletes ~/x".
    pub caution: Option<String>,
}

/// How much glave asks before acting. Secret files ask in every mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalMode {
    /// The cautious mode: ask before every change and every command.
    Ask,
    /// Changes and commands go through: in the sandbox when it is on, as the person when not.
    #[default]
    Auto,
}

impl ApprovalMode {
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "ask" => Some(Self::Ask),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Auto => "auto",
        }
    }

    /// Whether a tool with this effect needs the person's approval in this mode.
    pub(crate) fn asks_for(self, effect: Effect) -> bool {
        match effect {
            Effect::Reads => false,
            Effect::Changes | Effect::Runs => self == Self::Ask,
        }
    }
}

/// The person's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    AllowOnce,
    /// Allow this tool for the rest of the thread.
    AllowForThread,
    Deny,
    /// No one is there to answer: glave runs without anyone at the keyboard.
    Unavailable,
}

/// Decides whether an action may go ahead. The CLI asks in the terminal, or answers that no
/// one is there when it has none.
pub trait Approver: Send + Sync {
    fn decide(&self, request: &ApprovalRequest) -> impl Future<Output = Decision> + Send;

    /// Decides, and names the person who did when the approver knows: someone other than the
    /// thread's `for` is recorded as the decision's `by`. Plain [`Approver::decide`] by default.
    fn decide_by(
        &self,
        request: &ApprovalRequest,
    ) -> impl Future<Output = (Decision, Option<PrincipalId>)> + Send {
        let decided = self.decide(request);
        async move { (decided.await, None) }
    }

    /// Whether someone can answer questions; `question` is offered only then.
    fn can_answer(&self) -> bool {
        false
    }

    /// Puts `request`'s questions to the person: one answer for each, or `None` when no one
    /// answered. The question is waiting once this returns, before the future is awaited.
    fn answer(
        &self,
        request: &QuestionRequest,
    ) -> impl Future<Output = Option<Vec<String>>> + Send {
        let _ = request;
        ready(None)
    }
}

/// For runs with no one at the keyboard: every request is answered "no one is here".
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOneToAsk;

impl Approver for NoOneToAsk {
    fn decide(&self, _request: &ApprovalRequest) -> impl Future<Output = Decision> + Send {
        ready(Decision::Unavailable)
    }
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

/// What the model is told when an action needs a person and no one is at the keyboard: not to
/// retry or work around it, and to say what is left for a person.
pub(crate) fn no_one_there(request: &ApprovalRequest) -> String {
    if request.secret {
        "No one is available to approve reading this file, which usually holds secrets: glave is \
         running without anyone at the keyboard. Do not try to get its contents another way. \
         Finish what you can without it, and say at the end which value a person must provide."
            .to_owned()
    } else if let Some(reason) = &request.caution {
        format!(
            "Not run, because {reason}, and no one is available to approve that: glave is \
             running without anyone at the keyboard. Do not try to get the same result another \
             way. Finish what you can without it, and say at the end what still needs a person."
        )
    } else {
        "No one is available to approve this action: glave is running without anyone at the \
         keyboard, in a mode that needs approval for it. Do not retry it or do the same thing \
         another way. Finish what you can without it, and say at the end what still needs a \
         person (or that it can be run with --approval auto)."
            .to_owned()
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
    fn each_mode_asks_for_exactly_what_it_should() {
        use ApprovalMode::{Ask, Auto};
        assert!(!Ask.asks_for(Effect::Reads) && !Auto.asks_for(Effect::Reads));
        assert!(Ask.asks_for(Effect::Changes) && !Auto.asks_for(Effect::Changes));
        assert!(Ask.asks_for(Effect::Runs) && !Auto.asks_for(Effect::Runs));
        assert_eq!(ApprovalMode::default(), Auto);
        assert_eq!(ApprovalMode::named("edits"), None, "retired");
        for mode in [Ask, Auto] {
            assert_eq!(ApprovalMode::named(mode.name()), Some(mode));
        }
    }

    #[test]
    fn with_no_one_to_ask_secrets_are_declined() {
        let request = |secret| ApprovalRequest {
            thread: ThreadId::new(jiff::Timestamp::UNIX_EPOCH, [0; 10]).unwrap(),
            call_id: CallId::new("call_1").unwrap(),
            tool: "read".into(),
            effect: Effect::Reads,
            summary: "read .env".into(),
            secret,
            caution: None,
        };
        assert_eq!(block_on(AllowAll.decide(&request(true))), Decision::Deny);
        assert_eq!(
            block_on(AllowAll.decide(&request(false))),
            Decision::AllowOnce
        );
    }
}
