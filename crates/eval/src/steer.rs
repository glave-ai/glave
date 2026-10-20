//! A correction sent while a turn runs (`[glave] steer`): queued to the thread once the agent's
//! `after`th tool call has finished, as a person types while it works, so the model reads it at
//! its next call, as glave's session API queues a message sent mid-turn.

use glave_event::EventId;
use glave_harness::{Inbox, Live, Queued, Thread};
use glave_ports::{Caller, Clock, Entropy};
use serde::Deserialize;

/// `[glave] steer = { after = 1, message = "…" }`: never sent before a tool call, never empty.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "Written")]
pub struct Steer {
    /// How many of the first turn's tool calls finish before it is sent: 1 or more.
    pub after: u32,
    pub message: String,
}

/// `steer` as a task writes it, before it is checked.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    after: u32,
    message: String,
}

impl TryFrom<Written> for Steer {
    type Error = String;

    fn try_from(written: Written) -> Result<Self, Self::Error> {
        if written.after == 0 || written.message.trim().is_empty() {
            return Err("steer is sent after 1 or more tool calls, with a message".into());
        }
        Ok(Self {
            after: written.after,
            message: written.message,
        })
    }
}

/// Sends a task's correction to a thread's running turn, once.
pub(crate) struct Steering<'a> {
    steer: &'a Steer,
    inbox: Inbox,
    finished: u32,
    sent: bool,
}

impl<'a> Steering<'a> {
    pub(crate) fn new(steer: &'a Steer, thread: &Thread) -> Self {
        Self {
            steer,
            inbox: thread.inbox().clone(),
            finished: 0,
            sent: false,
        }
    }

    /// Queues the correction once `live` shows the `after`th of the turn's own tool calls
    /// finished, run or declined: what was sent, the first time.
    pub(crate) fn see(
        &mut self,
        live: &Live,
        clock: &dyn Clock,
        entropy: &dyn Entropy,
    ) -> Option<&'a str> {
        let finished = matches!(live, Live::ToolFinished { .. } | Live::ToolDeclined { .. });
        if self.sent || !finished {
            return None;
        }
        self.finished = self.finished.saturating_add(1);
        if self.finished < self.steer.after {
            return None;
        }
        let queued = Queued {
            id: EventId::new(clock.now(), entropy.bytes().ok()?).ok()?,
            text: self.steer.message.clone(),
            files: Vec::new(),
            caller: Caller::default(),
        };
        self.inbox.push(queued).ok()?;
        self.sent = true;
        Some(&self.steer.message)
    }

    /// Why the correction was never sent, if it was not: the agent stopped calling tools first.
    pub(crate) fn unsent(&self) -> Option<String> {
        (!self.sent).then(|| {
            format!(
                "the correction was never sent: the first turn ended with {} tool calls \
                 finished, and it waits for {}",
                self.finished, self.steer.after
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use glave_ports::testing::{CountingEntropy, ManualClock};

    use super::*;

    #[test]
    fn it_is_sent_once_after_the_tool_calls_it_waits_for() {
        let steer = Steer {
            after: 2,
            message: "use 4000".into(),
        };
        let inbox = Inbox::default();
        let mut steering = Steering {
            steer: &steer,
            inbox: inbox.clone(),
            finished: 0,
            sent: false,
        };
        let (clock, entropy) = (ManualClock::ticking(0), CountingEntropy::starting_at(1));
        let done = Live::ToolFinished {
            summary: "bash: ls".into(),
            is_error: false,
        };
        assert_eq!(steering.see(&Live::Waiting, &clock, &entropy), None);
        assert_eq!(steering.see(&done, &clock, &entropy), None);
        assert!(steering.unsent().is_some());
        assert_eq!(steering.see(&done, &clock, &entropy), Some("use 4000"));
        assert_eq!(steering.see(&done, &clock, &entropy), None, "once");
        assert_eq!(steering.unsent(), None);
        assert_eq!(inbox.end().len(), 1);
    }
}
