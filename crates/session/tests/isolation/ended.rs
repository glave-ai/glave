//! What a caller follows ends the moment their access does (#546): a thread's watch, a turn's
//! stream and a list's watch, followed by a browser whose sign-in ends while they wait, or by
//! someone whose company login's token expires (#1504), end at once with "no sign-in", and
//! nothing more is sent after. A project has one owner, who keeps it, so a sign-in is the
//! access that can end while they follow.

use std::sync::Arc;
use std::time::Duration;

use glave_event::{At, Auth, AuthMethod, ThreadId};
use glave_ports::testing::ManualClock;
use glave_ports::{Caller, Completion, InMemoryLog};
use glave_session::protocol::{ApiError, Notification, Reply, Request, Update, code};
use glave_session::signin::Browsers;
use glave_session::{Answering, ListWatch, Sent, TurnItem, TurnStream, Watched};
use jiff::Timestamp;
use serde_json::json;

use crate::support::*;

/// A server signing browsers in, a browser signed in to it, and that browser as a caller.
async fn signed_in(dir: &std::path::Path, answers: Vec<Completion>) -> (Tested, Caller) {
    let clock = ManualClock::at_second(1_791_001_000);
    let opened = Browsers::open(
        InMemoryLog::new(),
        scope(),
        "server".to_owned(),
        uuid::Uuid::from_u128(0x691),
        None,
        Box::new(clock),
    );
    let browsers = Arc::new(opened.await.unwrap());
    let mut setup = Setup::asking(answers, Answering::Clients);
    setup.browsers = Some(Arc::clone(&browsers));
    let (server, _) = setup.build(dir).await;
    let Ok(Reply::Link { code }) = server.request(Request::SigninLink {}).await else {
        panic!("a link");
    };
    let session = browsers.redeem(code.expose()).await.unwrap();
    let browser = Caller {
        person: None,
        auth: Some(Auth {
            method: AuthMethod::Local,
            issuer: None,
            subject: None,
            session: Some(session.id),
        }),
        trace: None,
        profile: None,
        org: None,
        from: None,
        until: None,
    };
    (server, browser)
}

/// How an ended sign-in ends what was followed.
fn no_sign_in<T>(ended: &Result<T, ApiError>) {
    let refused = ended.as_ref().err();
    assert_eq!(
        refused.map(|refused| refused.code),
        Some(code::NO_SIGN_IN),
        "what was followed ends as an ended sign-in"
    );
}

/// The `seq` of `thread`'s latest event.
async fn latest(server: &Tested, thread: ThreadId) -> u64 {
    let logged = events(server, thread).await;
    logged.last().map_or(0, |event| event.seq.get())
}

#[test]
fn a_thread_watch_ends_when_its_browser_signs_out() {
    block_on!(async {
        let dir = folder("ended-watch");
        let (server, browser) = signed_in(&dir, Vec::new()).await;
        let thread = start(&server, false).await;
        let after = glave_event::Seq::new(latest(&server, thread).await).ok();
        let mut watch = server.watch(&browser, thread, after).await.unwrap();
        // The watch waits for more when the browser signs out.
        let (came, signed_out) = tokio::join!(
            watch.next(),
            server.request_as(&browser, Request::SigninEnd {})
        );
        signed_out.unwrap();
        let Some(Watched::Ended(ended)) = came else {
            panic!("the watch went on, or ended some other way");
        };
        no_sign_in(&ended);
        let renamed = Request::ThreadRename {
            thread,
            title: "after".to_owned(),
        };
        server.request(renamed).await.unwrap();
        assert!(watch.next().await.is_none(), "nothing more after");
        // A watch begun after is refused as any request is.
        let again = server.watch(&browser, thread, None).await.err();
        assert_eq!(again.map(|refused| refused.code), Some(code::NO_SIGN_IN));
    });
}

