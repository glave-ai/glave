//! What a caller follows ends the moment their access does (#546): a thread's watch, a turn's
//! stream and a list's watch, followed by a browser whose sign-in ends while they wait, end at
//! once with "no sign-in", and nothing more is sent after. A project has one owner, who keeps
//! it, so a browser's sign-in is the access that can end while they follow.

use std::sync::Arc;

use glave_event::{Auth, AuthMethod, ThreadId};
use glave_ports::{Caller, Completion, InMemoryLog};
use glave_session::protocol::{ApiError, Notification, Reply, Request, Update, code};
use glave_session::signin::Browsers;
use glave_session::{Answering, Sent, TurnItem, TurnStream, Watched};
use serde_json::json;

use crate::support::*;

/// A server signing browsers in, a browser signed in to it, and that browser as a caller.
async fn signed_in(dir: &std::path::Path, answers: Vec<Completion>) -> (Tested, Caller) {
    let clock = glave_ports::testing::ManualClock::at_second(1_791_001_000);
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
    };
    (server, browser)
}

/// How an ended sign-in ends what was followed.
fn no_sign_in(ended: &Result<impl std::fmt::Debug, ApiError>) {
    let refused = ended.as_ref().err();
    assert_eq!(
        refused.map(|refused| refused.code),
        Some(code::NO_SIGN_IN),
        "{ended:?}"
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
            panic!("the watch ended, not {came:?}");
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
            panic!("the stream ended, not {came:?}");
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
        let mut list = server.watch_list(&browser, None, None).await.unwrap();
        let (came, signed_out) = tokio::join!(
            list.next(),
            server.request_as(&browser, Request::SigninEnd {})
        );
        signed_out.unwrap();
        let Some(Watched::Ended(ended)) = came else {
            panic!("the watch ended, not {came:?}");
        };
        no_sign_in(&ended);
        start(&server, false).await;
        assert!(list.next().await.is_none(), "nothing more after");
    });
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
