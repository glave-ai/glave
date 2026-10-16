//! The computer asking its caller for credentials (`credentials/request`): while a command
//! offered them runs, each use its helper makes becomes a request of the computer's own on the
//! connection, which the caller answers. Nothing is kept: the answer goes to the helper that
//! asked and is dropped.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use glave_ports::{BoxFuture, CredentialAsk, CredentialHost, Credentials, GitCredential};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use crate::wire::{self, Asked, Given};

/// How long the caller has to answer.
const ANSWER: Duration = Duration::from_secs(30);
/// Requests waiting for their answer on one connection at once; more get none.
const WAITING: usize = 64;

/// The computer's own requests on one connection, waiting for their answers.
#[derive(Default)]
pub(super) struct Asks {
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, oneshot::Sender<Result<Value, Value>>>>,
}

impl Asks {
    /// Hands the caller's answer to the request `id` it answers; one nobody waits for is
    /// dropped.
    pub(super) fn answered(&self, id: u64, answer: Result<Value, Value>) {
        let waiting = self
            .waiting
            .lock()
            .ok()
            .and_then(|mut waiting| waiting.remove(&id));
        if let Some(waiting) = waiting {
            let _ = waiting.send(answer);
        }
    }

    /// Asks the caller for `asked`, and waits for the answer.
    async fn ask(&self, lines: &mpsc::Sender<String>, asked: &Asked) -> Option<GitCredential> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (answer, answered) = oneshot::channel();
        {
            let mut waiting = self.waiting.lock().ok()?;
            if waiting.len() >= WAITING {
                return None;
            }
            waiting.insert(id, answer);
        }
        let sent = lines.send(wire::ask(id, asked)).await.is_ok();
        let answer = if sent {
            tokio::time::timeout(ANSWER, answered).await.ok()
        } else {
            None
        };
        if let Ok(mut waiting) = self.waiting.lock() {
            waiting.remove(&id);
        }
        let result = answer?.ok()?.ok()?;
        Given::deserialize(result).ok()?.credential
    }
}

/// One run's credentials, as the computer sees them: the hosts the caller offered, and each
/// request sent back to the caller.
pub(super) struct Asking<'a> {
    pub(super) asks: &'a Asks,
    pub(super) lines: &'a mpsc::Sender<String>,
    pub(super) run: String,
    pub(super) hosts: Vec<CredentialHost>,
}

impl Credentials for Asking<'_> {
    fn hosts(&self) -> BoxFuture<'_, Vec<CredentialHost>> {
        Box::pin(std::future::ready(self.hosts.clone()))
    }

    fn request<'a>(&'a self, ask: &'a CredentialAsk) -> BoxFuture<'a, Option<GitCredential>> {
        let asked = Asked {
            run: self.run.clone(),
            ask: ask.clone(),
        };
        Box::pin(async move { self.asks.ask(self.lines, &asked).await })
    }
}
