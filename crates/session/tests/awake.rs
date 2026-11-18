//! Keeping the computer awake while work runs (`specs/one-server.md`, owner decision 10): two
//! turns at once hold it once; when one ends it stays held, and when both have, it is let go.
//! The person turns it off and on from Settings (`awake/get`, `awake/set`); a server never
//! keeps one.

use std::sync::{Arc, Mutex};

use glave_event::{Outcome, Principal};
use glave_ports::{BoxFuture, EventLog, ProjectKind};
use glave_session::protocol::{AwakeInfo, Reply, Request, code};
use glave_session::{Answering, KeepAwake, Unawake};
use tokio::sync::{Notify, watch};

use crate::support::*;

/// A computer that counts how often it was held and how many holds it keeps now.
#[derive(Default)]
struct Computer {
    holds: Mutex<usize>,
    held: watch::Sender<usize>,
}

/// What a hold of [`Computer`] answers: it lets go when dropped.
struct Hold(watch::Sender<usize>);

impl Drop for Hold {
    fn drop(&mut self) {
        self.0.send_modify(|held| *held -= 1);
    }
}

impl KeepAwake for Computer {
    fn hold(&self) -> BoxFuture<'_, Option<Box<dyn Send>>> {
        Box::pin(async {
            *self.holds.lock().unwrap() += 1;
            self.held.send_modify(|held| *held += 1);
            Some(Box::new(Hold(self.held.clone())) as Box<dyn Send>)
        })
    }

    fn cannot(&self) -> Option<Unawake> {
        None
    }
}

/// Lets the server's work, and what keeps the computer awake, run as far as they can.
async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

#[test]
fn two_turns_hold_the_computer_once_and_the_last_to_end_lets_it_go() {
    let dir = folder("awake");
    block_on!(async {
        let computer = Arc::new(Computer::default());
        let mut held = computer.held.subscribe();
        let gate = Arc::new(Notify::new());
        let answers = vec![answer(Some("One."), None), answer(Some("Two."), None)];
        let mut setup = Setup::asking(answers, Answering::NoOne);
        setup.gate = Some(Arc::clone(&gate));
        setup.awake = Some(Arc::clone(&computer) as Arc<dyn KeepAwake>);
        let (server, seen) = setup.build(&dir).await;
        let (one, two) = (start(&server, false).await, start(&server, false).await);
        let mut first = stream_turn(&server, one, "slow", None).await.unwrap();
        let mut second = stream_turn(&server, two, "slow", None).await.unwrap();
        // Both turns wait for the model: the computer is held, once.
        while seen.lock().unwrap().len() < 2 {
            tokio::task::yield_now().await;
        }
        held.wait_for(|held| *held == 1).await.unwrap();

        // One ends, whichever the model answers first: the other still runs, so it stays held.
        gate.notify_one();
        let first_ended = tokio::select! {
            ended = first.finish() => {
                assert_eq!(ended.unwrap().outcome, Outcome::Completed);
                true
            }
            ended = second.finish() => {
                assert_eq!(ended.unwrap().outcome, Outcome::Completed);
                false
            }
        };
        settle().await;
        assert_eq!(*held.borrow(), 1, "held while a turn still runs");

        // The last ends: it is let go.
        gate.notify_one();
        let last = if first_ended { &mut second } else { &mut first };
        assert_eq!(last.finish().await.unwrap().outcome, Outcome::Completed);
        held.wait_for(|held| *held == 0).await.unwrap();
        assert_eq!(*computer.holds.lock().unwrap(), 1, "held once for both");
    });
}

#[test]
fn a_turn_that_fails_lets_it_go_too() {
    let dir = folder("awake-failed");
    block_on!(async {
        let computer = Arc::new(Computer::default());
        let mut held = computer.held.subscribe();
        // No answer: once let through, the model's script is exhausted and the turn fails.
        let gate = Arc::new(Notify::new());
        let mut setup = Setup::asking(Vec::new(), Answering::NoOne);
        setup.gate = Some(Arc::clone(&gate));
        setup.awake = Some(Arc::clone(&computer) as Arc<dyn KeepAwake>);
        let (server, _) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        let mut failing = stream_turn(&server, thread, "fail", None).await.unwrap();
        held.wait_for(|held| *held == 1).await.unwrap();
        gate.notify_one();
        let ended = failing.finish().await;
        assert!(
            ended.is_err() || ended.is_ok_and(|ended| ended.outcome != Outcome::Completed),
            "the turn fails"
        );
        held.wait_for(|held| *held == 0).await.unwrap();
        assert_eq!(*computer.holds.lock().unwrap(), 1);
    });
}

