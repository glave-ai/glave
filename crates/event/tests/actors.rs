//! Who acted: the fields that say who keep their exact shape.

#[cfg(test)]
mod actors {
    use glave_event::{Body, OriginKind, PrincipalKind, RawEvent, ThreadStarted, TraceId, decode};

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
    fn an_unknown_origin_kind_is_kept() {
        let data = r#"{"producer":{"name":"glave","version":"0.0.0"},"origin":{"kind":"sensor"}}"#;
        assert_eq!(
            started(1, data).origin.kind,
            OriginKind::Other("sensor".into())
        );
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
