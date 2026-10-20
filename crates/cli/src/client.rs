//! The CLI as a client of the session API: following a turn as it runs, answering approvals in
//! the terminal, compacting, and showing what failed with what to fix. It uses nothing a
//! remote client could not.

use std::collections::HashMap;
use std::future::{Future, poll_fn};
use std::path::Path;
use std::pin::pin;
use std::process::ExitCode;
use std::task::Poll;
use std::time::Duration;

use glave_event::{
    ApprovalRequested, Body, CallId, GoalReason, Outcome, PrincipalKind, ThreadId, decode,
};
use glave_harness::{ApprovalMode, Cancel, Goal, Question};
use glave_ports::Caller;
use glave_session::client::{Remote, RemoteTurn};
use glave_session::protocol::Meta;
use glave_session::protocol::{Answer, ApiError, Notification, Reply, Request, Update, code};
use glave_session::{Server, TurnItem, TurnStream};

use crate::asking;
use crate::config::{self, Endpoint};
use crate::data::{DataBlobs, DataLog};
use crate::gateway::GatewaySwitch;
use crate::input::{CtrlC, Input};
use crate::mcp::Servers;
use crate::terminal::Printer;
use crate::waiting::Waiting;

pub type Glave = Server<GatewaySwitch, DataLog, DataBlobs>;

/// The approvals a turn requested, by thread and call, until each is pending or decided.
type Requested = HashMap<(ThreadId, CallId), ApprovalRequested>;

/// The session API the terminal is a client of: the server in this process, or one elsewhere
/// over HTTP. The terminal uses the same requests and follows turns the same way with either.
#[derive(Clone, Copy)]
pub enum Session<'a> {
    Here(&'a Glave),
    Elsewhere(&'a Remote),
}

impl Session<'_> {
    /// Answers `request`, one that does not stream.
    pub async fn request(&self, request: Request) -> Result<Reply, ApiError> {
        // Boxed, as starting a turn below: the server's futures are large, and awaited often.
        match self {
            Self::Here(server) => Box::pin(server.request(request)).await,
            Self::Elsewhere(remote) => Box::pin(remote.request(&request)).await,
        }
    }

    /// Starts `request`, a turn or a goal, and follows it.
    pub async fn start(&self, request: Request) -> Result<Following, ApiError> {
        match self {
            Self::Here(server) => Box::pin(server.start_streaming(&Caller::default(), request))
                .await
                .map(Following::Here),
            Self::Elsewhere(remote) => Box::pin(remote.start(&request))
                .await
                .map(Following::Elsewhere),
        }
    }
}

/// A turn or a goal as it runs, here or elsewhere.
pub enum Following {
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
pub struct Client<'a> {
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
    /// The model and approval mode each turn chooses for the thread, when the person named
    /// them for a server elsewhere: the server records a choice only when it changes.
    pub model: Option<String>,
    pub approval: Option<ApprovalMode>,
}