#[test]
fn a_turn_stream_ends_when_its_browser_is_signed_out_elsewhere() {
    block_on!(async {
        let dir = folder("ended-turn");
        let asks = answer(None, Some(("bash", json!({"command": "touch made.txt"}))));
        let (server, browser) = signed_in(&dir, vec![asks]).await;
        let thread = start(&server, false).await;
        let request = turn_start(thread, "make it", None);
        let Ok(Sent::Started(mut stream)) = server.start_streaming(&browser, request).await else {
            panic!("a turn");
        };
        waiting(&mut stream).await;
        // The person on the computer signs every browser out.
        let everyone = Request::SigninEndOthers {};
        let (came, signed_out) = tokio::join!(stream.next(), server.request(everyone));
        signed_out.unwrap();
        let Some(TurnItem::Ended(ended)) = came else {
            panic!("the stream went on, or ended some other way");
        };
        no_sign_in(&ended);
        assert!(stream.next().await.is_none(), "nothing more after");
        // The turn itself goes on, the person's to answer or cancel.
        server
            .request(Request::TurnCancel { thread })
            .await
            .unwrap();
    });
}

#[test]
fn a_list_watch_ends_when_its_browser_signs_out() {
    block_on!(async {
        let dir = folder("ended-list");
        let (server, browser) = signed_in(&dir, Vec::new()).await;
        let mut list = server
            .watch_list(&browser, None, None, false)
            .await
            .unwrap();
        let (came, signed_out) = tokio::join!(
            list.next(),
            server.request_as(&browser, Request::SigninEnd {})
        );
        signed_out.unwrap();
        let Some(Watched::Ended(ended)) = came else {
            panic!("the watch went on, or ended some other way");
        };
        no_sign_in(&ended);
        start(&server, false).await;
        assert!(list.next().await.is_none(), "nothing more after");
    });
}

/// A watch on one server of a store ends within moments when its browser is signed out
/// through another, which tells this one nothing: it reads the browser's sign-ins again
/// (#1485).
#[test]
fn a_watch_ends_soon_after_its_browser_is_signed_out_through_another_server() {
    block_on!(async {
        let dir = folder("ended-elsewhere");
        let clock = ManualClock::at_second(1_791_001_000);
        let root = glave_keys::InMemoryRoot::new().unwrap();
        let store =
            glave_store_sqlite::Store::open_in_memory(glave_keys::KeySettings::new(Arc::new(root)))
                .unwrap();
        let open = |log| {
            Browsers::open(
                log,
                scope(),
                "server".to_owned(),
                uuid::Uuid::from_u128(0x691),
                None,
                Box::new(clock.clone()),
            )
        };
        let here = Arc::new(open(store.log()).await.unwrap());
        let other = open(store.log()).await.unwrap();
        let mut setup = Setup::asking(Vec::new(), Answering::Clients);
        setup.browsers = Some(Arc::clone(&here));
        let (server, _) = setup.build(&dir).await;
        let Ok(Reply::Link { code }) = server.request(Request::SigninLink {}).await else {
            panic!("a link");
        };
        let session = here.redeem(code.expose()).await.unwrap();
        let browser = Caller {
            auth: Some(Auth {
                method: AuthMethod::Local,
                issuer: None,
                subject: None,
                session: Some(session.id.clone()),
            }),
            ..Caller::default()
        };
        let mut list = server
            .watch_list(&browser, None, None, false)
            .await
            .unwrap();
        tokio::time::pause();
        // Signed out through the other server, which this one never hears of.
        other.end(&session.id).await.unwrap();
        let mut next = std::pin::pin!(list.next());
        tokio::select! {
            biased;
            _ = &mut next => panic!("the watch ended before the store was read again"),
            () = tokio::task::yield_now() => {}
        }
        clock.advance_millis(30_000);
        let Ok(Some(Watched::Ended(ended))) = tokio::time::timeout(IDLE, next).await else {
            panic!("the watch went on after its browser was signed out elsewhere");
        };
        no_sign_in(&ended);
    });
}

/// How long the server takes a company login's token in these tests, in milliseconds: less than
/// a thread's lease, so the turn it starts keeps its own.
const TAKEN: i64 = 20_000;

