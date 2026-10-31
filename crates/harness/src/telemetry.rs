//! The harness's spans and metrics (`tracing`): a turn, a tool call, a command on a computer,
//! and approvals. Content-free by construction: ids, names, counts, durations and outcomes,
//! never a prompt, an answer, arguments, output or a file. Metrics are events whose fields
//! follow tracing-opentelemetry's prefixes (`monotonic_counter.`, `histogram.`), so the same
//! event is a log line locally and a metric in OpenTelemetry. With no subscriber, all of it
//! costs a branch.

use std::time::Instant;

use glave_event::{Outcome, ThreadId, TraceId, Verdict};
use glave_ports::Called;
use glave_ports::computer::Ending;
use tracing::{Instrument as _, Span, field};

/// The span of one turn in `thread`, in `trace` when the caller sent one.
pub(crate) fn turn(thread: ThreadId, trace: Option<&TraceId>) -> Span {
    tracing::info_span!(
        "turn",
        thread = %thread,
        trace_id = trace.map(TraceId::as_str),
        outcome = field::Empty,
        model_calls = field::Empty,
        tool_calls = field::Empty,
    )
}

/// Records how a turn that began at `began` ended, on its span and as metrics.
pub(crate) fn turn_ended(span: &Span, began: Instant, ended: Option<(Outcome, u32, u32)>) {
    let seconds = began.elapsed().as_secs_f64();
    let outcome = match ended {
        Some((outcome, model_calls, tool_calls)) => {
            span.record("outcome", field::debug(outcome));
            span.record("model_calls", model_calls);
            span.record("tool_calls", tool_calls);
            format!("{outcome:?}")
        }
        None => "Error".to_owned(),
    };
    tracing::info!(
        parent: span,
        monotonic_counter.glave.turns = 1_u64,
        histogram.glave.turn.seconds = seconds,
        outcome = outcome.as_str(),
        "turn ended"
    );
}

/// Runs `call`, a call of the tool `tool`, in its span, and counts it.
pub(crate) async fn tool_call(tool: &str, call: impl Future<Output = Called>) -> Called {
    let span = tracing::info_span!("tool.call", tool, is_error = field::Empty);
    let called = call.instrument(span.clone()).await;
    // One that stops at what its service asks ends when it goes on.
    if let Called::Done(output) = &called {
        tool_ended(&span, tool, output.is_error);
    }
    called
}

/// Records how a call of `tool` ended, on its span and as metrics.
fn tool_ended(span: &Span, tool: &str, is_error: bool) {
    span.record("is_error", is_error);
    tracing::info!(
        parent: span,
        monotonic_counter.glave.tool.calls = 1_u64,
        tool,
        is_error,
        "tool call ended"
    );
    if is_error {
        tracing::info!(
            parent: span,
            monotonic_counter.glave.tool.errors = 1_u64,
            tool,
            "tool call failed"
        );
    }
}

/// Counts an approval and how it was decided.
pub(crate) fn approval(tool: &str, verdict: Verdict) {
    tracing::info!(
        monotonic_counter.glave.approvals = 1_u64,
        tool,
        decision = ?verdict,
        "approval decided"
    );
}

/// The span of one command on a computer: never the command itself.
pub(crate) fn command() -> Span {
    tracing::info_span!(
        "computer.command",
        ending = field::Empty,
        exit_code = field::Empty,
        millis = field::Empty,
    )
}

/// Records how a command ended on its span.
pub(crate) fn command_ended(span: &Span, ending: Option<(&Ending, u64)>) {
    let Some((ending, millis)) = ending else {
        span.record("ending", "failed");
        return;
    };
    span.record("millis", millis);
    match ending {
        Ending::Exited { code } => {
            span.record("ending", "exited");
            span.record("exit_code", code);
        }
        Ending::Signalled => {
            span.record("ending", "signalled");
        }
        Ending::TimedOut { .. } => {
            span.record("ending", "timed_out");
        }
        Ending::Cancelled => {
            span.record("ending", "cancelled");
        }
        Ending::Backgrounded { .. } => {
            span.record("ending", "backgrounded");
        }
        Ending::Idle { .. } => {
            span.record("ending", "idle");
        }
    }
}
