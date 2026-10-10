//! Who acted: older events read as the newer versions with their meaning unchanged, and the
//! new fields keep their exact shape.

#[cfg(test)]
mod actors {
    use glave_event::{
        ApprovalDecided, Body, MessageAdded, OriginKind, Principal, PrincipalKind, RawEvent,
        ThreadStarted, TraceId, TurnStarted, Verdict, decode,
    };

    const THREAD: &str = "0192f5c0-11aa-7e21-8c3d-5f9b2e4a6c10";

    fn event(kind: &str, v: u32, data: &str) -> Body {
        let json = format!(
            r#"{{"glave":1,"id":"0192f5c2-7000-7000-8000-000000000001","thread":"{THREAD}","seq":1,"at":"2026-10-10T09:00:00.000Z","type":"{kind}","v":{v},"data":{data}}}"#
        );
        decode(RawEvent::from_json(&json).unwrap()).unwrap().body
    }

    fn started(v: u32, data: &str) -> ThreadStarted {
        match event("thread.started", v, data) {
            Body::ThreadStarted(started) => started,
            other => panic!("decoded as {other:?}"),
        }
    }

    #[test]
    fn an_older_thread_reads_as_the_local_persons_with_its_origin_worked_out() {
        let producer = r#""producer":{"name":"glave","version":"0.0.0"}"#;
        let plain = started(1, &format!("{{{producer}}}"));
        assert_eq!(plain.for_, None);
        assert_eq!(plain.agent, None);
        assert_eq!(plain.origin.kind, OriginKind::Prompt);

        let child = started(
            2,
            &format!(r#"{{{producer},"parent":{{"thread":"{THREAD}","call_id":"call_7"}}}}"#),
        );
        assert_eq!(child.origin.kind, OriginKind::Subagent);
        assert!(child.parent.is_some(), "the parent link is kept");

        let fork = started(
            2,
            &format!(r#"{{{producer},"fork":{{"thread":"{THREAD}","seq":4}}}}"#),
        );
        assert_eq!(fork.origin.kind, OriginKind::Fork);
    }

    #[test]
    fn an_unknown_origin_kind_is_kept() {
        let data = r#"{"producer":{"name":"glave","version":"0.0.0"},"origin":{"kind":"sensor"}}"#;
        assert_eq!(
            started(3, data).origin.kind,
            OriginKind::Other("sensor".into())
        );
    }

    #[test]
    fn an_older_decision_by_no_one_is_glaves_and_one_by_the_person_names_no_one() {
        let decided = |verdict: &str| match event(
            "approval.decided",
            1,
            &format!(r#"{{"call_id":"call_9","decision":"{verdict}"}}"#),
        ) {
            Body::ApprovalDecided(ApprovalDecided { by, decision, .. }) => (decision, by),
            other => panic!("decoded as {other:?}"),
        };
        assert_eq!(
            decided("no_one_there"),
            (Verdict::NoOneThere, Some(Principal::glave()))
        );
        assert_eq!(
            decided("cancelled"),
            (Verdict::Cancelled, Some(Principal::glave()))
        );
        assert_eq!(decided("allow_once"), (Verdict::AllowOnce, None));
    }

    #[test]
    fn an_older_message_and_turn_read_with_no_author_trace_or_sign_in() {
        let Body::MessageAdded(MessageAdded { author, .. }) = event(
            "message.added",
            1,
            r#"{"role":"user","parts":[{"type":"text","text":"hi"}]}"#,
        ) else {
            panic!("not a message");
        };
        assert_eq!(author, None);
        let config = r#"{"sha256":"9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08","bytes":1,"media_type":"application/json"}"#;
        let Body::TurnStarted(TurnStarted { trace, auth, .. }) = event(
            "turn.started",
            1,
            &format!(r#"{{"model":{{"provider":"p","name":"m"}},"config":{config}}}"#),
        ) else {
            panic!("not a turn");
        };
        assert_eq!((trace, auth), (None, None));
    }

    #[test]
    fn a_trace_id_is_32_lowercase_hex_digits_not_all_zero() {
        assert!(TraceId::new("4bf92f3577b34da6a3ce929d0e0e4736").is_ok());
        for bad in [
            "",
            "4BF92F3577B34DA6A3CE929D0E0E4736",
            "4bf92f3577b34da6a3ce929d0e0e473",
            "00000000000000000000000000000000",
            "4bf92f3577b34da6a3ce929d0e0e473g",
        ] {
            assert!(TraceId::new(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_principal_kind_round_trips_through_its_text() {
        for kind in ["person", "agent", "group", "glave"] {
            assert_eq!(PrincipalKind::from(kind.to_owned()).as_str(), kind);
        }
    }
}
