//! The CLI as a client of the session API: following a turn as it runs, answering approvals in
//! the terminal, compacting, and showing what failed with what to fix. It uses nothing a
//! remote client could not.

pub(crate) mod plan;

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

use glave_event::{
    ApprovalRequested, Body, CallId, GoalReason, Outcome, PrincipalKind, ThreadId, decode,
};
use glave_harness::{Cancel, Goal, Permissions, Question};
use glave_ports::Caller;
use glave_session::client::{Remote, RemoteTurn};
use glave_session::protocol::Meta;
use glave_session::protocol::{Answer, ApiError, ErrorData, Notification, Reply, Request, Update};
use glave_session::{Sent, Server, TurnItem, TurnStream};

use crate::asking;
use crate::config::{self, Endpoint};
use crate::data::{DataBlobs, DataLog};
use crate::gateway::GatewaySwitch;
use crate::input::{CtrlC, Input};
use crate::mcp::Servers;
use crate::terminal::Printer;

pub(crate) type Glave = Server<GatewaySwitch, DataLog, DataBlobs>;

/// The approvals a turn requested, by thread and call, until each is pending or decided.
type Requested = HashMap<(ThreadId, CallId), ApprovalRequested>;

/// What the terminal says of a message queued to a turn already running in its thread,
/// which it does not follow.
const QUEUED: &str = "· queued to the turn running in this thread";

/// The session API the terminal is a client of: the server in this process, or one elsewhere
/// over HTTP. The terminal uses the same requests and follows turns the same way with either.
#[derive(Clone, Copy)]
pub(crate) enum Session<'a> {
    Here(&'a Glave),
    Elsewhere(&'a Remote),
}

impl Session<'_> {
    /// Answers `request`, one that does not stream.
    pub(crate) async fn request(&self, request: Request) -> Result<Reply, ApiError> {
        // Boxed, as starting a turn below: the server's futures are large, and awaited often.
        match self {
            Self::Here(server) => Box::pin(server.request(request)).await,
            Self::Elsewhere(remote) => Box::pin(remote.request(&request)).await,
        }
    }

    /// Starts `request`, a turn or a goal, and follows it; a message sent while a turn runs in
    /// its thread is queued to that turn instead.
    pub(crate) async fn start(&self, request: Request) -> Result<Sent<Following>, ApiError> {
        match self {
            Self::Here(server) => Box::pin(server.start_streaming(&Caller::default(), request))
                .await
                .map(|sent| sent.map(Following::Here)),
            Self::Elsewhere(remote) => Box::pin(remote.start(&request))
                .await
                .map(|sent| sent.map(Following::Elsewhere)),
        }
    }
}

/// A turn or a goal as it runs, here or elsewhere.
pub(crate) enum Following {
    Here(TurnStream),
    Elsewhere(RemoteTurn),
}

impl Following {
    async fn next(&mut self) -> Option<TurnItem> {
        match self {
            Self::Here(stream) => stream.next().await,
            Self::Elsewhere(turn) => turn.next().await,
        }
    }
}

/// What a turn is followed with.
pub(crate) struct Client<'a> {
    pub server: Session<'a>,
    pub thread: ThreadId,
    pub input: &'a Input,
    /// The model endpoint, when the model is called from here: for receipts and fixes.
    pub endpoint: Option<&'a Endpoint>,
    /// Whether approvals are asked in this terminal; without one, the server answers "no one
    /// there" itself.
    pub asks: bool,
    /// The MCP servers started here, if any.
    pub servers: Option<&'a Servers>,
    /// The model and mode each turn chooses for the thread, when the person named them for a
    /// server elsewhere: the server records a choice only when it changes.
    pub model: Option<String>,
    pub permissions: Option<Permissions>,
}

