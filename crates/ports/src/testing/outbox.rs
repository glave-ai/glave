//! A mailer that keeps what it is asked to send, for tests: the in-memory double of the email
//! plug. It sends nothing anywhere.

use std::future::ready;
use std::sync::{Mutex, PoisonError};

use crate::mail::{Mail, MailError, Mailer};
use crate::tools::BoxFuture;

/// Every message sent, in order.
#[derive(Default)]
pub struct Outbox(Mutex<Vec<Mail>>);

impl Outbox {
    /// What was sent so far.
    pub fn sent(&self) -> Vec<Mail> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl Mailer for Outbox {
    fn send<'a>(&'a self, mail: &'a Mail) -> BoxFuture<'a, Result<(), MailError>> {
        if !mail.to.contains('@') {
            return Box::pin(ready(Err(MailError::Address(mail.to.clone()))));
        }
        let mut sent = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        sent.push(mail.clone());
        Box::pin(ready(Ok(())))
    }
}
