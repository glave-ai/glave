//! Tracing a request (`specs/session.md`, "Tracing"): each request runs in a content-free
//! `session.request` span named by its method, and a turn's caller may send the W3C trace it
//! belongs to in `_meta.traceparent`. The trace is recorded with the turn (`turn.started`), and
//! with the `otel` feature the request's span joins that trace, so a client's spans, glave's
//! and the events line up. A `traceparent` that is not one is ignored, as W3C Trace Context
//! says, rather than refused.

use glave_event::TraceId;
use tracing::Span;

use crate::protocol::Request;

/// A parsed `traceparent`: `00-<trace id>-<parent id>-<flags>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Traceparent {
    pub trace: TraceId,
    /// The caller's span: 16 lowercase hex digits, not all zero.
    pub parent: String,
    /// Whether the caller sampled the trace.
    pub sampled: bool,
}

impl Traceparent {
    /// `text` as a version-00 `traceparent`, or nothing when it is not one.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().split('-');
        let (version, trace, parent, flags) =
            (parts.next()?, parts.next()?, parts.next()?, parts.next()?);
        let hex = |part: &str, len: usize| {
            part.len() == len
                && part.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                && part.bytes().any(|b| b != b'0')
        };
        let flags_hex = flags.len() == 2 && flags.bytes().all(|b| b.is_ascii_hexdigit());
        if parts.next().is_some() || version != "00" || !hex(parent, 16) || !flags_hex {
            return None;
        }
        let flags = u8::from_str_radix(flags, 16).ok()?;
        Some(Self {
            trace: TraceId::new(trace).ok()?,
            parent: parent.to_owned(),
            sampled: flags & 1 == 1,
        })
    }
}

/// The caller's trace for `request`: its `_meta.traceparent`, when it sends one.
pub(crate) fn traceparent(request: &Request) -> Option<Traceparent> {
    match request {
        Request::TurnStart { meta, .. } => meta.traceparent.as_deref().and_then(Traceparent::parse),
        _ => None,
    }
}

/// The span `request` runs in: its method, and the caller's trace when it sent one. Never the
/// request's params.
pub(crate) fn span(request: &Request) -> Span {
    let parent = traceparent(request);
    let span = tracing::info_span!(
        "session.request",
        method = request.method(),
        trace_id = parent.as_ref().map(|parent| parent.trace.as_str()),
    );
    #[cfg(feature = "otel")]
    if let Some(parent) = &parent {
        otel::join(&span, parent);
    }
    span
}

/// The trace a turn started by `request` belongs to: the caller's, or, with the `otel`
/// feature and a tracer running, the one its span is in.
pub(crate) fn trace_of(request: &Request) -> Option<TraceId> {
    let sent = traceparent(request).map(|parent| parent.trace);
    #[cfg(feature = "otel")]
    let sent = sent.or_else(otel::current);
    sent
}

#[cfg(feature = "otel")]
mod otel {
    use glave_event::TraceId;
    use opentelemetry::trace::{
        SpanContext, SpanId, TraceContextExt as _, TraceFlags, TraceId as OtelTraceId, TraceState,
    };
    use tracing::Span;
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;

    use super::Traceparent;

    /// Makes `span` a child of the caller's span in `parent`'s trace.
    pub(super) fn join(span: &Span, parent: &Traceparent) {
        let (Ok(trace), Ok(id)) = (
            OtelTraceId::from_hex(parent.trace.as_str()),
            SpanId::from_hex(&parent.parent),
        ) else {
            return;
        };
        let flags = if parent.sampled {
            TraceFlags::SAMPLED
        } else {
            TraceFlags::default()
        };
        let remote = SpanContext::new(trace, id, flags, true, TraceState::default());
        let context = opentelemetry::Context::new().with_remote_span_context(remote);
        let _ = span.set_parent(context);
    }

    /// The trace the current span is in, when a tracer is running.
    pub(super) fn current() -> Option<TraceId> {
        let context = Span::current().context();
        let span = context.span();
        let trace = span.span_context().trace_id();
        (trace != OtelTraceId::INVALID)
            .then(|| TraceId::new(trace.to_string()).ok())
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    proptest! {
        #[test]
        fn any_text_parses_or_is_ignored(text in ".{0,80}") {
            let _ = Traceparent::parse(&text);
        }

        #[test]
        fn a_well_formed_traceparent_reads_back(
            trace in "[0-9a-f]{31}[1-9a-f]",
            parent in "[1-9a-f][0-9a-f]{15}",
            flags in 0_u8..=255,
        ) {
            let text = format!("00-{trace}-{parent}-{flags:02x}");
            let parsed = Traceparent::parse(&text).unwrap();
            prop_assert_eq!(parsed.trace.as_str(), trace.as_str());
            prop_assert_eq!(parsed.parent, parent);
            prop_assert_eq!(parsed.sampled, flags & 1 == 1);
        }
    }

    #[test]
    fn a_traceparent_is_read_and_anything_else_ignored() {
        let parsed =
            Traceparent::parse("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01").unwrap();
        assert_eq!(parsed.trace.as_str(), "4bf92f3577b34da6a3ce929d0e0e4736");
        assert_eq!(parsed.parent, "00f067aa0ba902b7");
        assert!(parsed.sampled);
        let unsampled =
            Traceparent::parse("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00").unwrap();
        assert!(!unsampled.sampled);
        for bad in [
            "",
            "00",
            "01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
            "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
            "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-1",
            "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra",
        ] {
            assert_eq!(Traceparent::parse(bad), None, "{bad:?}");
        }
    }
}
