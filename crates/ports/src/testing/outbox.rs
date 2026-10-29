//! A mailer that keeps what it is asked to send, for tests: the in-memory double of the email
//! plug. It sends nothing anywhere.

use std::future::ready;
use std::sync::{Mutex, PoisonError};

use crate::mail::{Mail, MailError, Mailer};
use crate::tools::BoxFuture;

/// Every message sent, in order; or, refusing, none, as a mail server that is down.
#[derive(Default)]
pub struct Outbox(Mutex<Vec<Mail>>, bool);

impl Outbox {
    /// One whose mail server refuses everything.
    pub fn refusing() -> Self {
        Self(Mutex::default(), true)
    }

    /// What was sent so far.
    pub fn sent(&self) -> Vec<Mail> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        if self.1 {
            return Box::pin(ready(Err(MailError::Send("refused".into()))));
        }
        let mut sent = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        sent.push(mail.clone());
        Box::pin(ready(Ok(())))
    }
}