/// A server people sign in to with a company login, its clock the test's.
async fn company(dir: &std::path::Path, answers: Vec<Completion>) -> (Tested, ManualClock) {
    let mut setup = Setup::asking(answers, Answering::Clients);
    setup.identity_provider = true;
    let clock = setup.clock.clone();
    let (server, _) = setup.build(dir).await;
    (server, clock)
}

/// Someone signed in with a company login's token that the server takes for [`TAKEN`] more.
fn with_token(clock: &ManualClock) -> Caller {
    let until = Timestamp::from_millisecond(clock.millis() + TAKEN).unwrap();
    Caller {
        until: Some(until),
        ..elsewhere()
    }
}

/// A thread `caller` starts in their Chats.
async fn started_by(server: &Tested, caller: &Caller) -> ThreadId {
    let started = json!({"method": "thread/start", "params": {}});
    let started = serde_json::from_value(started).unwrap();
    match server.request_as(caller, started).await.unwrap() {
        Reply::Thread { thread } => thread.id,
        other => panic!("a thread, not {other:?}"),
    }
}

#[test]
fn a_turn_stream_and_a_thread_watch_end_when_their_token_expires() {
    block_on!(async {
        let dir = folder("ended-token");
        let (server, clock) = company(&dir, vec![answer(Some("Hello."), None)]).await;
        let grace = with_token(&clock);
        let thread = started_by(&server, &grace).await;
        let mut watch = server.watch(&grace, thread, None).await.unwrap();
        let request = turn_start(thread, "hi", None);
        let Ok(Sent::Started(mut stream)) = server.start_streaming(&grace, request).await else {
            panic!("a turn");
        };
        // Both send while the token is taken, and nothing once it has expired, though the turn
        // goes on.
        assert!(matches!(watch.next().await, Some(Watched::Note(_))));
        assert!(matches!(stream.next().await, Some(TurnItem::Note(_))));
        clock.advance_millis(TAKEN);
        let came = stream.next().await;
        let Some(TurnItem::Ended(ended)) = came else {
            panic!("the stream ended, not {came:?}");
        };
        no_sign_in(&ended);
        assert!(stream.next().await.is_none(), "nothing more after");
        let came = watch.next().await;
        let Some(Watched::Ended(ended)) = came else {
            panic!("the watch ended, not {came:?}");
        };
        no_sign_in(&ended);
        assert!(watch.next().await.is_none(), "nothing more after");
        // A watch begun after is refused, as a request with the token is.
        let again = server.watch(&grace, thread, None).await.err();
        assert_eq!(again.map(|refused| refused.code), Some(code::NO_SIGN_IN));
    });
}

#[test]
fn a_list_watch_ends_when_its_token_expires() {
    block_on!(async {
        let dir = folder("ended-token-list");
        let (server, clock) = company(&dir, Vec::new()).await;
        let grace = with_token(&clock);
        let mut list = server.watch_list(&grace, None, None, false).await.unwrap();
        clock.advance_millis(TAKEN);
        // Something to tell them of, after.
        started_by(&server, &elsewhere()).await;
        let came = list.next().await;
        let Some(Watched::Ended(ended)) = came else {
            panic!("the watch ended, not {came:?}");
        };
        no_sign_in(&ended);
        assert!(list.next().await.is_none(), "nothing more after");
    });
}

/// The longest these tests wait, on the runtime's paused time, which passes at once, for an
/// idle watch to wake at its token's end.
const IDLE: Duration = Duration::from_secs(3_600);

