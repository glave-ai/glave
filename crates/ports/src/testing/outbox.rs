//! A mailer that keeps what it is asked to send, for tests: the in-memory double of the email
//! plug. It sends nothing anywhere.

use std::future::ready;

use tokio::sync::watch;

use super::HUNG;
use crate::mail::{Mail, MailError, Mailer};
use crate::tools::BoxFuture;

/// Every message sent, in order; or, refusing, none, as a mail server that is down.
pub struct Outbox {
    sent: watch::Sender<Vec<Mail>>,
    refusing: bool,
    /// Whether each message waits to be let through, as a slow mail server holds it.
    holding: watch::Sender<bool>,
}

impl Default for Outbox {
    fn default() -> Self {
        Self {
            sent: watch::Sender::new(Vec::new()),
            refusing: false,
            holding: watch::Sender::new(false),
        }
    }
}

impl Outbox {
    /// One whose mail server refuses everything.
    pub fn refusing() -> Self {
        Self {
            refusing: true,
            ..Self::default()
        }
    }

    /// What was sent so far.
    pub fn sent(&self) -> Vec<Mail> {
        self.sent.borrow().clone()
    }

    /// What was sent once `count` messages were, as a server sends a link after it answers;
    /// after [`HUNG`] without them, what was sent by then.
    pub async fn sent_by(&self, count: usize) -> Vec<Mail> {
        let mut sent = self.sent.subscribe();
        let _ = tokio::time::timeout(HUNG, sent.wait_for(|sent| sent.len() >= count)).await;
        self.sent()
    }

    /// From now on, every message waits to be sent until [`Outbox::release`], or [`HUNG`].
    pub fn hold(&self) {
        self.holding.send_replace(true);
    }

    /// Sends every message held, and those after at once.
    pub fn release(&self) {
        self.holding.send_replace(false);
    }
}

impl Mailer for Outbox {
    fn reaches(&self, to: &str) -> bool {
        to.contains('@') && !to.chars().any(char::is_whitespace)
    }

    fn send<'a>(&'a self, mail: &'a Mail) -> BoxFuture<'a, Result<(), MailError>> {
        if !self.reaches(&mail.to) {
            return Box::pin(ready(Err(MailError::Address(mail.to.clone()))));
        }
        if self.refusing {
            return Box::pin(ready(Err(MailError::Send("refused".into()))));
        }
        let mut holding = self.holding.subscribe();
        Box::pin(async move {
            let _ = tokio::time::timeout(HUNG, holding.wait_for(|held| !*held)).await;
            self.sent.send_modify(|sent| sent.push(mail.clone()));
            Ok(())
        })
    }
}
