//! The gateway's span and metrics for each model call (`tracing`): the route, the target, the
//! tokens, the cost and how it failed. Never the request or the answer. Metrics are events
//! whose fields follow tracing-opentelemetry's prefixes, so the same event is a log line
//! locally and a metric in OpenTelemetry.

use glave_ports::{Completion, ProviderError};
use tracing::{Span, field};

/// The span of one call on the route `route` to the target `target`.
pub(crate) fn model_call(route: &str, target: &str) -> Span {
    tracing::info_span!(
        "model.call",
        route,
        model = target,
        input_tokens = field::Empty,
        output_tokens = field::Empty,
        failure = field::Empty,
    )
}

/// Records how a call ended on its span, and counts it, its tokens and its cost.
pub(crate) fn model_ended(
    span: &Span,
    route: &str,
    target: &str,
    sent: &Result<Completion, ProviderError>,
) {
    match sent {
        Ok(completion) => {
            let counts = completion.usage.counts();
            span.record("input_tokens", counts.input);
            span.record("output_tokens", counts.output);
            let cost_micros = completion
                .cost
                .as_ref()
                .and_then(|cost| u64::try_from(cost.amount.micros).ok())
                .unwrap_or(0);
            tracing::info!(
                parent: span,
                monotonic_counter.glave.model.calls = 1_u64,
                monotonic_counter.glave.model.input_tokens = counts.input,
                monotonic_counter.glave.model.output_tokens = counts.output,
                monotonic_counter.glave.model.cost_micros = cost_micros,
                route,
                model = target,
                "model call ended"
            );
        }
        Err(error) => {
            let failure = format!("{:?}", error.failure());
            span.record("failure", failure.as_str());
            tracing::info!(
                parent: span,
                monotonic_counter.glave.model.failures = 1_u64,
                route,
                model = target,
                failure = failure.as_str(),
                "model call failed"
            );
        }
    }
}
