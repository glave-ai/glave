//! The CLI as a client of the session API: following a turn as it runs, answering approvals in
//! the terminal, compacting, and showing what failed with what to fix. It uses nothing a
//! remote client could not.

pub(crate) mod plan;

use std::collections::HashMap;
use std::path::Path;
use std::process::ExitCode;

use glave_event::{
    ApprovalRequested, Body, CallId, Outcome, RawEvent, Seq, ServiceAnswer, ServiceAsked, ThreadId,
    decode,
};
use glave_harness::{Cancel, Permissions, Question};
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

/// What the terminal says when its input ends at an approval, which it leaves unanswered.
const LEFT: &str = "· your input ended, so nothing was answered";

/// What following a turn does after one of its notifications.
enum Next {
    /// Goes on following it.
    Follow,
    /// Cancels it: the person pressed Ctrl-C instead of answering.
    Cancel,
    /// Leaves it: the input ended before an answer, as when the terminal closes, so nothing
    /// was answered and the turn waits for one from any client (`specs/session.md`,
    /// "Approvals").
    Leave,
}

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

    /// Starts `request`, a turn, and follows it; a message sent while a turn runs in
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

/// A turn as it runs, here or elsewhere.
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
        self.report(printer);
        let mut next = self.turn_start(prompt);
        loop {
            // Listening before the turn starts: the model may be asked before its answer
            // arrives.
            let interrupt = Cancel::new();
            let _ctrl_c = CtrlC::cancels(&interrupt);
            let started = self.server.start(next).await;
            let (code, proposed) = self.follow(started, &interrupt, printer).await;
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

    /// Follows a started turn to the end, cancelling it on `interrupt`.
    /// A message queued to a turn already running is said to be, and is no failure. Also says
    /// whether it may have proposed a plan, which the server is asked about.
    async fn follow(
        &self,
        started: Result<Sent<Following>, ApiError>,
        interrupt: &Cancel,
        printer: &mut Printer,
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
                    match self.note(&note, &mut requested, printer, interrupt).await {
                        Next::Follow => {}
                        Next::Cancel => cancelling = self.cancel().await,
                        Next::Leave => {
                            printer.status(LEFT);
                            return (ExitCode::FAILURE, false);
                        }
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

    /// Shows one notification of a turn; answers an approval once it is
    /// pending, never one already decided, as one the judge let run. Says what follows.
    async fn note(
        &self,
        note: &Notification,
        requested: &mut Requested,
        printer: &mut Printer,
        interrupt: &Cancel,
    ) -> Next {
        match note {
            // A turn the terminal follows is not a task, nor a list.
            Notification::TaskFinished { .. }
            | Notification::Changed { .. }
            | Notification::SettingsChanged { .. } => Next::Follow,
            Notification::Live {
                thread,
                update: Update::ApprovalPending { approval },
            } => match requested.remove(&(*thread, approval.clone())) {
                Some(asked) => self.decide(*thread, &asked, printer, interrupt).await,
                None => Next::Follow,
            },
            // A subagent's steps are its own; its report reaches the model, not here.
            Notification::Live { thread, .. } if *thread != self.thread => Next::Follow,
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
                Next::Follow
            }
            Notification::Event { thread, event } => {
                let Ok(decoded) = decode(event.clone()) else {
                    return Next::Follow;
                };
                match decoded.body {
                    Body::TurnStarted(started) if *thread == self.thread => {
                        for gone in &started.missing {
                            printer.warning_once(&gone.message);
                        }
                    }
                    // A subagent's never asks: no one is there for it.
                    Body::ServiceAsked(asked) if *thread == self.thread => {
                        return self
                            .answer_service(decoded.header.seq, &asked, printer, interrupt)
                            .await;
                    }
                    Body::ServiceAnswered(answered) if *thread == self.thread => {
                        printer.status(&answered.told);
                    }
                    // Asked about once pending; dropped once decided.
                    Body::ApprovalRequested(asked) => {
                        requested.insert((*thread, asked.call_id.clone()), asked);
                    }
                    Body::ApprovalDecided(decision) => {
                        requested.remove(&(*thread, decision.call_id));
                    }
                    _ => {}
                }
                Next::Follow
            }
        }
    }

    /// Answers `asked`, pending in `thread`, as the person says when this terminal asks: yes,
    /// always in this thread, or anything else no. The end of input is no answer: the approval
    /// is left pending, for any client to answer.
    async fn decide(
        &self,
        thread: ThreadId,
        asked: &ApprovalRequested,
        printer: &mut Printer,
        interrupt: &Cancel,
    ) -> Next {
        if !self.asks {
            // Started answered by no one, so never pending; nothing to answer.
            return Next::Follow;
        }
        let outside = asked.outside_sandbox.as_ref();
        printer.approval(&asked.summary, asked.caution.as_deref(), outside);
        let Some(line) = interrupt.run(self.input.line()).await else {
            return Next::Cancel;
        };
        let Some(line) = line else {
            return Next::Leave;
        };
        let decision = match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => Answer::AllowOnce,
            "a" | "always" => Answer::AllowForThread,
            _ => Answer::Deny,
        };
        let decide = Request::ApprovalDecide {
            thread,
            approval: asked.call_id.clone(),
            decision,
        };
        if let Err(error) = self.server.request(decide).await {
            printer.error(&error.to_string());
        }
        Next::Follow
    }

    /// Puts the model's questions to the person and sends the answers; cancels the turn when
    /// the person pressed Ctrl-C instead of answering.
    async fn answer(
        &self,
        question: &CallId,
        questions: &[Question],
        printer: &mut Printer,
        interrupt: &Cancel,
    ) -> Next {
        let mut answers = Vec::with_capacity(questions.len());
        for asked in questions {
            printer.ask_question(asked);
            let Some(line) = interrupt.run(self.input.line()).await else {
                return Next::Cancel;
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
        Next::Follow
    }

    /// Puts what a service asks to the person, a field at a time, and sends what they answer, or
    /// that they declined; cancels the turn when the person pressed Ctrl-C instead of answering.
    async fn answer_service(
        &self,
        at: Seq,
        asked: &ServiceAsked,
        printer: &mut Printer,
        interrupt: &Cancel,
    ) -> Next {
        printer.service_asks(asked);
        let mut values = serde_json::Map::new();
        let mut declined = false;
        for field in &asked.fields {
            let answer = loop {
                printer.service_field(field);
                let Some(line) = interrupt.run(self.input.line()).await else {
                    return Next::Cancel;
                };
                match asking::field_answer(field, line.as_deref()) {
                    Ok(answer) => break answer,
                    Err(why) => printer.status(&why),
                }
            };
            match answer {
                asking::Field::Value(value) => {
                    values.insert(field.name.clone(), value);
                }
                asking::Field::Skipped => {}
                asking::Field::Declined => {
                    declined = true;
                    break;
                }
            }
        }
        // Nothing goes unasked: not a form with no fields, nor one of defaults alone.
        if !declined {
            printer.service_confirm(&asked.service);
            let Some(line) = interrupt.run(self.input.line()).await else {
                return Next::Cancel;
            };
            declined = !asking::confirmed(line.as_deref());
        }
        let answer = if declined {
            ServiceAnswer::Declined
        } else {
            ServiceAnswer::Answered { values }
        };
        let send = async |answer| {
            let request = Request::ServiceAnswer {
                thread: self.thread,
                call: asked.call_id.clone(),
                asked: at,
                answer,
            };
            self.server.request(request).await
        };
        for error in asking::sent(answer, send).await {
            printer.error(&error);
        }
        Next::Follow
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
    fn report(&self, printer: &mut Printer) {
        if let Some(servers) = self.servers {
            servers.report(printer);
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
    read_events(server, thread, |event| {
        lines.push_str(&event.to_json()?);
        lines.push('\n');
        Ok(())
    })
    .await?;
    tokio::fs::write(path, lines).await?;
    Ok(())
}

/// Hands each of the thread's events to `read`, in order, read a page at a time.
pub(crate) async fn read_events(
    server: Session<'_>,
    thread: ThreadId,
    mut read: impl FnMut(&RawEvent) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mut after = None;
    loop {
        let page = Request::ThreadEvents {
            thread,
            after,
            limit: Some(5_000),
        };
        let Reply::Events { events, next, .. } = server.request(page).await? else {
            anyhow::bail!("the server answered thread/events with something else");
        };
        events.iter().try_for_each(&mut read)?;
        match next {
            Some(seq) => after = Some(seq),
            None => return Ok(()),
        }
    }
}
