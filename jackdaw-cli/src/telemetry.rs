//! Where `jackdaw run` sends what the engine reports: traces to an OpenTelemetry collector (OTLP over HTTP), metrics
//! to Prometheus.
use std::net::SocketAddr;

/// The telemetry flags of `jackdaw run`.
#[derive(clap::Args, Debug, Clone, Default)]
pub struct TelemetryArgs {
    /// Send traces to this OpenTelemetry collector, over OTLP/HTTP (for example `http://localhost:4318`; spans go to
    /// `/v1/traces` under it). `JACKDAW_OTLP_FILTER` picks which spans, in `RUST_LOG` syntax (default `info`)
    #[arg(long, env = "OTEL_EXPORTER_OTLP_ENDPOINT", value_name = "URL")]
    pub otlp_endpoint: Option<String>,

    /// Serve Prometheus metrics at `http://ADDR/metrics` while workflows run (for example `127.0.0.1:9464`)
    #[arg(long, env = "JACKDAW_METRICS_ADDR", value_name = "ADDR")]
    pub metrics_addr: Option<SocketAddr>,
}

/// Why telemetry could not be set up.
#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("the OTLP exporter could not be built: {message}"))]
    Otlp { message: String },
    #[snafu(display("the Prometheus endpoint could not be served on {addr}: {message}"))]
    Prometheus { addr: SocketAddr, message: String },
}

/// The trace pipeline, flushed and shut down by [`Telemetry::shutdown`].
#[derive(Default)]
pub struct Telemetry {
    #[cfg(feature = "telemetry")]
    provider: Option<opentelemetry_sdk::trace::SdkTracerProvider>,
}

/// The OTLP traces endpoint for a collector's base URL (as `OTEL_EXPORTER_OTLP_ENDPOINT` is: `/v1/traces` is added).
#[cfg(feature = "telemetry")]
fn traces_endpoint(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.ends_with("/v1/traces") {
        base.to_string()
    } else {
        format!("{base}/v1/traces")
    }
}

impl Telemetry {
    /// Builds the trace exporter, if an endpoint is given, and returns the tracer for the subscriber's layer. W3C trace
    /// context becomes the propagator, so listener requests and HTTP calls join the callers' traces.
    #[cfg(feature = "telemetry")]
    pub fn traces(
        &mut self,
        args: &TelemetryArgs,
    ) -> Result<Option<opentelemetry_sdk::trace::Tracer>, Error> {
        use opentelemetry::trace::TracerProvider as _;
        use opentelemetry_otlp::WithExportConfig as _;
        let Some(endpoint) = &args.otlp_endpoint else {
            return Ok(None);
        };
        let exporter = opentelemetry_otlp::SpanExporter::builder()
            .with_http()
            .with_endpoint(traces_endpoint(endpoint))
            .build()
            .map_err(|e| Error::Otlp {
                message: e.to_string(),
            })?;
        let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
            .with_batch_exporter(exporter)
            .with_resource(
                opentelemetry_sdk::Resource::builder()
                    .with_service_name("jackdaw")
                    .build(),
            )
            .build();
        opentelemetry::global::set_text_map_propagator(
            opentelemetry_sdk::propagation::TraceContextPropagator::new(),
        );
        let tracer = provider.tracer("jackdaw");
        self.provider = Some(provider);
        Ok(Some(tracer))
    }

    /// Serves the engine's metrics for Prometheus to scrape, if an address is given. Needs a tokio runtime.
    #[cfg(feature = "telemetry")]
    pub fn metrics(args: &TelemetryArgs) -> Result<(), Error> {
        let Some(addr) = args.metrics_addr else {
            return Ok(());
        };
        // Histograms, not summaries: buckets can be added up across processes and over time in Grafana.
        const BUCKETS: &[f64] = &[
            0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
            30.0, 60.0, 300.0, 900.0, 3600.0,
        ];
        metrics_exporter_prometheus::PrometheusBuilder::new()
            .with_http_listener(addr)
            .set_buckets_for_metric(
                metrics_exporter_prometheus::Matcher::Suffix("_seconds".to_string()),
                BUCKETS,
            )
            .map_err(|e| Error::Prometheus {
                addr,
                message: e.to_string(),
            })?
            .install()
            .map_err(|e| Error::Prometheus {
                addr,
                message: e.to_string(),
            })
    }

    /// Without the `telemetry` feature, asking for an exporter is an error rather than silently nothing.
    #[cfg(not(feature = "telemetry"))]
    pub fn metrics(args: &TelemetryArgs) -> Result<(), Error> {
        match args.metrics_addr {
            Some(addr) => Err(Error::Prometheus {
                addr,
                message: "this jackdaw was built without the `telemetry` feature".into(),
            }),
            None => Ok(()),
        }
    }

    /// Sends the spans not sent yet. (The exporter's HTTP client blocks, so this runs off the async threads.)
    pub async fn shutdown(self) {
        #[cfg(feature = "telemetry")]
        if let Some(provider) = self.provider {
            let _ = tokio::task::spawn_blocking(move || {
                if let Err(error) = provider.shutdown() {
                    eprintln!("traces could not all be sent: {error}");
                }
            })
            .await;
        }
    }
}

#[cfg(all(test, feature = "telemetry"))]
mod tests {
    use super::traces_endpoint;

    #[test]
    fn the_traces_path_is_added_once() {
        assert_eq!(
            traces_endpoint("http://localhost:4318"),
            "http://localhost:4318/v1/traces"
        );
        assert_eq!(
            traces_endpoint("http://localhost:4318/"),
            "http://localhost:4318/v1/traces"
        );
        assert_eq!(
            traces_endpoint("http://collector/v1/traces"),
            "http://collector/v1/traces"
        );
    }
}
