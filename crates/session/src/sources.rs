//! What a thread's replies drew on from the web (`specs/session.md`, "Sources"), read from its
//! log: each `web_fetch` call records the page it read in its `tool.call.completed`
//! (`glave_event::WebPage`), and a turn's pages are its last reply's sources, those read before
//! it, each once.

use glave_event::{Body, Part, RawEvent, Role, Seq, ThreadId, WebPage, decode};
use glave_ports::{BlobStore, EventLog, ModelProvider};

use crate::protocol::{ApiError, Reply, ReplySources};
use crate::server::{MAX_PAGE, Server};
use crate::words::Sentence;

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// What `thread`'s replies drew on, read page by page from its log.
    pub(crate) async fn sources(&self, thread: ThreadId) -> Result<Reply, ApiError> {
        let mut reading = Reading::default();
        let mut after = None;
        loop {
            let (events, next) = self.page(thread, after, Some(MAX_PAGE)).await?;
            for event in events {
                reading.read(event);
            }
            match next {
                Some(next) => after = Some(next),
                None => break,
            }
        }
        Ok(Reply::Sources {
            sources: reading.done(),
        })
    }
}

/// A thread's log read so far, in order: the sources of the turns it has passed, and the turn
/// it is in.
#[derive(Default)]
pub(crate) struct Reading {
    sources: Vec<ReplySources>,
    turn: Turn,
}

/// What one turn read, each page with the `seq` that recorded it, and its latest reply.
#[derive(Default)]
struct Turn {
    pages: Vec<(Seq, WebPage)>,
    reply: Option<Seq>,
}

impl Reading {
    /// Reads the next event of the log.
    pub(crate) fn read(&mut self, event: RawEvent) {
        if !matches!(
            event.kind.as_str(),
            "turn.started" | "tool.call.completed" | "message.added"
        ) {
            return;
        }
        let seq = event.seq;
        let Ok(event) = decode(event) else {
            return;
        };
        match event.body {
            Body::TurnStarted(_) => {
                let ended = std::mem::take(&mut self.turn);
                self.sources.extend(ended.sources());
            }
            Body::ToolCallCompleted(done) if !done.is_error => {
                if let Some(page) = done.structured.as_ref().and_then(WebPage::read) {
                    self.turn.pages.push((seq, page));
                }
            }
            Body::MessageAdded(message)
                if message.role == Role::Assistant && said(&message.parts) =>
            {
                self.turn.reply = Some(seq);
            }
            _ => {}
        }
    }

    /// The sources of every reply read, in order, the turn it is in last.
    pub(crate) fn done(mut self) -> Vec<ReplySources> {
        self.sources.extend(self.turn.sources());
        self.sources
    }
}

impl Turn {
    /// Its reply's sources: the pages read before it, each once, in the order first read; none
    /// with no reply, or one that drew on no page.
    fn sources(self) -> Option<ReplySources> {
        let reply = self.reply?;
        let mut pages: Vec<WebPage> = Vec::new();
        for (seq, page) in self.pages {
            if seq < reply && !pages.iter().any(|kept| kept.url == page.url) {
                pages.push(page);
            }
        }
        if pages.is_empty() {
            return None;
        }
        let count = u64::try_from(pages.len()).unwrap_or(u64::MAX);
        Some(ReplySources {
            reply,
            sentence: Sentence::Sources { pages: count }.into(),
            pages,
        })
    }
}

/// Whether a message says anything: text that is not blank.
fn said(parts: &[Part]) -> bool {
    parts
        .iter()
        .any(|part| matches!(part, Part::Text { text } if !text.trim().is_empty()))
}

#[cfg(test)]
mod tests;
