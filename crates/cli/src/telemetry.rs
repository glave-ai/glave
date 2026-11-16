//! Logs and telemetry: glave's spans and metrics (content-free by construction in the crates
//! that emit them) written to a local file with `--log`, or to standard error as JSON lines
//! with `--log -` (`json.rs`), and sent to an OpenTelemetry collector when `[telemetry] otlp`
//! names one (the `otlp` feature). Off unless asked for. Only glave's own targets are ever
//! recorded, at the level GLAVE_LOG names: other crates' logs could carry headers or bodies.

use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::Mutex;

use anyhow::Context as _;
use glave_config::policy::Choices;
use glave_ports::SystemClock;
use tracing::dispatcher::DefaultGuard;
use tracing_subscriber::Layer as _;
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt as _;

mod json;

/// The level of glave's spans and events recorded, unless GLAVE_LOG names another.
const DEFAULT_LEVEL: LevelFilter = LevelFilter::INFO;

/// The `--log` that is standard error rather than a file: the log as JSON lines, for whatever
/// collects a server's output (a container's runtime, a service manager) and hands it to a log
/// store, which reads one JSON object a line.
const STANDARD_ERROR: &str = "-";

/// Logs and telemetry while this lives; flushed and stopped when it is dropped.
#[must_use = "telemetry stops when this is dropped"]
pub(crate) struct Telemetry {
    _guard: Option<DefaultGuard>,
    #[cfg(feature = "otlp")]
    _otlp: Option<otlp::Providers>,
}

impl Telemetry {
    /// Nothing recorded.
    pub(crate) fn off() -> Self {
        Self {
            _guard: None,
            #[cfg(feature = "otlp")]
            _otlp: None,
        }
    }
}

/// Starts sending glave's spans to the collector `choices` name, the log `log` beside it, over
/// the log alone the run started with ([`start`], in `session.rs`); nothing more when they
/// name none.
pub(crate) fn start_for(log: Option<&Path>, choices: &Choices) -> anyhow::Result<Telemetry> {
    match choices.telemetry.as_ref() {
        Some(chosen) => start(log, Some(chosen.value.as_str())),
        None => Ok(Telemetry::off()),
    }
}

/// Whether the log `log` is standard error (`-`) rather than a file.
pub(crate) fn on_standard_error(log: Option<&Path>) -> bool {
    log.is_some_and(|path| path.as_os_str() == STANDARD_ERROR)
}

/// Starts recording glave's spans to the file `log`, or to standard error when it is `-`, and
/// sending them to the collector at `otlp`, as asked; nothing when neither is.
pub(crate) fn start(log: Option<&Path>, otlp: Option<&str>) -> anyhow::Result<Telemetry> {
    if log.is_none() && otlp.is_none() {
        return Ok(Telemetry::off());
    }
    let level = level()?;
    let (file, standard_error) = match log {
        Some(path) if on_standard_error(Some(path)) => {
            let layer = json::layer(std::io::stderr, SystemClock).with_filter(filter(level)?);
            (None, Some(layer))
        }
        Some(path) => {
            let layer = tracing_subscriber::fmt::layer()
                .with_writer(Mutex::new(open(path)?))
                .with_ansi(false)
                .with_span_events(FmtSpan::CLOSE)
                .with_filter(filter(level)?);
            (Some(layer), None)
        }
        None => (None, None),
    };
    let subscriber = tracing_subscriber::registry()
        .with(file)
        .with(standard_error);
    #[cfg(feature = "otlp")]
    {
        let providers = otlp.map(otlp::Providers::start).transpose()?;
        let layers = match &providers {
            Some(providers) => Some(providers.layers(filter(level)?, filter(level)?)),
            None => None,
        };
        let guard = tracing::subscriber::set_default(subscriber.with(layers));
        Ok(Telemetry {
            _guard: Some(guard),
            _otlp: providers,
        })
    }
    #[cfg(not(feature = "otlp"))]
    {
        if let Some(url) = otlp {
            anyhow::bail!(
                "[telemetry] otlp = {url:?}: this glave was built without OpenTelemetry export \
                 (build it with --features otlp)"
            );
        }
        let guard = tracing::subscriber::set_default(subscriber);
        Ok(Telemetry {
            _guard: Some(guard),
        })
    }
}

