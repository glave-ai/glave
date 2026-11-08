//! Previews of what a thread's commands in the background serve (`specs/session.md`,
//! "Previews"). A preview never runs at glave's own origin, where a page could read the app's
//! sign-in and call its API: each is a listener of its own on this computer's loopback
//! (`127.0.0.1` at a port the system gives it, so an origin of its own, and a host apart from
//! the app's `localhost`, whose passkeys it cannot ask for), carrying every connection to the
//! port its command listens on, bytes as they are, so live reload's WebSocket goes through too.
//! Nothing off this computer reaches one. They are opened for the person whose computer it is,
//! as their client lists the thread's commands, and closed when the command ends or no longer
//! listens there, and when the thread is archived, deleted or let go, or the server stops.
//!
//! Only commands on this computer are previewed, to a browser on this computer: a command on a
//! worker, on a person's computer in the cloud, or a browser reaching the server at a public
//! origin needs each preview at a name of its own (`<id>.preview.<domain>`), reached through
//! the computer protocol (#356). That is where [`carry`]'s reach goes then.

mod carry;

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use glave_event::ThreadId;
use glave_harness::Running;
use glave_ports::computer::CommandId;
use glave_ports::host::PublicOrigin;
use glave_ports::{BlobStore, Caller, EventLog, ModelProvider};
use tokio::task::JoinHandle;

use crate::protocol::Preview;
use crate::server::Server;

/// The most previews a server keeps open at once; past it, a command's port waits for one to
/// close.
const MOST: usize = 32;

/// The previews open, each by its thread, its command and the port that command listens on.
#[derive(Debug, Default)]
pub(crate) struct Previews {
    open: Mutex<BTreeMap<Key, Open>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    thread: ThreadId,
    command: CommandId,
    port: u16,
}

/// A preview open: the port of its own on loopback, and the task that carries its connections,
/// stopped with every connection it carries when this goes.
#[derive(Debug)]
struct Open {
    at: u16,
    carrying: Option<JoinHandle<()>>,
}

impl Open {
    /// Stops it, answering its task, done once awaited.
    fn stopped(mut self) -> Option<JoinHandle<()>> {
        let carrying = self.carrying.take();
        if let Some(task) = &carrying {
            task.abort();
        }
        carrying
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        if let Some(task) = &self.carrying {
            task.abort();
        }
    }
}

/// How long a preview's task may take to end once stopped: at its next turn.
const CLOSING: Duration = Duration::from_secs(5);

/// Previews told to close, closed once this is awaited: their listeners and every connection
/// they carry gone.
#[derive(Debug)]
#[must_use = "a preview is closed for sure once this is awaited"]
pub(crate) struct Closing(Vec<JoinHandle<()>>);

impl Closing {
    pub(crate) async fn done(self) {
        for task in self.0 {
            let _ = tokio::time::timeout(CLOSING, task).await;
        }
    }
}

impl Previews {
    fn open(&self) -> MutexGuard<'_, BTreeMap<Key, Open>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Closes the previews `which` picks.
    fn closing(&self, which: impl Fn(&Key) -> bool) -> Closing {
        let mut open = self.open();
        let picked: Vec<Key> = open.keys().filter(|key| which(key)).copied().collect();
        let tasks = picked
            .iter()
            .filter_map(|key| open.remove(key))
            .filter_map(Open::stopped)
            .collect();
        Closing(tasks)
    }

    /// The previews of `thread`'s commands, oldest command first, `listening` naming each
    /// command and the port it listens on now: one not open yet is opened, while there is room,
    /// and one of `thread`'s no longer in it is closed.
    pub(crate) async fn settle(
        &self,
        thread: ThreadId,
        listening: &[(CommandId, u16)],
    ) -> Vec<Preview> {
        let gone = |key: &Key| {
            key.thread == thread && !listening.contains(&(key.command, key.port))
        };
        self.closing(gone).done().await;
        let mut open = self.open();
        for &(command, port) in listening {
            let key = Key {
                thread,
                command,
                port,
            };
            if open.contains_key(&key) || open.len() >= MOST {
                continue;
            }
            match opened(port) {
                Ok(preview) => {
                    open.insert(key, preview);
                }
                Err(error) => tracing::warn!(%error, "could not open a preview"),
            }
        }
        open.iter()
            .filter(|(key, _)| key.thread == thread)
            .map(|(key, preview)| Preview {
                command: key.command,
                port: key.port,
                url: format!("http://{}:{}/", Ipv4Addr::LOCALHOST, preview.at),
            })
            .collect()
    }

    /// Closes the previews of `thread`'s commands that no longer run, as `runs` says.
    pub(crate) fn keep_running(&self, thread: ThreadId, runs: impl Fn(CommandId) -> bool) -> Closing {
        self.closing(|key| key.thread == thread && !runs(key.command))
    }

    /// Closes every preview of `thread`.
    pub(crate) fn close(&self, thread: ThreadId) -> Closing {
        self.closing(|key| key.thread == thread)
    }

    /// Closes every preview.
    pub(crate) fn close_all(&self) -> Closing {
        self.closing(|_| true)
    }
}

/// A preview of the port `port` on this computer: a listener of its own on loopback, at a port
/// the system gives it, carrying each connection there.
fn opened(port: u16) -> std::io::Result<Open> {
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let at = listener.local_addr()?.port();
    let listener = tokio::net::TcpListener::from_std(listener)?;
    Ok(Open {
        at,
        carrying: Some(tokio::spawn(carry::carry(listener, port))),
    })
}

/// Whether previews on this computer's loopback reach a browser of whoever calls: the server is
/// reached at this computer alone (it has no public origin, or this computer's own
/// `localhost`), people have no computers of their own elsewhere, and this computer is the
/// caller's (`theirs`: the person it works for), as it is for a terminal (#1212's rule).
fn reached_here(origin: Option<&PublicOrigin>, homes: bool, theirs: bool) -> bool {
    origin.is_none_or(PublicOrigin::on_this_computer) && !homes && theirs
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// The previews of `thread`'s commands `running` for `caller`: none unless they reach a
    /// browser of theirs, and only of the commands on this computer.
    pub(crate) async fn previews_for(
        &self,
        caller: &Caller,
        thread: ThreadId,
        running: &[Running],
    ) -> Vec<Preview> {
        let theirs = self.computer_is_theirs(caller.person);
        let origin = self.inner.origin.as_ref();
        if !reached_here(origin, self.inner.homes.is_some(), theirs) {
            return Vec::new();
        }
        let here = self.inner.harness.workspace().info().computer;
        let listening: Vec<(CommandId, u16)> = running
            .iter()
            .filter(|command| command.computer == here)
            .flat_map(|command| command.ports.iter().map(|port| (command.shown.id, *port)))
            .collect();
        self.inner.previews.settle(thread, &listening).await
    }
}

#[cfg(test)]
mod tests;