impl Client<'_> {
    /// Runs one turn, which Ctrl-C cancels, printing its progress and receipt.
    pub async fn turn(&self, prompt: &str, printer: &mut Printer) -> ExitCode {
        self.report(printer).await;
        let started = self
            .server
            .start(Request::TurnStart {
                thread: self.thread,
                prompt: prompt.to_owned(),
                background: false,
                budget: None,
                model: self.model.clone(),
                approval: self.approval,
                meta: Meta::default(),
            })
            .await;
        self.follow(started, printer).await
    }

    /// Works toward `goal` across turns until it ends, which Ctrl-C does too, printing every
    /// turn's progress and then the receipt for all of them. Succeeds only when it is met.
    pub async fn goal(&self, goal: Goal, printer: &mut Printer) -> ExitCode {
        self.report(printer).await;
        let started = self
            .server
            .start(Request::GoalStart {
                thread: self.thread,
                objective: Some(goal.objective),
                budget: Some(goal.token_budget),
                background: false,
            })
            .await;
        let code = self.follow(started, printer).await;
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

    /// Follows a started turn, or a goal's turns, to the end.
    async fn follow(
        &self,
        started: Result<Following, ApiError>,
        printer: &mut Printer,
    ) -> ExitCode {
        let mut stream = match started {
            Ok(stream) => stream,
            Err(error) => {
                self.show_error(printer, &error);
                return ExitCode::FAILURE;
            }
        };
        let interrupt = Cancel::new();
        let _ctrl_c = CtrlC::cancels(&interrupt);
        let waiting = printer.waiting();
        let ended = ticking(
            async {
                let mut cancelling = false;
                let mut requested = Requested::new();
                loop {
                    let item = if cancelling {
                        stream.next().await
                    } else if let Some(item) = interrupt.run(stream.next()).await {
                        item
                    } else {
                        cancelling = self.cancel().await;
                        continue;
                    };
                    match item {
                        Some(TurnItem::Ended(ended)) => return Some(ended),
                        Some(TurnItem::Note(note)) => {
                            if self.note(&note, &mut requested, printer, &interrupt).await {
                                cancelling = self.cancel().await;
                            }
                        }
                        Some(TurnItem::Missed(_)) => {}
                        None => return None,
                    }
                }
            },
            &waiting,
        )
        .await;
        match ended {
            Some(Ok(turn)) => {
                printer.receipt(&turn, self.endpoint.is_some_and(|endpoint| endpoint.local));
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
        }
    }

    /// Shows one notification; answers an approval once it is pending, never one already
    /// decided, as one the judge let run. True when the person pressed Ctrl-C instead of
    /// answering.
    async fn note(
        &self,
        note: &Notification,
        requested: &mut Requested,
        printer: &mut Printer,
        interrupt: &Cancel,
    ) -> bool {
        match note {
            // A turn the terminal follows is not a task.
            Notification::TaskFinished { .. } => false,
            Notification::Live {
                thread,
                update: Update::ApprovalPending { approval },
            } => match requested.remove(&(*thread, approval.clone())) {
                Some(asked) => self.decide(*thread, &asked, printer, interrupt).await,
                None => false,
            },
            Notification::Live { thread, update } if *thread != self.thread => {
                printer.child(*thread, update.clone());
                false
            }
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
                    Body::MessageAdded(added)
                        if added
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

    /// Answers `asked`, pending in `thread`: as the person says when this terminal asks, no
    /// when the server is elsewhere and no one is here. True when the person pressed Ctrl-C
    /// instead of answering.
    async fn decide(
        &self,
        thread: ThreadId,
        asked: &ApprovalRequested,
        printer: &mut Printer,
        interrupt: &Cancel,
    ) -> bool {
        let decision = if self.asks {
            // A subagent asks under its own thread.
            let summary = printer.labeled(thread, &asked.summary);
            let outside = asked.outside_sandbox.as_ref();
            printer.approval(&summary, asked.caution.as_deref(), outside);
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
        } else if matches!(self.server, Session::Elsewhere(_)) {
            // A server elsewhere waits for its clients; no one is at this terminal.
            Answer::Deny
        } else {
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

    /// Summarizes the thread now, which Ctrl-C cancels.
    pub async fn compact(&self, printer: &mut Printer) {
        let interrupt = Cancel::new();
        let _ctrl_c = CtrlC::cancels(&interrupt);
        let compact = self.server.request(Request::ThreadCompact {
            thread: self.thread,
        });
        let mut compact = pin!(compact);
        let replied = if let Some(replied) = interrupt.run(compact.as_mut()).await {
            replied
        } else {
            self.cancel().await;
            compact.await
        };
        match replied {
            Ok(Reply::Compacted {
                before_tokens,
                after_tokens,
            }) => printer.update(Update::Compacted {
                before_tokens,
                after_tokens,
            }),
            Ok(_) => {}
            Err(error) if error.code == code::CANCELLED => {
                printer.error("compaction cancelled; the thread is as it was");
            }
            Err(error) => self.show_error(printer, &error),
        }
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
        if let Some(data) = &error.data
            && let Some(endpoint) = self.endpoint
            && let Some(fix) = config::fix(data.failure, endpoint)
        {
            printer.fix(&fix);
        }
    }
}

/// Runs `work`, updating the waiting line once a second until it finishes.
async fn ticking<T>(work: impl Future<Output = T>, waiting: &Waiting) -> T {
    let mut work = pin!(work);
    let mut clock = tokio::time::interval(Duration::from_secs(1));
    poll_fn(|cx| {
        if let Poll::Ready(value) = work.as_mut().poll(cx) {
            return Poll::Ready(value);
        }
        while clock.poll_tick(cx).is_ready() {
            waiting.tick();
        }
        Poll::Pending
    })
    .await
}

/// Writes the thread's events as JSON Lines, one event per line, exactly as stored.
pub async fn export(server: Session<'_>, thread: ThreadId, path: &Path) -> anyhow::Result<()> {
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