/// The log file at `path`, opened to append, its folder made first.
fn open(path: &Path) -> anyhow::Result<File> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("could not create {}", parent.display()))?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("could not open the log {}", path.display()))
}

/// The level GLAVE_LOG names, or the default. Never `off`, which tracing would take: a log asked
/// for that records nothing, not even the error glave stops on, is refused.
fn level() -> anyhow::Result<LevelFilter> {
    match std::env::var("GLAVE_LOG") {
        Ok(text) if !text.trim().is_empty() => text
            .trim()
            .parse()
            .ok()
            .filter(|level| *level != LevelFilter::OFF)
            .ok_or_else(|| {
                anyhow::anyhow!("GLAVE_LOG must be error, warn, info, debug or trace, not {text:?}")
            }),
        _ => Ok(DEFAULT_LEVEL),
    }
}

/// Only glave's own targets, at `level`.
fn filter(level: LevelFilter) -> anyhow::Result<EnvFilter> {
    EnvFilter::builder()
        .with_default_directive(LevelFilter::OFF.into())
        .parse(format!("glave={level}"))
        .context("could not build the log filter")
}

#[cfg(feature = "otlp")]
mod otlp {
    use anyhow::Context as _;
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::{MetricExporter, Protocol, SpanExporter, WithExportConfig as _};
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::metrics::SdkMeterProvider;
    use opentelemetry_sdk::trace::SdkTracerProvider;
    use tracing_subscriber::Layer;
    use tracing_subscriber::filter::EnvFilter;
    use tracing_subscriber::registry::LookupSpan;

    /// The span and metric pipelines to a collector, shut down (and so flushed) on drop.
    pub(super) struct Providers {
        spans: SdkTracerProvider,
        metrics: SdkMeterProvider,
    }

    impl Providers {
        /// Pipelines to the collector whose OTLP/HTTP endpoint is `url`.
        pub(super) fn start(url: &str) -> anyhow::Result<Self> {
            let base = url.trim().trim_end_matches('/');
            let resource = Resource::builder().with_service_name("glave").build();
            let spans = SpanExporter::builder()
                .with_http()
                .with_protocol(Protocol::HttpBinary)
                .with_endpoint(format!("{base}/v1/traces"))
                .build()
                .with_context(|| format!("could not export spans to {base}"))?;
            let metrics = MetricExporter::builder()
                .with_http()
                .with_protocol(Protocol::HttpBinary)
                .with_endpoint(format!("{base}/v1/metrics"))
                .build()
                .with_context(|| format!("could not export metrics to {base}"))?;
            Ok(Self {
                spans: SdkTracerProvider::builder()
                    .with_batch_exporter(spans)
                    .with_resource(resource.clone())
                    .build(),
                metrics: SdkMeterProvider::builder()
                    .with_periodic_exporter(metrics)
                    .with_resource(resource)
                    .build(),
            })
        }

        /// The layers that turn glave's spans and metric events into OpenTelemetry's.
        pub(super) fn layers<S>(&self, spans: EnvFilter, metrics: EnvFilter) -> impl Layer<S>
        where
            S: tracing::Subscriber + for<'span> LookupSpan<'span>,
        {
            let tracer = self.spans.tracer("glave");
            let spans = tracing_opentelemetry::layer()
                .with_tracer(tracer)
                .with_filter(spans);
            let metrics =
                tracing_opentelemetry::MetricsLayer::new(self.metrics.clone()).with_filter(metrics);
            spans.and_then(metrics)
        }
    }

    impl Drop for Providers {
        fn drop(&mut self) {
            let _ = self.spans.shutdown();
            let _ = self.metrics.shutdown();
        }
    }
}
