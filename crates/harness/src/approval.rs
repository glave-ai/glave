//! Asking the person before acting: how much to ask (what a thread's mode comes to), before
//! the model sees a file that usually holds secrets, and before a connected service's write
//! after untrusted content. See `specs/approvals.md`.

mod act;

pub use self::act::acts_after_untrusted;

use std::future::{Future, ready};
use std::path::Path;

use glave_event::{
    ApprovalRequested, Arguments, Ask, CallId, Caution, Lifted, OutsideSandbox, PrincipalId, Risk,
    Settled, ThreadId, WebPage, Worded as _,
};
use glave_ports::{Assessment, Effect, ToolSource};

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
    /// What would run, whole, as the person and the judge see it: `bash: cargo test`, or a
    /// service's tool with every key of its arguments. Never why it asks.
    pub summary: String,
    /// What it asks the person, by what would run: "Run this command?".
    pub ask: Ask,
    /// The model would see a secret: a file that usually holds one, such as `.env`, or what a
    /// command prints. With no one to ask, these requests are declined.
    pub secret: bool,
    /// Why the call needs the person in any mode that asks, when it does: the model and the
    /// judge are told it as a clause ("it deletes ~/x", [`Caution::because`]), and the person
    /// reads it as a sentence.
    pub caution: Option<Caution>,
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
            ask: self.ask.english(),
            asking: Some(self.ask.clone()),
            detail: detail_of(&self.tool, &self.summary).to_owned(),
            secret: self.secret,
            caution: self.caution.as_ref().and_then(Caution::sentences),
            reason: self.caution.clone(),
            outside_sandbox: self.outside_sandbox.clone(),
        }
    }

    /// The record of this request, which glave settled `how` without asking anyone: why it
    /// asks, then how it was settled.
    pub(crate) fn settled(&self, how: Settled) -> ApprovalRequested {
        let reason = Caution {
            settled: Some(how),
            ..self.caution.clone().unwrap_or_default()
        };
        ApprovalRequested {
            caution: reason.sentences(),
            reason: Some(reason),
            ..self.asked()
        }
    }
}

/// What `source` says of a call of `tool` with `arguments`, whose tool declares `declared`: its
/// effect, the caution it needs the person for, whether it would show the model a secret, and
/// whether no judge may clear it.
pub(crate) async fn assessed(
    source: &dyn ToolSource,
    tool: &str,
    arguments: &Arguments,
    declared: Effect,
) -> (Effect, Option<Risk>, bool, bool) {
    match source.assess(tool, arguments).await {
        Assessment::AsDeclared => (declared, None, false, false),
        Assessment::Reads => (Effect::Reads, None, false, false),
        Assessment::Caution {
            reason,
            secret,
            firm,
        } => (declared, Some(reason), secret, firm),
    }
}

/// What an approval of a call of `tool` from `source` asks the person, and, for a connected
/// service's tool that does not say it only reads (`effect`), why it needs them once the thread
/// has read untrusted content (`specs/approvals.md`, "Untrusted content").
pub(crate) fn asking(
    source: &dyn ToolSource,
    tool: &str,
    effect: Effect,
    untrusted: bool,
) -> (Ask, Option<Risk>) {
    if !source.connected() {
        return (ask_of(tool), None);
    }
    let naming = source.naming(tool);
    let acts = (untrusted && effect != Effect::Reads).then(|| acts_after_untrusted(tool, &naming));
    (act::ask_with(tool, &naming), acts)
}

