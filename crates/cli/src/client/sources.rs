//! What the replies of a turn the terminal follows drew on from the web, shown once it ends as
//! the server says it (`thread/sources`), asked for only when the turn may have read a page;
//! and whether it proposed a plan, which `plan.rs` shows.

use glave_event::{Body, Seq, ThreadId, WebPage, decode};
use glave_session::protocol::{Notification, Reply, Request};

use super::Session;
use crate::terminal::Printer;

/// What the terminal saw of the turns it follows in a thread: where their events began,
/// whether they may have read a page, and whether one proposed a plan.
#[derive(Default)]
pub(crate) struct Followed {
    since: Option<Seq>,
    read: bool,
    proposed: bool,
}

impl Followed {
    /// Notes `note`, when it is an event of `thread`.
    pub(crate) fn note(&mut self, thread: ThreadId, note: &Notification) {
        let Notification::Event { thread: of, event } = note else {
            return;
        };
        if *of != thread {
            return;
        }
        self.since.get_or_insert(event.seq);
        // What a call read is the server's to say; the terminal only asks after one may have.
        if !matches!(event.kind.as_str(), "tool.call.requested" | "plan.updated") {
            return;
        }
        match decode(event.clone()).map(|decoded| decoded.body) {
            Ok(Body::ToolCallRequested(requested)) => {
                self.read |= requested.tool == WebPage::TOOL;
            }
            // What the plan is is the server's to say; the terminal asks once one was proposed.
            Ok(Body::PlanUpdated(updated)) => self.proposed |= updated.proposed,
            _ => {}
        }
    }

    /// Whether a turn it followed proposed a plan.
    pub(crate) fn proposed(&self) -> bool {
        self.proposed
    }

    /// Notes that notifications were missed: one may have been a page's read, or a plan's
    /// proposal.
    pub(crate) fn missed(&mut self) {
        self.read = true;
        self.proposed = true;
    }

    /// Shows what the replies written since the terminal began following drew on.
    pub(crate) async fn show(&self, session: Session<'_>, thread: ThreadId, printer: &mut Printer) {
        let (true, Some(since)) = (self.read, self.since) else {
            return;
        };
        // Sources say more about a reply already shown; without them, it still stands.
        let Ok(Reply::Sources { sources }) =
            session.request(Request::ThreadSources { thread }).await
        else {
            return;
        };
        for sourced in sources.iter().filter(|sourced| sourced.reply > since) {
            printer.sources(sourced);
        }
    }
}
