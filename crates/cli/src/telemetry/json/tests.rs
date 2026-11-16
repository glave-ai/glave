use std::io;
use std::sync::{Arc, Mutex};

use glave_ports::testing::ManualClock;
use serde_json::{Value, json};
use tracing::field;
use tracing_subscriber::layer::SubscriberExt as _;

use super::layer;

/// What the clock reads: 2023-11-14T22:13:20.123Z.
const MILLIS: i64 = 1_700_000_000_123;

/// The bytes the layer writes, kept to read back.
#[derive(Clone, Default)]
struct Written(Arc<Mutex<Vec<u8>>>);

impl io::Write for Written {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|poisoned| io::Error::other(poisoned.to_string()))?
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Each line `run` logs, read as JSON; any line that isn't fails.
fn logged(run: impl FnOnce()) -> Vec<Value> {
    let written = Written::default();
    let writer = written.clone();
    let clock = ManualClock::at_millis(MILLIS);
    let subscriber = tracing_subscriber::registry().with(layer(move || writer.clone(), clock));
    tracing::subscriber::with_default(subscriber, run);
    let text = String::from_utf8(written.0.lock().unwrap().clone()).unwrap();
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap_or_else(|_| panic!("not JSON: {line}")))
        .collect()
}

#[test]
fn an_event_is_a_line_in_the_names_log_stores_read_with_the_spans_it_is_in() {
    let lines = logged(|| {
        let request = tracing::info_span!("session.request", method = "turn/start");
        let _request = request.enter();
        let turn = tracing::info_span!("turn", thread = "t-1", outcome = field::Empty);
        let _turn = turn.enter();
        turn.record("outcome", "Done");
        tracing::warn!(error = %"the store is gone", "glave's records failed");
    });
    assert_eq!(
        lines.first(),
        Some(&json!({
            "time": "2023-11-14T22:13:20.123Z",
            "severity": "WARNING",
            "message": "glave's records failed",
            "target": module_path!(),
            "error": "the store is gone",
            "spans": [
                {"name": "session.request", "method": "turn/start"},
                {"name": "turn", "thread": "t-1", "outcome": "Done"},
            ],
        })),
        "{lines:#?}"
    );
}

#[test]
fn each_level_is_named_as_syslog_names_it() {
    let lines = logged(|| {
        tracing::error!("e");
        tracing::warn!("w");
        tracing::info!("i");
        tracing::debug!("d");
        tracing::trace!("t");
    });
    let severities: Vec<&Value> = lines.iter().map(|line| &line["severity"]).collect();
    assert_eq!(
        severities,
        ["ERROR", "WARNING", "INFO", "DEBUG", "TRACE"],
        "{lines:#?}"
    );
}

#[test]
fn a_span_that_closes_is_a_line_of_its_own_with_its_timings() {
    let lines = logged(|| {
        let call = tracing::info_span!("model.call", model = "m", input_tokens = 12_u64);
        drop(call.enter());
    });
    let [closed] = lines.as_slice() else {
        panic!("one line, for the close: {lines:#?}");
    };
    assert_eq!(closed["message"], "close");
    assert_eq!(closed["severity"], "INFO");
    assert!(closed["time.busy"].is_string(), "{closed:#}");
    assert!(closed["time.idle"].is_string(), "{closed:#}");
    assert_eq!(
        closed["spans"],
        json!([{"name": "model.call", "model": "m", "input_tokens": 12}])
    );
}

#[test]
fn values_keep_their_types_and_a_message_stays_on_one_line() {
    let lines = logged(|| {
        tracing::info!(
            monotonic_counter.glave.turns = 1_u64,
            histogram.glave.turn.seconds = 1.5,
            is_error = false,
            "a turn\nended \"well\""
        );
    });
    let [line] = lines.as_slice() else {
        panic!("one line, a newline in its message escaped: {lines:#?}");
    };
    assert_eq!(line["monotonic_counter.glave.turns"], 1);
    assert_eq!(line["histogram.glave.turn.seconds"], 1.5);
    assert_eq!(line["is_error"], false);
    assert_eq!(line["message"], "a turn\nended \"well\"");
    assert!(line.get("spans").is_none(), "in no span: {line:#}");
}

#[test]
fn a_field_never_takes_the_place_of_glaves_own_names() {
    let lines = logged(|| tracing::info!(severity = "made up", time = 0, target = "x", "m"));
    let [line] = lines.as_slice() else {
        panic!("one line: {lines:#?}");
    };
    assert_eq!(line["severity"], "INFO");
    assert_eq!(line["time"], "2023-11-14T22:13:20.123Z");
    assert_eq!(line["target"], module_path!());
}
