//! glave's log as JSON lines (`--log -`), for whatever collects a server's output: one object a
//! line for each event, in the names log stores read without being told (`time` in RFC 3339,
//! `severity` in syslog's words, `message`), then its `target`, its own fields beside them, and
//! the spans it is in, outermost first, each with its fields. Only what glave's events and
//! spans carry, which is content-free where they are made.

use std::fmt;

use glave_ports::Clock;
use serde_json::{Map, Value};
use tracing::field::{Field, Visit};
use tracing::span::Record;
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::field::RecordFields;
use tracing_subscriber::fmt::format::{FmtSpan, Writer};
use tracing_subscriber::fmt::{
    FmtContext, FormatEvent, FormatFields, FormattedFields, Layer, MakeWriter,
};
use tracing_subscriber::registry::{LookupSpan, Scope};

/// The layer that writes each of glave's events, and each span as it closes, to `writer` as a
/// line of JSON, timed by `clock`.
pub(super) fn layer<S, W, C>(writer: W, clock: C) -> Layer<S, JsonFields, Json<C>, W>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    W: for<'writer> MakeWriter<'writer> + 'static,
    C: Clock + 'static,
{
    tracing_subscriber::fmt::layer()
        .with_span_events(FmtSpan::CLOSE)
        .fmt_fields(JsonFields)
        .event_format(Json { clock })
        .with_writer(writer)
}

/// An event as one JSON object.
pub(super) struct Json<C> {
    clock: C,
}

impl<S, C> FormatEvent<S, JsonFields> for Json<C>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
    C: Clock,
{
    fn format_event(
        &self,
        context: &FmtContext<'_, S, JsonFields>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut entry = Map::new();
        event.record(&mut Values(&mut entry));
        let spans: Vec<Value> = context
            .event_scope()
            .into_iter()
            .flat_map(Scope::from_root)
            .map(|span| {
                let mut fields = span
                    .extensions()
                    .get::<FormattedFields<JsonFields>>()
                    .map(|kept| object(&kept.fields))
                    .unwrap_or_default();
                fields.insert("name".to_owned(), span.name().into());
                Value::Object(fields)
            })
            .collect();
        if !spans.is_empty() {
            entry.insert("spans".to_owned(), spans.into());
        }
        // glave's own names last, so no field of the same name takes their place.
        let metadata = event.metadata();
        entry.insert("target".to_owned(), metadata.target().into());
        entry.insert("severity".to_owned(), severity(*metadata.level()).into());
        entry.insert("time".to_owned(), self.clock.now().to_string().into());
        writeln!(writer, "{}", Value::Object(entry))
    }
}

/// A span's fields, kept as a JSON object for [`Json`] to put in the spans of each event.
pub(super) struct JsonFields;

impl<'writer> FormatFields<'writer> for JsonFields {
    fn format_fields<R: RecordFields>(
        &self,
        mut writer: Writer<'writer>,
        fields: R,
    ) -> fmt::Result {
        let mut values = Map::new();
        fields.record(&mut Values(&mut values));
        write!(writer, "{}", Value::Object(values))
    }

    /// Fields recorded on a span after it began, beside the ones it began with.
    fn add_fields(
        &self,
        current: &'writer mut FormattedFields<Self>,
        fields: &Record<'_>,
    ) -> fmt::Result {
        let mut values = object(&current.fields);
        fields.record(&mut Values(&mut values));
        current.fields = Value::Object(values).to_string();
        Ok(())
    }
}

/// The JSON object `kept` holds, as [`JsonFields`] wrote it.
fn object(kept: &str) -> Map<String, Value> {
    serde_json::from_str(kept).unwrap_or_default()
}

/// A level in syslog's words, which log stores read: `WARNING`, never tracing's `WARN`, which
/// some take for no severity at all. Below `DEBUG`, tracing's `TRACE`, which they keep as none.
fn severity(level: Level) -> &'static str {
    match level {
        Level::ERROR => "ERROR",
        Level::WARN => "WARNING",
        Level::INFO => "INFO",
        Level::DEBUG => "DEBUG",
        _ => "TRACE",
    }
}

/// Puts each field it is shown into the object, as the JSON value nearest its type.
struct Values<'object>(&'object mut Map<String, Value>);

impl Values<'_> {
    fn put(&mut self, field: &Field, value: Value) {
        self.0.insert(field.name().to_owned(), value);
    }
}

impl Visit for Values<'_> {
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.put(field, value.into());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.put(field, value.into());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.put(field, value.into());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.put(field, value.into());
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.put(field, value.into());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.put(field, format!("{value:?}").into());
    }
}

#[cfg(test)]
mod tests;