#[test]
fn a_turn_the_person_stops_lets_it_go_too() {
    let dir = folder("awake-cancelled");
    block_on!(async {
        let computer = Arc::new(Computer::default());
        let mut held = computer.held.subscribe();
        // The model never answers: the turn runs until it is stopped.
        let mut setup = Setup::asking(vec![answer(Some("Slow."), None)], Answering::NoOne);
        setup.gate = Some(Arc::new(Notify::new()));
        setup.awake = Some(Arc::clone(&computer) as Arc<dyn KeepAwake>);
        let (server, _) = setup.build(&dir).await;
        let thread = start(&server, false).await;
        let mut running = stream_turn(&server, thread, "slow", None).await.unwrap();
        held.wait_for(|held| *held == 1).await.unwrap();
        server
            .request(Request::TurnCancel { thread })
            .await
            .unwrap();
        assert_eq!(running.finish().await.unwrap().outcome, Outcome::Cancelled);
        held.wait_for(|held| *held == 0).await.unwrap();
    });
}

/// Whether `server` keeps its computer awake, as Settings asks.
async fn awake<L: EventLog + 'static>(server: &Tested<L>) -> AwakeInfo {
    match server.request(Request::AwakeGet {}).await.unwrap() {
        Reply::Awake { awake } => awake,
        other => panic!("whether it keeps awake, not {other:?}"),
    }
}

#[test]
fn the_person_turns_it_off_and_on_and_no_one_else_does() {
    let dir = folder("awake-chosen");
    block_on!(async {
        let computer = Arc::new(Computer::default());
        let log = Shared::default();
        let mut setup = Setup::asking(Vec::new(), Answering::NoOne);
        setup.awake = Some(Arc::clone(&computer) as Arc<dyn KeepAwake>);
        let (server, _) = setup.build_on(&dir, log.clone()).await;
        assert_eq!(awake(&server).await, AwakeInfo::Chosen { on: true });
        let off = server.request(Request::AwakeSet { on: false }).await;
        assert!(matches!(
            off,
            Ok(Reply::Awake {
                awake: AwakeInfo::Chosen { on: false }
            })
        ));
        assert_eq!(awake(&server).await, AwakeInfo::Chosen { on: false });
        // Someone signed in from elsewhere is not the person on this computer.
        let elsewhere = elsewhere();
        let refused = server
            .request_as(&elsewhere, Request::AwakeSet { on: true })
            .await;
        assert_eq!(
            refused.err().map(|error| error.code),
            Some(code::NOT_ALLOWED)
        );

        // Kept in the person's settings: a server started later on the same store reads it,
        // and its turns leave the computer as it is.
        let mut next = Setup::asking(vec![answer(Some("Hi."), None)], Answering::NoOne);
        next.seed = 2;
        next.settings.person = server.person();
        next.projects = Some(Arc::clone(server.projects()));
        next.awake = Some(Arc::clone(&computer) as Arc<dyn KeepAwake>);
        let (again, _) = next.build_on(&dir, log).await;
        let thread = start(&again, false).await;
        turn(&again, thread, "hi").await.unwrap();
        settle().await;
        assert_eq!(*computer.holds.lock().unwrap(), 0, "never held");
        assert_eq!(awake(&again).await, AwakeInfo::Chosen { on: false });
    });
}

/// Someone signed in from elsewhere asks whether the computer stays awake: not theirs to
/// choose, they are told why, and nothing of the person's is read, so no Chats of theirs is
/// made to read it from.
#[test]
fn someone_elsewhere_is_told_it_is_not_theirs_and_reads_nothing_of_the_person_s() {
    let dir = folder("awake-elsewhere");
    block_on!(async {
        let computer = Arc::new(Computer::default());
        let mut setup = Setup::asking(Vec::new(), Answering::NoOne);
        setup.awake = Some(Arc::clone(&computer) as Arc<dyn KeepAwake>);
        let (server, _) = setup.build(&dir).await;
        let told = server.request_as(&elsewhere(), Request::AwakeGet {}).await;
        let Ok(Reply::Awake {
            awake: AwakeInfo::Never { reason },
        }) = told
        else {
            panic!("not theirs to choose, not {told:?}");
        };
        assert_eq!(
            &*reason,
            "only the person on this computer chooses whether it stays awake"
        );
        let person = Principal::person(server.person().unwrap());
        let theirs = server
            .projects()
            .list(scope().org(), &person, 10)
            .await
            .unwrap();
        assert!(
            theirs.iter().all(|it| it.kind != ProjectKind::Personal),
            "no Chats made for the person here"
        );
    });
}

#[test]
fn a_server_keeps_no_computer_awake() {
    let dir = folder("awake-never");
    block_on!(async {
        let (server, _) = Setup::new(Vec::new()).build(&dir).await;
        let AwakeInfo::Never { reason } = awake(&server).await else {
            panic!("never, on a server");
        };
        assert_eq!(
            &*reason,
            "this glave is a server, which doesn't sleep: there is nothing to keep awake"
        );
        let refused = server.request(Request::AwakeSet { on: true }).await;
        assert_eq!(
            refused.err().map(|error| error.code),
            Some(code::NOT_AVAILABLE)
        );
    });
}