/// What an approval of `tool`, which is no connected service's, asks: by what the tool does,
/// for the harness's own tools and `web_fetch`, else as any tool.
pub(crate) fn ask_of(tool: &str) -> Ask {
    match Tool::named(tool) {
        Some(Tool::Bash) => Ask::Command,
        Some(Tool::Edit) => Ask::Edit,
        Some(Tool::Write) => Ask::Write,
        Some(Tool::Read) => Ask::Read,
        _ if tool == WebPage::TOOL => Ask::Page,
        _ => Ask::Allow,
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
    /// Nothing asks, risky actions and secrets included (Full access).
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

    /// Whether someone can answer approvals and questions in `thread` now: a pending approval
    /// is announced, and `question` runs, only then; it is listed where someone could at the
    /// thread's first call (`prefix.rs`).
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
    let because = request.caution.as_ref().and_then(Caution::because);
    if request.secret {
        "No one is available to approve this, which would show you secrets such as keys, tokens \
         or passwords: glave is running without anyone at the keyboard. Do not try to get them \
         another way. Finish what you can without it, and say at the end which value a person \
         must provide."
            .to_owned()
    } else if let Some(reason) = because.as_ref().filter(|_| outside) {
        format!(
            "Not run, because {reason}, and no one is available to allow that: glave is running \
             without anyone at the keyboard. Do not get the same result another way, such as by \
             pointing the command at folders inside the project. Finish what you can in the \
             sandbox, and say at the end which command still needs a person to allow it."
        )
    } else if let Some(reason) = because.as_ref().filter(|_| lifted == Some(Lifted::Network)) {
        format!(
            "Not run, because {reason}, and no one is available to allow that: glave is running \
             without anyone at the keyboard. Do not try to reach the internet another way, \
             and do not stand in for what it would fetch, such as by writing your own \
             replacement for a package. Finish what does not depend on it, and say at the end \
             which command still needs a person to allow it."
        )
    } else if let Some(reason) = &because {
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
        fn asked(tool: &str, summary: &str) -> String {
            format!("{} | {}", ask_of(tool).english(), detail_of(tool, summary))
        }
        assert_eq!(
            asked("bash", "bash: git push -u origin fix/x"),
            "Run this command? | git push -u origin fix/x"
        );
        assert_eq!(
            asked("edit", "edit src/main.rs"),
            "Edit this file? | src/main.rs"
        );
        assert_eq!(
            asked("write", "write notes.md (3 lines)"),
            "Write this file? | notes.md (3 lines)"
        );
        assert_eq!(asked("read", "read .env"), "Read this file? | .env");
        assert_eq!(
            asked("web_fetch", "web_fetch https://tokio.rs"),
            "Open this page? | https://tokio.rs"
        );
        assert_eq!(
            asked("remember", r#"remember {"text":"x"}"#),
            r#"Allow this? | {"text":"x"}"#
        );
        assert_eq!(
            asked("search", "Searched the web: tokio"),
            "Allow this? | Searched the web: tokio",
            "a summary that does not start with its tool's name is all detail"
        );
        assert_eq!(detail_of("bashful", "bash: ls"), "bash: ls");
        assert_eq!(detail_of("", "x"), "x");
    }

    #[test]
    fn only_a_connected_service_s_write_after_untrusted_content_is_a_caution() {
        let tool = "mcp__gmail__send_message";
        let connected = glave_ports::InMemoryTools::new(Vec::new()).as_connected();
        let own = glave_ports::InMemoryTools::new(Vec::new());
        let gmail = Risk::ActsAfterUntrusted {
            service: "gmail".into(),
        };
        let send = Ask::With {
            act: "Send message".into(),
            service: "gmail".into(),
        };
        let after = asking(&connected, tool, Effect::Runs, true);
        assert_eq!(after, (send.clone(), Some(gmail)));
        let read = asking(&connected, tool, Effect::Reads, true);
        assert_eq!(read, (send.clone(), None), "a read");
        let before = asking(&connected, tool, Effect::Runs, false);
        assert_eq!(before, (send, None), "before untrusted content");
        let unconnected = asking(&own, tool, Effect::Runs, true);
        assert_eq!(
            unconnected,
            (Ask::Allow, None),
            "glave's own, or the web search"
        );
    }

    #[test]
    fn with_no_one_to_ask_secrets_are_declined() {
        let request = |secret| ApprovalRequest {
            thread: ThreadId::new(jiff::Timestamp::UNIX_EPOCH, [0; 10]).unwrap(),
            call_id: CallId::new("call_1").unwrap(),
            tool: "read".into(),
            effect: Effect::Reads,
            summary: "read .env".into(),
            ask: Ask::Read,
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
