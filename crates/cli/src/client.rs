//! The CLI as a client of the session API: following a turn as it runs, answering approvals in
//! the terminal, compacting, and showing what failed with what to fix. It uses nothing a
//! remote client could not.

use std::future::{Future, poll_fn};
use std::pin::pin;
use std::process::ExitCode;
use std::task::Poll;
use std::time::Duration;

use glave_event::{Body, Outcome, ThreadId, decode};
use glave_harness::Cancel;
use glave_provider_openai::OpenAiCompatible;
use glave_session::protocol::{Answer, ApiError, Notification, Reply, Request, code};
use glave_session::{Server, TurnItem};
use glave_store_sqlite::{SqliteBlobs, SqliteLog};

use crate::config::{self, Endpoint};
use crate::input::{CtrlC, Input};
use crate::terminal::Printer;
use crate::waiting::Waiting;

pub type Glave = Server<OpenAiCompatible, SqliteLog, SqliteBlobs>;

/// What a turn is followed with.
pub struct Client<'a> {
    pub server: &'a Glave,
    pub thread: ThreadId,
    pub input: &'a Input,
    pub endpoint: &'a Endpoint,
    /// Whether approvals are asked in this terminal; without one, the server answers "no one
    /// there" itself.
    pub asks: bool,
}

impl Client<'_> {
    /// Runs one turn, which Ctrl-C cancels, printing its progress and receipt.
    pub async fn turn(&self, prompt: &str, printer: &mut Printer) -> ExitCode {
        let mut stream = match self.server.start_turn(self.thread, prompt).await {
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
                            if self.note(&note, printer, &interrupt).await {
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
                printer.receipt(&turn, self.endpoint.local);
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

    /// Shows one notification; answers an approval when this terminal asks them. True when the
    /// person pressed Ctrl-C instead of answering.
    async fn note(&self, note: &Notification, printer: &mut Printer, interrupt: &Cancel) -> bool {
        match note {
            Notification::Live { update, .. } => {
                printer.update(update.clone());
                false
            }
            Notification::Event { event, .. } => {
                let Ok(decoded) = decode(event.clone()) else {
                    return false;
                };
                let Body::ApprovalRequested(asked) = decoded.body else {
                    return false;
                };
                if !self.asks {
                    return false;
                }
                printer.approval(&asked.summary);
                let Some(line) = interrupt.run(self.input.line()).await else {
                    return true;
                };
                let decision = match line
                    .map(|answer| answer.trim().to_ascii_lowercase())
                    .as_deref()
                {
                    Some("y" | "yes") => Answer::AllowOnce,
                    Some("a" | "always") => Answer::AllowForThread,
                    _ => Answer::Deny,
                };
                let decide = Request::ApprovalDecide {
                    thread: self.thread,
                    approval: asked.call_id,
                    decision,
                };
                if let Err(error) = self.server.request(decide).await {
                    printer.error(&error.to_string());
                }
                false
            }
        }
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
            }) => printer.update(glave_session::protocol::Update::Compacted {
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

    /// Shows what failed and, when the person can fix it, what to change.
    fn show_error(&self, printer: &mut Printer, error: &ApiError) {
        printer.error(&error.message);
        if let Some(data) = &error.data
            && let Some(fix) = config::fix(data.failure, self.endpoint)
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