impl Client<'_> {
    /// Runs one turn, which Ctrl-C cancels, printing its progress and receipt. When it
    /// proposed a plan, prints it, and with someone here asks whether to build it, or what to
    /// change, and runs that too (`plan.rs`).
    pub(crate) async fn turn(&self, prompt: &str, printer: &mut Printer) -> ExitCode {
        self.report(printer).await;
        let mut next = self.turn_start(prompt);
        loop {
            // Listening before the turn starts: the model may be asked before its answer
            // arrives.
            let interrupt = Cancel::new();
            let _ctrl_c = CtrlC::cancels(&interrupt);
            let started = self.server.start(next).await;
            let (code, proposed) = self.follow(started, &interrupt, printer, false).await;
            if !proposed || code != ExitCode::SUCCESS {
                return code;
            }
            match self.after_plan(printer, &interrupt).await {
                Some(request) => next = request,
                None => return code,
            }
        }
    }

    /// The request for a turn with `prompt`.
    fn turn_start(&self, prompt: &str) -> Request {
        Request::TurnStart {
            thread: self.thread,
            prompt: prompt.to_owned(),
            background: false,
            budget: None,
            model: self.model.clone(),
            permissions: self.permissions,
            effort: None,
            meta: Meta::default(),
            // With no one here, no one answers: what would ask is declined at once.
            unattended: !self.asks,
            files: Vec::new(),
        }
    }

    /// Works toward `goal` across turns until it ends, which Ctrl-C does too, printing every
    /// turn's progress and then the receipt for all of them. Succeeds only when it is met.
    pub(crate) async fn goal(&self, goal: Goal, printer: &mut Printer) -> ExitCode {
        self.report(printer).await;
        let interrupt = Cancel::new();
        let _ctrl_c = CtrlC::cancels(&interrupt);
        let started = self
            .server
            .start(Request::GoalStart {
                thread: self.thread,
                objective: Some(goal.objective),
                budget: Some(goal.token_budget),
                background: false,
                // With no one here, no one answers: what would ask is declined at once.
                unattended: !self.asks,
            })
            .await;
        let (code, _) = self.follow(started, &interrupt, printer, true).await;
        let get = Request::GoalGet {
            thread: self.thread,
        };
        match self.server.request(get).await {
            Ok(Reply::Goal { goal: Some(goal) }) if goal.ended.is_some_and(GoalReason::met) => {
                ExitCode::SUCCESS
            }
            _ if code == ExitCode::from(130) => code,
            _ => ExitCode::FAILURE,
        }
    }

    /// Follows a started turn, or a `goal`'s turns, to the end, cancelling it on `interrupt`.
    /// A message queued to a turn already running is said to be, and is no failure. Also says
    /// whether it may have proposed a plan, which the server is asked about.
    async fn follow(
        &self,
        started: Result<Sent<Following>, ApiError>,
        interrupt: &Cancel,
        printer: &mut Printer,
        goal: bool,
    ) -> (ExitCode, bool) {
        let mut stream = match started {
            Ok(Sent::Started(stream)) => stream,
            Ok(Sent::Queued(_)) => {
                printer.status(QUEUED);
                return (ExitCode::SUCCESS, false);
            }
            Err(error) => {
                self.show_error(printer, &error);
                return (ExitCode::FAILURE, false);
            }
        };
        let mut proposed = false;
        let mut cancelling = false;
        let mut requested = Requested::new();
        let ended = loop {
            let item = if cancelling {
                stream.next().await
            } else if let Some(item) = interrupt.run(stream.next()).await {
                item
            } else {
                cancelling = self.cancel().await;
                continue;
            };
            match item {
                Some(TurnItem::Ended(ended)) => break Some(ended),
                Some(TurnItem::Note(note)) => {
                    proposed |= proposes(self.thread, &note);
                    if self
                        .note(&note, &mut requested, printer, interrupt, goal)
                        .await
                    {
                        cancelling = self.cancel().await;
                    }
                }
                // One of them may have been the proposal.
                Some(TurnItem::Missed(_)) => proposed = true,
                None => break None,
            }
        };
        let code = match ended {
            Some(Ok(turn)) => {
                printer.receipt(&turn);
                match turn.outcome {
                    Outcome::Completed => ExitCode::SUCCESS,
                    // As shells report a program stopped by Ctrl-C.
                    Outcome::Cancelled => ExitCode::from(130),
                    _ => ExitCode::FAILURE,
                }
            }
            Some(Err(error)) => {
                self.show_error(printer, &error);
                ExitCode::FAILURE
            }
            None => ExitCode::FAILURE,
        };
        (code, proposed)
    }

    /// Shows one notification of a turn, or of a `goal`'s; answers an approval once it is
    /// pending, never one already decided, as one the judge let run. True when the person
    /// pressed Ctrl-C instead of answering.
    async fn note(
        &self,
        note: &Notification,
        requested: &mut Requested,
        printer: &mut Printer,
        interrupt: &Cancel,
        goal: bool,
    ) -> bool {
        match note {
            // A turn the terminal follows is not a task, nor a list.
            Notification::TaskFinished { .. }
            | Notification::Changed { .. }
            | Notification::SettingsChanged { .. } => false,
            Notification::Live {
                thread,
                update: Update::ApprovalPending { approval },
            } => match requested.remove(&(*thread, approval.clone())) {
                Some(asked) => self.decide(*thread, &asked, printer, interrupt).await,
                None => false,
            },
            // A subagent's steps are its own; its report reaches the model, not here.
            Notification::Live { thread, .. } if *thread != self.thread => false,
            Notification::Live {
                update:
                    Update::Asked {
                        question,
                        questions,
                    },
                ..
            } => self.answer(question, questions, printer, interrupt).await,
            Notification::Live { update, .. } => {
                printer.update(update.clone());
                false
            }
            Notification::Event { thread, event } => {
                let Ok(decoded) = decode(event.clone()) else {
                    return false;
                };
                match decoded.body {
                    // Asked about once pending; dropped once decided.
                    Body::ApprovalRequested(asked) => {
                        requested.insert((*thread, asked.call_id.clone()), asked);
                    }
                    Body::ApprovalDecided(decision) => {
                        requested.remove(&(*thread, decision.call_id));
                    }
                    Body::GoalEnded(ended) => {
                        printer.goal_ended(ended.reason, ended.tokens, ended.note.as_deref());
                    }
                    // A goal's continuation; glave's other words are the model's to read.
                    Body::MessageAdded(added)
                        if goal
                            && added
                                .author
                                .as_ref()
                                .is_some_and(|author| author.kind == PrincipalKind::Glave) =>
                    {
                        printer.status("\n· continuing toward the goal");
                    }
                    _ => {}
                }
                false
            }
        }
    }

    /// Answers `asked`, pending in `thread`, as the person says when this terminal asks. True when the person pressed Ctrl-C
    /// instead of answering.
    async fn decide(
        &self,
        thread: ThreadId,
        asked: &ApprovalRequested,
        printer: &mut Printer,
        interrupt: &Cancel,
    ) -> bool {
        let decision = if self.asks {
            let outside = asked.outside_sandbox.as_ref();
            printer.approval(&asked.summary, asked.caution.as_deref(), outside);
            let Some(line) = interrupt.run(self.input.line()).await else {
                return true;
            };
            match line
                .map(|answer| answer.trim().to_ascii_lowercase())
                .as_deref()
            {
                Some("y" | "yes") => Answer::AllowOnce,
                Some("a" | "always") => Answer::AllowForThread,
                _ => Answer::Deny,
            }
        } else {
            // Started answered by no one, so never pending; nothing to answer.
            return false;
        };
        let decide = Request::ApprovalDecide {
            thread,
            approval: asked.call_id.clone(),
            decision,
        };
        if let Err(error) = self.server.request(decide).await {
            printer.error(&error.to_string());
        }
        false
    }

    /// Puts the model's questions to the person and sends the answers. True when the person
    /// pressed Ctrl-C instead of answering.
    async fn answer(
        &self,
        question: &CallId,
        questions: &[Question],
        printer: &mut Printer,
        interrupt: &Cancel,
    ) -> bool {
        let mut answers = Vec::with_capacity(questions.len());
        for asked in questions {
            printer.ask_question(asked);
            let Some(line) = interrupt.run(self.input.line()).await else {
                return true;
            };
            answers.push(asking::answer(asked, line.as_deref().unwrap_or_default()));
        }
        let answer = Request::QuestionAnswer {
            thread: self.thread,
            question: question.clone(),
            answers,
        };
        if let Err(error) = self.server.request(answer).await {
            printer.error(&error.to_string());
        }
        false
    }

    /// Asks the server to cancel the turn; true once asked, so it is asked only once.
    async fn cancel(&self) -> bool {
        // The turn may have just ended on its own, which is fine.
        let _ = self
            .server
            .request(Request::TurnCancel {
                thread: self.thread,
            })
            .await;
        true
    }

    /// Reports the MCP servers started here, if there are any.
    async fn report(&self, printer: &mut Printer) {
        if let Some(servers) = self.servers {
            servers.report(printer).await;
        }
    }

    /// Shows what failed and, when the person can fix it, what to change.
    fn show_error(&self, printer: &mut Printer, error: &ApiError) {
        printer.error(&error.message);
        if let Some(ErrorData::Failed { failure }) = &error.data
            && let Some(endpoint) = self.endpoint
            && let Some(fix) = config::fix(*failure, endpoint)
        {
            printer.fix(&fix);
        }
    }
}

/// Whether `note` records a plan proposed in `thread`: what it is is the server's to say,
/// asked once the turn ends.
fn proposes(thread: ThreadId, note: &Notification) -> bool {
    let Notification::Event { thread: of, event } = note else {
        return false;
    };
    *of == thread
        && event.kind.as_str() == "plan.updated"
        && matches!(
            decode(event.clone()).map(|decoded| decoded.body),
            Ok(Body::PlanUpdated(updated)) if updated.proposed
        )
}

/// Writes the thread's events as JSON Lines, one event per line, exactly as stored.
pub(crate) async fn export(
    server: Session<'_>,
    thread: ThreadId,
    path: &Path,
) -> anyhow::Result<()> {
    let mut lines = String::new();
    let mut after = None;
    loop {
        let page = Request::ThreadEvents {
            thread,
            after,
            limit: Some(5_000),
        };
        let Reply::Events { events, next } = server.request(page).await? else {
            anyhow::bail!("the server answered thread/events with something else");
        };
        for event in &events {
            lines.push_str(&event.to_json()?);
            lines.push('\n');
        }
        match next {
            Some(seq) => after = Some(seq),
            None => break,
        }
    }
    tokio::fs::write(path, lines).await?;
    Ok(())
}