/// Watches waiting with nothing to send still end when their token expires: each wakes for it
/// on the runtime's time, and finds it past on the server's clock.
#[test]
fn idle_watches_end_when_their_token_expires() {
    block_on!(async {
        let dir = folder("ended-token-idle");
        let (server, clock) = company(&dir, Vec::new()).await;
        let grace = with_token(&clock);
        let thread = started_by(&server, &grace).await;
        let logged = events_as(&server, &grace, thread).await;
        let after = logged.last().map(|event| event.seq);
        let mut watch = server.watch(&grace, thread, after).await.unwrap();
        let mut list = server.watch_list(&grace, None, None, false).await.unwrap();
        // Whatever starting the thread set going is done first: from here nothing happens.
        for _ in 0..1_000 {
            tokio::task::yield_now().await;
        }
        tokio::time::pause();
        // The thread's watch waits for something to send, as the list's does already, while
        // the token is still taken.
        let mut next = std::pin::pin!(watch.next());
        tokio::select! {
            biased;
            _ = &mut next => panic!("the watch sent with nothing to send"),
            () = tokio::task::yield_now() => {}
        }
        clock.advance_millis(TAKEN);
        let Ok(Some(Watched::Ended(ended))) = tokio::time::timeout(IDLE, next).await else {
            panic!("the waiting watch did not end at its token's end");
        };
        no_sign_in(&ended);
        let Ok(Some(Watched::Ended(ended))) = tokio::time::timeout(IDLE, list.next()).await else {
            panic!("the waiting list watch did not end at its token's end");
        };
        no_sign_in(&ended);
    });
}

/// A list watch sending a page of what changed stops at its token's end between one thread's
/// summary and the next, not once the page is sent: what it sends after is only what it began
/// sending before.
#[test]
fn a_list_watch_sends_nothing_begun_after_its_token_expires() {
    block_on!(async {
        let dir = folder("ended-token-page");
        let (server, clock) = company(&dir, Vec::new()).await;
        let grace = with_token(&clock);
        let since = At::from(Timestamp::from_millisecond(clock.millis()).unwrap());
        for _ in 0..4 {
            started_by(&server, &grace).await;
        }
        // From before the four: a page of four to send, one held for the client at a time. It
        // sends the first and waits to send the second.
        let mut list = server
            .watch_list(&grace, None, Some(since), false)
            .await
            .unwrap();
        for _ in 0..1_000 {
            tokio::task::yield_now().await;
        }
        clock.advance_millis(TAKEN);
        let sent = sent_until_ended(&mut list).await;
        assert!(
            sent <= 2,
            "{sent} sent: one held, and one begun before its end"
        );
    });
}

/// A list watch sending a page of what changed stops when its browser signs out between one
/// thread's summary and the next, not once the page is sent.
#[test]
fn a_list_watch_sends_nothing_begun_after_its_browser_signs_out() {
    block_on!(async {
        let dir = folder("ended-signout-page");
        let (server, browser) = signed_in(&dir, Vec::new()).await;
        for _ in 0..4 {
            start(&server, false).await;
        }
        // From the start: a page of every thread to send, one held for the client at a time.
        let since = At::from(Timestamp::UNIX_EPOCH);
        let mut list = server
            .watch_list(&browser, None, Some(since), false)
            .await
            .unwrap();
        for _ in 0..1_000 {
            tokio::task::yield_now().await;
        }
        server
            .request_as(&browser, Request::SigninEnd {})
            .await
            .unwrap();
        let sent = sent_until_ended(&mut list).await;
        assert!(
            sent <= 2,
            "{sent} sent: one held, and one begun before the sign-out"
        );
    });
}

/// How many summaries `list` sends before it ends, as an ended sign-in ends it.
async fn sent_until_ended(list: &mut ListWatch) -> usize {
    let mut sent = 0;
    loop {
        match list.next().await {
            Some(Watched::Note(_)) => sent += 1,
            Some(Watched::Ended(ended)) => {
                no_sign_in(&ended);
                return sent;
            }
            None => panic!("the list watch ended without saying why"),
        }
    }
}

/// Follows `stream` until its turn waits for an approval.
async fn waiting(stream: &mut TurnStream) {
    while let Some(item) = stream.next().await {
        if let TurnItem::Note(note) = item
            && let Notification::Live {
                update: Update::ApprovalPending { .. },
                ..
            } = note.as_ref()
        {
            return;
        }
    }
    panic!("the turn ended without waiting for an approval");
}
