//! Asking the person before acting: how much to ask (what a thread's mode comes to), and
//! before the model sees a file that usually holds secrets, which Full access, never asking,
//! declines instead. See `specs/approvals.md`.

use std::future::{Future, ready};
use std::path::Path;

use glave_event::{
    ApprovalRequested, Arguments, CallId, Lifted, OutsideSandbox, PrincipalId, ThreadId, WebPage,
};
use glave_ports::Effect;

use crate::question::QuestionRequest;
use crate::tools::Tool;
use crate::tools::workspace::Workspace;
use glave_ports::secrets::is_secret_file;

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
    /// What would run: `bash: cargo test`. Never why it asks.
    pub summary: String,
    /// The model would see a secret: a file that usually holds one, such as `.env`, or what a
    /// command prints. With no one to ask, and in Full access, which asks no one, these requests
    /// are declined.
    pub secret: bool,
    /// Why the call needs the person in any mode that asks, when it does, as the model and the
    /// judge are told it: "it deletes ~/x". The person reads it as a sentence (`sentence`).
    pub caution: Option<String>,
    /// The command would run outside the sandbox, or in it with the network, as the model
    /// asked, with its reason. Such a request asks every time: allowing it for the thread
    /// allows it once.
    pub outside_sandbox: Option<OutsideSandbox>,
}

impl ApprovalRequest {
    /// The record of this request: what would run, and apart from it, why it asks.
    pub(crate) fn asked(&self) -> ApprovalRequested {
        ApprovalRequested {
            call_id: self.call_id.clone(),
            tool: self.tool.clone(),
            summary: self.summary.clone(),
            ask: ask_of(&self.tool).to_owned(),
            detail: detail_of(&self.tool, &self.summary).to_owned(),
            secret: self.secret,
            caution: self.caution.as_deref().map(sentence),
            outside_sandbox: self.outside_sandbox.clone(),
        }
    }
}

/// Every sentence an approval asks with, in English, as `approval.requested` records it: for a
/// command, an edit, a write, a read, a page, and any other tool. A transport writes each again
/// in the caller's language, finding it in the session's table by these words.
pub const ASKS: [&str; 6] = [
    "Run this command?",
    "Edit this file?",
    "Write this file?",
    "Read this file?",
    "Open this page?",
    "Allow this?",
];

/// What an approval for `tool` asks the person, in a sentence: by what the tool does, for the
/// harness's own tools and `web_fetch`, else a sentence for any tool.
fn ask_of(tool: &str) -> &'static str {
    let [command, edit, write, read, page, any] = ASKS;
    match Tool::named(tool) {
        Some(Tool::Bash) => command,
        Some(Tool::Edit) => edit,
        Some(Tool::Write) => write,
        Some(Tool::Read) => read,
        _ if tool == WebPage::TOOL => page,
        _ => any,
    }
}

/// What would run, apart from the sentence that says what it is: `summary`, which every tool
/// source writes as the tool's name then what it acts on (`bash: cargo test`, `edit
/// src/main.rs`), without that name. A summary that does not start with it is all detail.
fn detail_of<'a>(tool: &str, summary: &'a str) -> &'a str {
    summary
        .strip_prefix(tool)
        .filter(|_| !tool.is_empty())
        .and_then(|rest| rest.strip_prefix(": ").or_else(|| rest.strip_prefix(' ')))
        .unwrap_or(summary)
}

/// A caution's reason as the person reads it, a sentence of its own: "it deletes ~/x" is
/// "It deletes ~/x.".
pub(crate) fn sentence(reason: &str) -> String {
    let reason = reason.trim();
    let mut chars = reason.chars();
    let mut sentence: String = chars
        .next()
        .map(char::to_uppercase)
        .into_iter()
        .flatten()
        .chain(chars)
        .collect();
    if !sentence.is_empty() && !sentence.ends_with(['.', '!', '?']) {
        sentence.push('.');
    }
    sentence
}

/// How much asks before acting, as a thread's mode comes to it where its commands run
/// (`specs/approvals.md`, "Modes"): it is never chosen itself, only the mode is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApprovalMode {
    /// Every change and command asks first.
    Everything,
    /// Ask first in its sandbox: commands go through, since the sandbox keeps them from
    /// changing the project, and every change and every other tool that acts asks first.
    Changes,
    /// Changes and commands go through; only risky ones ask, or go to the judge.
    Risky,
    /// Nothing asks, risky actions included (Full access): what would show the model a secret
    /// is declined at once instead.
    Never,
}

