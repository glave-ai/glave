//! Keeping the person's own computer from going to sleep on its own while glave works there
//! (`specs/one-server.md`, owner decision 10): from when the first turn, task or compaction
//! starts until the last one ends, whatever ends it, the host holds the computer awake; then
//! it lets go. The screen may still sleep, and closing the lid still sleeps. The person turns it
//! off and on in Settings (`awake/get`, `awake/set`); it is on until they do. Their choice is
//! kept in their settings ledger (`awake.set`), so every server on one store reads the same.

use std::sync::{Arc, Mutex, PoisonError};

use glave_event::{AwakeSet, Body};
use glave_ports::{BlobStore, BoxFuture, Caller, EventLog, ModelProvider};
use tokio::sync::{OnceCell, watch};
use tokio::task::JoinHandle;

use crate::protocol::{ApiError, AwakeInfo, Reply, code};
use crate::providers::local;
use crate::server::Server;
use crate::words::Sentence;

/// How the host keeps its computer awake: a power assertion on macOS, an inhibitor on Linux.
pub trait KeepAwake: Send + Sync {
    /// Keeps the computer from sleeping on its own until what it answers is dropped; nothing,
    /// when it could not, which it says in the log.
    fn hold(&self) -> BoxFuture<'_, Option<Box<dyn Send>>>;

    /// Why the host cannot keep this computer awake, when it has nothing to keep it with.
    fn cannot(&self) -> Option<Unawake>;
}

/// Why a host has nothing to keep its computer awake with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unawake {
    /// It runs Linux without systemd, which glave keeps it awake through.
    NoSystemd,
    /// Its system has nothing glave keeps it awake through.
    Nothing,
}

/// How many pieces of work run now, and whether the person wants the computer kept awake.
#[derive(Debug, Clone, Copy)]
struct State {
    working: usize,
    on: bool,
}

/// The work running on this computer, and the computer held awake while there is any and the
/// person wants it.
pub(crate) struct Awake {
    keep: Arc<dyn KeepAwake>,
    state: Arc<watch::Sender<State>>,
    /// What holds the computer awake while work runs, started with the first.
    keeper: Mutex<Option<JoinHandle<()>>>,
    /// Set once the person's choice was read from their settings ledger.
    read: OnceCell<()>,
}

/// A piece of work running: the computer stays awake until the last one is dropped.
#[must_use = "the computer is held awake only while it is kept"]
pub(crate) struct Working(Arc<watch::Sender<State>>);

impl Awake {
    /// Awake while work runs, until the person's choice is read.
    pub(crate) fn new(keep: Arc<dyn KeepAwake>) -> Self {
        Self {
            keep,
            state: Arc::new(watch::Sender::new(State {
                working: 0,
                on: true,
            })),
            keeper: Mutex::new(None),
            read: OnceCell::new(),
        }
    }

    /// Counts one more piece of work, until what it answers is dropped. Called in the
    /// runtime, where the keeper starts the first time.
    pub(crate) fn hold(&self) -> Working {
        self.state.send_modify(|state| state.working += 1);
        let mut keeper = self.keeper.lock().unwrap_or_else(PoisonError::into_inner);
        if keeper.is_none() {
            let (keep, state) = (Arc::clone(&self.keep), self.state.subscribe());
            *keeper = Some(tokio::spawn(keep_awake(keep, state)));
        }
        Working(Arc::clone(&self.state))
    }

    /// Whether the computer is kept awake, as Settings shows it.
    fn info(&self) -> AwakeInfo {
        match self.keep.cannot() {
            Some(unawake) => AwakeInfo::Cannot {
                reason: match unawake {
                    Unawake::NoSystemd => Sentence::AwakeNeedsSystemd,
                    Unawake::Nothing => Sentence::AwakeWithNothing,
                }
                .into(),
            },
            None => AwakeInfo::Chosen {
                on: self.state.borrow().on,
            },
        }
    }

    /// Follows the person's choice at once: let go when it is off, held when it is on and work
    /// runs.
    fn choose(&self, on: bool) -> AwakeInfo {
        self.state.send_modify(|state| state.on = on);
        self.info()
    }
}

impl Drop for Awake {
    /// Stops the keeper, which lets go of what it held.
    fn drop(&mut self) {
        let keeper = self
            .keeper
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(keeper) = keeper.take() {
            keeper.abort();
        }
    }
}

impl Drop for Working {
    fn drop(&mut self) {
        self.0
            .send_modify(|state| state.working = state.working.saturating_sub(1));
    }
}

/// Holds the computer awake whenever work runs and the person wants it, and lets go once no
/// work runs or they turn it off, until the state's sender is gone.
async fn keep_awake(keep: Arc<dyn KeepAwake>, mut state: watch::Receiver<State>) {
    loop {
        let wanted = |state: &State| state.working > 0 && state.on;
        if state.wait_for(wanted).await.is_err() {
            return;
        }
        let held = keep.hold().await;
        if state.wait_for(|state| !wanted(state)).await.is_err() {
            return;
        }
        drop(held);
    }
}

