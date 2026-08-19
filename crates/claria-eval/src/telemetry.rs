//! OTLP/gRPC trace export, and the stderr subscriber that is always on.
//!
//! # Authority
//!
//! This module configures transport and never resolves credentials. Ingest
//! authentication is read by the OpenTelemetry SDK from the standard
//! `OTEL_EXPORTER_OTLP_HEADERS` environment variable, so a token is never a
//! command-line argument, never lives in a config file this tool parses, and
//! never lands on a span attribute.
//!
//! Against the OpenObserve receiver on the Pi that variable looks like:
//!
//! ```text
//! OTEL_EXPORTER_OTLP_ENDPOINT=http://rpi.lan
//! OTEL_EXPORTER_OTLP_HEADERS="Authorization=Basic%20<token>,organization=default,stream-name=claria"
//! ```
//!
//! `organization=default` is required over gRPC — without it OpenObserve
//! rejects every export with `InvalidArgument` and the sender sees nothing.
//! The space in `Basic <token>` must be percent-encoded. `stream-name=claria`
//! is what routes these spans into the `claria` stream rather than `default`.
//!
//! Export is optional: with no `OTEL_EXPORTER_OTLP_ENDPOINT` the OTLP layer
//! is simply absent and the stderr subscriber runs alone.

use std::time::Duration;

use eyre::{Context, Result};
use opentelemetry::{KeyValue, trace::TracerProvider as _};
use opentelemetry_otlp::{SpanExporter, WithExportConfig as _};
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use tracing_subscriber::{
    EnvFilter, Layer as _, fmt, layer::SubscriberExt as _, util::SubscriberInitExt as _,
};

/// The service these spans are attributed to in the receiver.
pub const SERVICE_NAME: &str = "claria-eval";

/// The standard SDK variable that turns export on. Read here only to decide
/// whether to build an exporter; the SDK reads it again for the endpoint.
pub const ENDPOINT_VAR: &str = "OTEL_EXPORTER_OTLP_ENDPOINT";

/// How long one export batch — and the final flush — may take.
const EXPORT_TIMEOUT: Duration = Duration::from_secs(5);

/// How long the shutdown flush may take before the command gives up on it.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// What the OTLP layer records, and what it must not.
///
/// The transport crates are silenced explicitly: an OTLP exporter that logged
/// through `tracing` would feed its own export failures back into itself.
const OTEL_TRACE_FILTER: &str = "claria_eval=trace,claria=trace,\
claria_report_store=trace,claria_bedrock=trace,claria_records=trace,claria_storage=trace,\
hyper=off,h2=off,opentelemetry=off,tonic=off,tower=off,aws_smithy_runtime=off,\
aws_config=off,aws_sdk_s3=off,aws_sdk_bedrockruntime=off";

/// Installed subscriber plus the provider that has to be flushed on the way
/// out.
pub struct Telemetry {
    provider: Option<SdkTracerProvider>,
}

impl Telemetry {
    /// Whether spans are actually being exported.
    pub fn exporting(&self) -> bool {
        self.provider.is_some()
    }

    /// Flush and shut the exporter down.
    ///
    /// The caller fails the command on an error: this is the short-lived-CLI
    /// policy, not the long-running-daemon one. A run whose spans never left
    /// the process is not a run anybody can read afterwards, and reporting it
    /// as a success is how silent export failures survive.
    pub fn shutdown(self) -> Result<()> {
        let Some(provider) = self.provider else {
            return Ok(());
        };
        let mut failures = Vec::new();
        if let Err(error) = provider.force_flush() {
            failures.push(format!("flush: {error}"));
        }
        if let Err(error) = provider.shutdown_with_timeout(SHUTDOWN_TIMEOUT) {
            failures.push(format!("shutdown: {error}"));
        }
        if failures.is_empty() {
            Ok(())
        } else {
            Err(eyre::eyre!(
                "OTLP trace export did not complete ({}); the run itself may have succeeded, \
                 but its spans are not in the receiver",
                failures.join("; ")
            ))
        }
    }
}

/// Install the stderr subscriber, and the OTLP layer when an endpoint is
/// configured.
pub fn init() -> Result<Telemetry> {
    let provider = match std::env::var(ENDPOINT_VAR) {
        Ok(endpoint) if !endpoint.trim().is_empty() => Some(build_provider(endpoint.trim())?),
        _ => None,
    };

    // stderr, not stdout: stdout carries the plan and the draft, which a
    // caller may pipe.
    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(true)
        .with_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")));

    let otel_layer = provider.as_ref().map(|provider| {
        tracing_opentelemetry::layer()
            .with_tracer(provider.tracer(SERVICE_NAME))
            .with_filter(EnvFilter::new(OTEL_TRACE_FILTER))
    });

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(otel_layer)
        .try_init()
        .wrap_err("could not install the tracing subscriber")?;

    Ok(Telemetry { provider })
}

/// The endpoint is an authority (`http://rpi.lan`), not a path: OTLP/gRPC
/// method paths come from the protobuf service definition, so anything after
/// the host would be prepended to them.
fn build_provider(endpoint: &str) -> Result<SdkTracerProvider> {
    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_endpoint(endpoint.to_string())
        .with_timeout(EXPORT_TIMEOUT)
        .build()
        .wrap_err("could not build the OTLP trace exporter")?;
    Ok(SdkTracerProvider::builder()
        .with_resource(
            Resource::builder()
                .with_service_name(SERVICE_NAME)
                .with_attributes([
                    KeyValue::new("service.version", env!("CARGO_PKG_VERSION")),
                    KeyValue::new("process.executable.name", SERVICE_NAME),
                ])
                .build(),
        )
        .with_batch_exporter(exporter)
        .build())
}