/// Whether a tool with `effect` needs the person's approval in `mode`; `confined` when it is
/// a command that runs in the thread's sandbox.
pub(crate) fn asks_for(mode: ApprovalMode, effect: Effect, confined: bool) -> bool {
    match (effect, mode) {
        (Effect::Reads, _) | (_, ApprovalMode::Risky | ApprovalMode::Never) => false,
        (Effect::Runs, ApprovalMode::Changes) => !confined,
        (Effect::Changes, ApprovalMode::Changes) | (_, ApprovalMode::Everything) => true,
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
    /// When someone can answer, the approval is pending once this returns, before the future
    /// is awaited.
    fn decide_by(
        &self,
        request: &ApprovalRequest,
    ) -> impl Future<Output = (Decision, Option<PrincipalId>)> + Send {
        let decided = self.decide(request);
        async move { (decided.await, None) }
    }

    /// Whether someone can answer approvals and questions in `thread`: `question` is offered,
    /// and a pending approval announced, only then.
    fn can_answer(&self, thread: ThreadId) -> bool {
        let _ = thread;
        false
    }

    /// Whether the person answers `thread`'s approvals and questions in the log, at any time,
    /// from any client: then a turn stops at what it asks, holding nothing, and goes on from the
    /// log once the answer is recorded ([`crate::Harness::go_on`]), rather than waiting for
    /// [`Approver::decide_by`] or [`Approver::answer`].
    fn waits(&self, thread: ThreadId) -> bool {
        let _ = thread;
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
    let lifted = request
        .outside_sandbox
        .as_ref()
        .map(|outside| outside.lifted);
    let outside = lifted == Some(Lifted::Sandbox);
    if request.secret {
        "No one is available to approve this, which would show you secrets such as keys, tokens \
         or passwords: glave is running without anyone at the keyboard. Do not try to get them \
         another way. Finish what you can without it, and say at the end which value a person \
         must provide."
            .to_owned()
    } else if let Some(reason) = request.caution.as_ref().filter(|_| outside) {
        format!(
            "Not run, because {reason}, and no one is available to allow that: glave is running \
             without anyone at the keyboard. Do not get the same result another way, such as by \
             pointing the command at folders inside the project. Finish what you can in the \
             sandbox, and say at the end which command still needs a person to allow it."
        )
    } else if let Some(reason) = request
        .caution
        .as_ref()
        .filter(|_| lifted == Some(Lifted::Network))
    {
        format!(
            "Not run, because {reason}, and no one is available to allow that: glave is running \
             without anyone at the keyboard. Do not try to reach the internet another way, \
             and do not stand in for what it would fetch, such as by writing your own \
             replacement for a package. Finish what does not depend on it, and say at the end \
             which command still needs a person to allow it."
        )
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
         person (or that it can run once the person chooses wider permissions)."
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
            .stat(&path)
            .await
            .is_ok_and(|resolved| is_secret_file(Path::new(&resolved.path)))
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
        Tool::CommandOutput => match crate::tools::command_named(arguments) {
            Some(number) => format!("command_output {number}"),
            None => "command_output".to_owned(),
        },
    }
}

#[cfg(test)]
mod tests {

    use glave_ports::block_on;

    use super::*;

    fn read(path: &str) -> Arguments {
        Arguments::Json(serde_json::from_value(serde_json::json!({ "path": path })).unwrap())
    }

    #[test]
    fn secret_files_need_approval_even_behind_a_symlink() {
        let dir = glave_ports::testing::TempDir::new("secret-read").unwrap();
        std::fs::write(dir.join(".env"), "TOKEN=x\n").unwrap();
        std::fs::write(dir.join("README.md"), "hi\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join(".env"), dir.join("notes.txt")).unwrap();
        block_on!(async {
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
        use ApprovalMode::{Changes, Everything, Never, Risky};
        for confined in [false, true] {
            for mode in [Everything, Changes, Risky, Never] {
                assert!(!asks_for(mode, Effect::Reads, confined), "reads never ask");
            }
            assert!(asks_for(Everything, Effect::Changes, confined));
            assert!(asks_for(Everything, Effect::Runs, confined));
            assert!(
                asks_for(Changes, Effect::Changes, confined),
                "changes ask in Ask first"
            );
            for mode in [Risky, Never] {
                assert!(!asks_for(mode, Effect::Changes, confined));
                assert!(!asks_for(mode, Effect::Runs, confined));
            }
        }
        assert!(
            !asks_for(Changes, Effect::Runs, true),
            "a command in the sandbox goes"
        );
        assert!(
            asks_for(Changes, Effect::Runs, false),
            "any other tool that acts asks"
        );
    }

    #[test]
    fn an_approval_asks_by_what_would_run_with_what_would_run_apart() {
        fn asked<'a>(tool: &str, summary: &'a str) -> (&'static str, &'a str) {
            (ask_of(tool), detail_of(tool, summary))
        }
        assert_eq!(
            asked("bash", "bash: git push -u origin fix/x"),
            ("Run this command?", "git push -u origin fix/x")
        );
        assert_eq!(
            asked("edit", "edit src/main.rs"),
            ("Edit this file?", "src/main.rs")
        );
        assert_eq!(
            asked("write", "write notes.md (3 lines)"),
            ("Write this file?", "notes.md (3 lines)")
        );
        assert_eq!(asked("read", "read .env"), ("Read this file?", ".env"));
        assert_eq!(
            asked("web_fetch", "web_fetch https://tokio.rs"),
            ("Open this page?", "https://tokio.rs")
        );
        assert_eq!(
            asked("remember", r#"remember {"text":"x"}"#),
            ("Allow this?", r#"{"text":"x"}"#)
        );
        assert_eq!(
            asked("search", "Searched the web: tokio"),
            ("Allow this?", "Searched the web: tokio"),
            "a summary that does not start with its tool's name is all detail"
        );
        assert_eq!(asked("bashful", "bash: ls").1, "bash: ls");
        assert_eq!(asked("", "x").1, "x");
    }

    #[test]
    fn a_caution_reads_as_a_sentence_of_its_own() {
        assert_eq!(
            sentence("it deletes /home/ana/notes, outside the project"),
            "It deletes /home/ana/notes, outside the project."
        );
        assert_eq!(
            sentence("this file usually holds secrets; the model would see them"),
            "This file usually holds secrets; the model would see them."
        );
        assert_eq!(sentence(" Already one. "), "Already one.");
        assert_eq!(sentence(""), "");
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
            outside_sandbox: None,
        };
        assert_eq!(block_on!(AllowAll.decide(&request(true))), Decision::Deny);
        assert_eq!(
            block_on!(AllowAll.decide(&request(false))),
            Decision::AllowOnce
        );
    }
}