impl<P, L, B> Server<P, L, B>
where
    P: ModelProvider + 'static,
    L: EventLog + 'static,
    B: BlobStore + 'static,
{
    /// Answers `awake/get`.
    pub(crate) async fn awake(&self) -> Reply {
        let Some(awake) = &self.inner.awake else {
            return Reply::Awake {
                awake: AwakeInfo::Never,
            };
        };
        self.read_choice(awake).await;
        Reply::Awake {
            awake: awake.info(),
        }
    }

    /// Reads the person's choice from their settings ledger, the first time it is needed:
    /// awake with none, or one this glave cannot read. One that could not be read is read again
    /// next time.
    pub(crate) async fn read_choice(&self, awake: &Awake) {
        let read = awake.read.get_or_try_init(|| async {
            // Glave's own read, for the person on this computer, whose computer it is.
            let on = match self.setting::<AwakeSet>(&Caller::default()).await? {
                Some(Body::AwakeSet(set)) => set.on,
                _ => true,
            };
            awake.state.send_modify(|state| state.on = on);
            Ok::<_, ApiError>(())
        });
        if let Err(error) = read.await {
            tracing::warn!(%error, "could not read whether the person keeps this computer awake");
        }
    }

    /// Answers `awake/set`, for the person on this computer, where it is theirs to choose.
    pub(crate) async fn set_awake(&self, caller: &Caller, on: bool) -> Result<Reply, ApiError> {
        let Some(awake) = &self.inner.awake else {
            return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::AwakeOnServer));
        };
        if !local(caller) {
            return Err(ApiError::new(code::NOT_ALLOWED, Sentence::AwakeOnlyHere));
        }
        if let AwakeInfo::Cannot { reason } = awake.info() {
            return Err(ApiError::new(code::NOT_AVAILABLE, reason));
        }
        // Read first, so a read of the ledger from before this choice never overrides it.
        self.read_choice(awake).await;
        self.keep_setting(caller, &AwakeSet { on }).await?;
        Ok(Reply::Awake {
            awake: awake.choose(on),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host that records each hold and each letting go.
    #[derive(Default)]
    struct Recorded {
        seen: Arc<Mutex<Vec<&'static str>>>,
        held: watch::Sender<usize>,
    }

    /// What a [`Recorded`] hold answers: it lets go when dropped.
    struct Let {
        seen: Arc<Mutex<Vec<&'static str>>>,
        held: watch::Sender<usize>,
    }

    impl Drop for Let {
        fn drop(&mut self) {
            self.seen.lock().unwrap().push("let go");
            self.held.send_modify(|held| *held -= 1);
        }
    }

    impl KeepAwake for Recorded {
        fn hold(&self) -> BoxFuture<'_, Option<Box<dyn Send>>> {
            Box::pin(async {
                self.seen.lock().unwrap().push("held");
                self.held.send_modify(|held| *held += 1);
                let held = Let {
                    seen: Arc::clone(&self.seen),
                    held: self.held.clone(),
                };
                Some(Box::new(held) as Box<dyn Send>)
            })
        }

        fn cannot(&self) -> Option<Unawake> {
            None
        }
    }

    /// Lets the keeper run as far as it can go.
    async fn settle() {
        for _ in 0..16 {
            tokio::task::yield_now().await;
        }
    }

    #[test]
    fn work_that_starts_again_holds_it_again() {
        glave_ports::block_on!(async {
            let host = Arc::new(Recorded::default());
            let mut held = host.held.subscribe();
            let awake = Awake::new(Arc::clone(&host) as Arc<dyn KeepAwake>);
            drop(awake.hold());
            settle().await;
            let again = awake.hold();
            held.wait_for(|held| *held == 1).await.unwrap();
            drop(again);
            held.wait_for(|held| *held == 0).await.unwrap();
            settle().await;
            let seen = host.seen.lock().unwrap().clone();
            assert_eq!(seen.last(), Some(&"let go"));
            assert_eq!(
                seen.iter().filter(|each| **each == "held").count(),
                seen.len() / 2
            );
        });
    }

    #[test]
    fn turned_off_it_lets_go_at_once_and_turned_on_it_holds_again() {
        glave_ports::block_on!(async {
            let host = Arc::new(Recorded::default());
            let mut held = host.held.subscribe();
            let awake = Awake::new(Arc::clone(&host) as Arc<dyn KeepAwake>);
            assert_eq!(awake.info(), AwakeInfo::Chosen { on: true });
            let working = awake.hold();
            held.wait_for(|held| *held == 1).await.unwrap();
            let off = awake.choose(false);
            assert_eq!(off, AwakeInfo::Chosen { on: false });
            held.wait_for(|held| *held == 0).await.unwrap();
            awake.choose(true);
            held.wait_for(|held| *held == 1).await.unwrap();
            drop(working);
            held.wait_for(|held| *held == 0).await.unwrap();
            assert_eq!(
                *host.seen.lock().unwrap(),
                ["held", "let go", "held", "let go"]
            );
        });
    }

    #[test]
    fn a_server_that_goes_lets_go() {
        glave_ports::block_on!(async {
            let host = Arc::new(Recorded::default());
            let mut held = host.held.subscribe();
            let awake = Awake::new(Arc::clone(&host) as Arc<dyn KeepAwake>);
            let working = awake.hold();
            held.wait_for(|held| *held == 1).await.unwrap();
            drop(awake);
            held.wait_for(|held| *held == 0).await.unwrap();
            drop(working);
            assert_eq!(*host.seen.lock().unwrap(), ["held", "let go"]);
        });
    }
}
