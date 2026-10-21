use std::future::poll_fn;

use super::*;
use crate::hub::Hub;
use crate::protocol::Update;
use crate::watch::tests::{event, thread};
use crate::watch::{Reader, Watch};

/// The body's next frame as text, if it has one ready now.
fn ready(body: &mut Answer) -> Option<String> {
    let waker = std::task::Waker::noop();
    let mut cx = Context::from_waker(waker);
    match Pin::new(body).poll_frame(&mut cx) {
        Poll::Ready(Some(Ok(frame))) => {
            let data = frame.into_data().unwrap();
            Some(String::from_utf8(data.to_vec()).unwrap())
        }
        Poll::Ready(None) => Some("(end)".into()),
        Poll::Pending => None,
    }
}

/// The body's next frame as text, waiting for it.
async fn next(body: &mut Answer) -> String {
    let frame = poll_fn(|cx| Pin::new(&mut *body).poll_frame(cx)).await;
    let data = frame.unwrap().unwrap().into_data().unwrap();
    String::from_utf8(data.to_vec()).unwrap()
}

#[test]
fn a_watchs_events_carry_their_seq_and_a_quiet_stream_a_comment_every_15_seconds() {
    glave_ports::block_on!(async {
        tokio::time::pause();
        let hub = Hub::default();
        let watched = thread(1);
        let read: Reader = Arc::new(|_, _| Box::pin(async { Ok(Vec::new()) }));
        let watch = Watch::new(watched, read, hub.watch(watched), None);
        let mut body = Answer::stream(
            Streamed::Watch(watch),
            Some(Id::Number(7)),
            Language::English,
        );
        assert_eq!(ready(&mut body), None, "nothing to say yet");
        tokio::time::advance(KEEP_ALIVE.checked_sub(Duration::from_millis(1)).unwrap()).await;
        assert_eq!(ready(&mut body), None, "not quiet for long enough");
        tokio::time::advance(Duration::from_millis(1)).await;
        assert_eq!(next(&mut body).await, ": keep-alive\n\n");

        let recorded = event(watched, 1, false);
        let thread = recorded.thread;
        hub.publish(
            thread,
            Notification::Event {
                thread,
                event: recorded,
            },
        );
        let sent = next(&mut body).await;
        assert!(sent.starts_with("id: 1\ndata: {"), "{sent}");
        let update = Update::Text { text: "hi".into() };
        hub.publish(thread, Notification::Live { thread, update });
        let sent = next(&mut body).await;
        assert!(
            sent.starts_with("data: {"),
            "a live update has no id: {sent}"
        );
        // Each thing sent puts the next comment off again.
        tokio::time::advance(KEEP_ALIVE.checked_sub(Duration::from_millis(1)).unwrap()).await;
        assert_eq!(ready(&mut body), None);

        let deleted = event(watched, 2, true);
        hub.publish(
            thread,
            Notification::Event {
                thread,
                event: deleted,
            },
        );
        assert!(next(&mut body).await.starts_with("id: 2\n"));
        let last = next(&mut body).await;
        assert_eq!(
            last,
            "data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ended\":\"deleted\"}}\n\n"
        );
        assert_eq!(ready(&mut body).as_deref(), Some("(end)"));
    });
}

#[test]
fn a_streams_last_answer_is_in_the_callers_language() {
    glave_ports::block_on!(async {
        let hub = Hub::default();
        let watched = thread(1);
        let read: Reader = Arc::new(|_, _| Box::pin(async { Ok(Vec::new()) }));
        let watch = Watch::new(watched, read, hub.watch(watched), None);
        let mut body = Answer::stream(
            Streamed::Watch(watch),
            Some(Id::Number(7)),
            Language::Accented,
        );
        // The server goes, and the watch ends with its own sentence.
        drop(hub);
        let last = next(&mut body).await;
        assert!(
            last.contains("\"message\":\"[ţĥé šéŕṽéŕ šţöþþéð one"),
            "{last}"
        );
    });
}
