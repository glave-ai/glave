//! A turn's W3C trace: `_meta.traceparent` on `turn/start` is recorded in `turn.started`, and
//! one that is not a traceparent is ignored rather than refused. Nothing the model sees
//! changes.

#[cfg(test)]
#[path = "support/model.rs"]
#[expect(dead_code, reason = "shared helpers; this file uses some of them")]
mod support;

#[cfg(test)]
mod tracing {
    use glave_event::{Body, TraceId, decode};
    use glave_session::Answering;
    use glave_session::protocol::{Meta, Reply, Request};

    use super::support::*;

    fn turn_with(thread: glave_event::ThreadId, traceparent: &str) -> Request {
        Request::TurnStart {
            background: false,
            budget: None,
            model: None,
            approval: None,
            thread,
            prompt: "go".into(),
            meta: Meta {
                traceparent: Some(traceparent.to_owned()),
                ..Meta::default()
            },
        }
    }

    #[test]
    fn the_caller_s_trace_is_recorded_with_the_turn_and_a_bad_one_ignored() {
        let dir = folder("session-tracing");
        block_on(async {
            let answers = vec![answer(Some("One."), None), answer(Some("Two."), None)];
            let server = server(&dir, answers, Answering::NoOne).await;
            let thread = start(&server, false).await;
            let parent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
            let reply = server.request(turn_with(thread, parent)).await.unwrap();
            assert!(matches!(reply, Reply::Turn { .. }), "{reply:?}");
            let reply = server.request(turn_with(thread, "not-a-trace")).await;
            assert!(matches!(reply, Ok(Reply::Turn { .. })), "{reply:?}");
            let recorded: Vec<Option<TraceId>> = events(&server, thread)
                .await
                .into_iter()
                .filter_map(|raw| match decode(raw).unwrap().body {
                    Body::TurnStarted(started) => Some(started.trace),
                    _ => None,
                })
                .collect();
            let want = TraceId::new("4bf92f3577b34da6a3ce929d0e0e4736").unwrap();
            assert_eq!(recorded, [Some(want), None]);
        });
    }
}
