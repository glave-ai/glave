//! Opening a thread from its end (`thread/turns`): its latest 20 turns whole, with the model and
//! effort it chose before them, and what came before read back on demand, so the pages read
//! back are the thread, each event once.

use glave_event::{ModelRef, RawEvent, Seq, ThreadId};
use glave_ports::Completion;
use glave_session::protocol::{ModelInfo, Reply, Request};

use crate::support::*;

/// A server whose model `hope-12b`, which offers reasoning efforts, answers `turns` turns.
async fn server(dir: &std::path::Path, turns: usize) -> Tested<Shared> {
    let answers: Vec<Completion> = (0..turns)
        .map(|_| {
            let mut completion = answer(Some("Done."), None);
            completion.model = ModelRef {
                provider: "script".into(),
                name: "hope-12b".into(),
            };
            completion
        })
        .collect();
    let mut setup = Setup::new(answers);
    let mut hope = ModelInfo::new("hope-12b");
    hope.reasoning = Some(glave_event::Reasoning {
        efforts: vec!["high".into(), "low".into()],
        default: Some("low".into()),
        mandatory: false,
    });
    setup.models = vec![hope];
    setup.build_on(dir, Shared::default()).await.0
}

/// A thread on `hope-12b` at high effort, chosen as it started, with `turns` turns.
async fn thread_of(server: &Tested<Shared>, turns: usize) -> ThreadId {
    let started = Request::ThreadStart {
        project: Some(scope().project()),
        worktree: None,
        background: false,
        computer: None,
        model: Some("hope-12b".into()),
        permissions: None,
        effort: Some(Some("high".into())),
    };
    let Reply::Thread { thread } = server.request(started).await.unwrap() else {
        panic!("a thread");
    };
    for each in 0..turns {
        turn(server, thread.id, &format!("turn {each}"))
            .await
            .unwrap();
    }
    thread.id
}

/// A page of `thread/turns`: its events and `earlier`.
async fn turns(
    server: &Tested<Shared>,
    thread: ThreadId,
    before: Option<Seq>,
) -> (Vec<RawEvent>, Option<Seq>) {
    let reply = server
        .request(Request::ThreadTurns { thread, before })
        .await
        .unwrap();
    let Reply::Events {
        events,
        next,
        earlier,
    } = reply
    else {
        panic!("events, not {reply:?}");
    };
    assert_eq!(next, None, "read back, a page says nothing of reading on");
    (events, earlier)
}

fn kinds(events: &[RawEvent]) -> Vec<&str> {
    events.iter().map(|event| event.kind.as_str()).collect()
}

#[test]
fn a_long_thread_opens_with_its_latest_turns_and_reads_back_to_its_start() {
    let dir = folder("history-long");
    block_on!(async {
        let server = server(&dir, 25).await;
        let thread = thread_of(&server, 25).await;
        let all = events(&server, thread).await;
        let (opened, earlier) = turns(&server, thread, None).await;
        let earlier = earlier.expect("there is more before the latest turns");
        let (choices, latest): (Vec<RawEvent>, Vec<RawEvent>) =
            opened.into_iter().partition(|event| event.seq < earlier);
        assert_eq!(
            kinds(&choices),
            ["model.chosen", "effort.chosen"],
            "what the thread chose before its latest turns comes with them"
        );
        let starts = kinds(&latest)
            .into_iter()
            .filter(|kind| *kind == "turn.started")
            .count();
        assert_eq!(starts, 20);
        assert_eq!(
            latest.first().map(|event| event.kind.as_str()),
            Some("turn.started")
        );
        assert_eq!(latest.first().map(|event| event.seq), Some(earlier));
        let tail = &all[all.len() - latest.len()..];
        assert_eq!(
            seqs(&latest),
            seqs(tail),
            "the turns run to the thread's end"
        );

        let (before, more) = turns(&server, thread, Some(earlier)).await;
        assert_eq!(more, None, "five turns are all that came before");
        assert_eq!(seqs(&before), seqs(&all[..all.len() - latest.len()]));
    });
}

#[test]
fn a_short_thread_opens_whole() {
    let dir = folder("history-short");
    block_on!(async {
        let server = server(&dir, 3).await;
        let thread = thread_of(&server, 3).await;
        let (opened, earlier) = turns(&server, thread, None).await;
        assert_eq!(earlier, None);
        assert_eq!(seqs(&opened), seqs(&events(&server, thread).await));
    });
}

fn seqs(events: &[RawEvent]) -> Vec<u64> {
    events.iter().map(|event| event.seq.get()).collect()
}
