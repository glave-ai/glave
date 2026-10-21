//! Keeping the person's own computer from going to sleep on its own while glave works there
//! (`specs/one-server.md`, owner decision 10): from when the first turn, task or compaction
//! starts until the last one ends, whatever ends it, the host holds the computer awake; then
//! it lets go. The screen may still sleep, and closing the lid still sleeps. The person turns it
//! off and on in Settings (`awake/get`, `awake/set`); it is on until they do.

use std::sync::{Arc, Mutex, PoisonError};

use glave_ports::{BlobStore, BoxFuture, Caller, EventLog, ModelProvider};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::protocol::{ApiError, AwakeInfo, Reply, code};
use crate::providers::local;
use crate::server::Server;
use crate::words::Sentence;

/// How the host keeps its computer awake (a power assertion on macOS, an inhibitor on Linux),
/// and where it keeps the person's choice of it.
pub trait KeepAwake: Send + Sync {
    /// Keeps the computer from sleeping on its own until what it answers is dropped; nothing,
    /// when it could not, which it says in the log.
    fn hold(&self) -> BoxFuture<'_, Option<Box<dyn Send>>>;

    /// Why the host cannot keep this computer awake, in a sentence, when it has nothing to
    /// keep it with.
    fn cannot(&self) -> Option<String>;

    /// Whether the person wants it kept awake, as they had chosen when the host started: yes
    /// until they choose.
    fn chosen(&self) -> bool;

    /// Keeps the person's choice, for the servers that start after this one too.
    fn choose(&self, on: bool) -> BoxFuture<'_, Result<(), ApiError>>;
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
}

/// A piece of work running: the computer stays awake until the last one is dropped.
#[must_use = "the computer is held awake only while it is kept"]
pub(crate) struct Working(Arc<watch::Sender<State>>);

impl Awake {
    pub(crate) fn new(keep: Arc<dyn KeepAwake>) -> Self {
        let on = keep.chosen();
        Self {
            keep,
            state: Arc::new(watch::Sender::new(State { working: 0, on })),
            keeper: Mutex::new(None),
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
            Some(reason) => AwakeInfo::Cannot { reason },
            None => AwakeInfo::Chosen {
                on: self.state.borrow().on,
            },
        }
    }

    /// Keeps the person's choice, and follows it at once: let go when it is off, held when it
    /// is on and work runs.
    async fn choose(&self, on: bool) -> Result<AwakeInfo, ApiError> {
        if let AwakeInfo::Cannot { reason } = self.info() {
            return Err(ApiError::new(code::NOT_AVAILABLE, reason));
        }
        self.keep.choose(on).await?;
        self.state.send_modify(|state| state.on = on);
        Ok(self.info())
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
    pub(crate) fn awake(&self) -> Reply {
        let awake = self
            .inner
            .awake
            .as_ref()
            .map_or(AwakeInfo::Never, Awake::info);
        Reply::Awake { awake }
    }

    /// Answers `awake/set`, for the person on this computer, where it is theirs to choose.
    pub(crate) async fn set_awake(&self, caller: &Caller, on: bool) -> Result<Reply, ApiError> {
        let Some(awake) = &self.inner.awake else {
            return Err(ApiError::new(code::NOT_AVAILABLE, Sentence::AwakeOnServer));
        };
        if !local(caller) {
            return Err(ApiError::new(code::NOT_ALLOWED, Sentence::AwakeOnlyHere));
        }
        Ok(Reply::Awake {
            awake: awake.choose(on).await?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A host that records each hold and each letting go, and keeps the choice it is given.
    #[derive(Default)]
    struct Recorded {
        seen: Arc<Mutex<Vec<&'static str>>>,
        held: watch::Sender<usize>,
        chosen: Mutex<Vec<bool>>,
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

        fn cannot(&self) -> Option<String> {
            None
        }

        fn chosen(&self) -> bool {
            true
        }

        fn choose(&self, on: bool) -> BoxFuture<'_, Result<(), ApiError>> {
            self.chosen.lock().unwrap().push(on);
            Box::pin(async { Ok(()) })
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
            let off = awake.choose(false).await.unwrap();
            assert_eq!(off, AwakeInfo::Chosen { on: false });
            held.wait_for(|held| *held == 0).await.unwrap();
            awake.choose(true).await.unwrap();
            held.wait_for(|held| *held == 1).await.unwrap();
            drop(working);
            held.wait_for(|held| *held == 0).await.unwrap();
            assert_eq!(*host.chosen.lock().unwrap(), [false, true]);
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
