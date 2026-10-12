//! Logs and telemetry: glave's spans and metrics (content-free by construction in the crates
//! that emit them) written to a local file with `--log`, and sent to an OpenTelemetry
//! collector when `[telemetry] otlp` names one (the `otlp` feature). Off unless asked for.
//! Only glave's own targets are ever recorded, at the level GLAVE_LOG names: other crates'
//! logs could carry headers or bodies.

use std::fs::OpenOptions;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Context as _;
use glave_config::policy::Choices;
use tracing::dispatcher::DefaultGuard;
use tracing_subscriber::Layer as _;
use tracing_subscriber::filter::{EnvFilter, LevelFilter};
use tracing_subscriber::fmt::format::FmtSpan;
use tracing_subscriber::layer::SubscriberExt as _;

/// The level of glave's spans and events recorded, unless GLAVE_LOG names another.
const DEFAULT_LEVEL: LevelFilter = LevelFilter::INFO;

/// Logs and telemetry while this lives; flushed and stopped when it is dropped.
#[must_use = "telemetry stops when this is dropped"]
pub struct Telemetry {
    _guard: Option<DefaultGuard>,
    #[cfg(feature = "otlp")]
    _otlp: Option<otlp::Providers>,
}

impl Telemetry {
    /// Nothing recorded.
    pub fn off() -> Self {
        Self {
            _guard: None,
            #[cfg(feature = "otlp")]
            _otlp: None,
        }
    }
}

/// Starts recording glave's spans to the file `log`, and to the collector `choices` name.
pub fn start_for(log: Option<&Path>, choices: &Choices) -> anyhow::Result<Telemetry> {
    let otlp = choices.telemetry.as_ref();
    start(log, otlp.map(|chosen| chosen.value.as_str()))
}

/// Starts recording glave's spans to the file `log`, and sending them to the collector at
/// `otlp`, as asked; nothing when neither is.
pub fn start(log: Option<&Path>, otlp: Option<&str>) -> anyhow::Result<Telemetry> {
    if log.is_none() && otlp.is_none() {
        return Ok(Telemetry::off());
    }
    let level = level()?;
    let file = match log {
        Some(path) => {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("could not create {}", parent.display()))?;
            }
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .with_context(|| format!("could not open the log {}", path.display()))?;
            let layer = tracing_subscriber::fmt::layer()
                .with_writer(Mutex::new(file))
                .with_ansi(false)
                .with_span_events(FmtSpan::CLOSE)
                .with_filter(filter(level)?);
            Some(layer)
        }
        None => None,
    };
    let subscriber = tracing_subscriber::registry().with(file);
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

/// The level GLAVE_LOG names, or the default.
fn level() -> anyhow::Result<LevelFilter> {
    match std::env::var("GLAVE_LOG") {
        Ok(text) if !text.trim().is_empty() => text.trim().parse().map_err(|_| {
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
