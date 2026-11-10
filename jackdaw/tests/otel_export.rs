//! What an OpenTelemetry backend (Tempo, Jaeger, Grafana) receives from the engine's spans, through
//! `tracing-opentelemetry`: span names, parents, status, and trace context across a listener.
//!
//! Its own test binary: it installs a global subscriber and propagator.
#![allow(clippy::expect_used, clippy::panic)]
use jackdaw::api::Listener;
use jackdaw::prelude::*;
use opentelemetry::trace::{SpanKind, Status, TracerProvider as _};
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SpanData};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;
use tracing_subscriber::layer::SubscriberExt;

static FLAKY_CALLS: AtomicU32 = AtomicU32::new(0);

/// Fails its first call with a retryable error.
#[task(name = "otel.flaky")]
async fn flaky(_ctx: TaskContext, n: u64) -> Result<u64, TaskError> {
    if FLAKY_CALLS.fetch_add(1, Ordering::SeqCst) == 0 {
        return Err(TaskError::retryable("the first call fails"));
    }
    Ok(n)
}

#[task(name = "otel.double")]
async fn double(_ctx: TaskContext, n: u64) -> Result<u64, TaskError> {
    Ok(n * 2)
}

#[workflow(namespace = "otel", name = "child", version = "1")]
struct Child;

#[workflow_methods]
impl Child {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, n: u64) -> WorkflowResult<u64> {
        ctx.call(Double, n).await
    }
}

#[workflow(namespace = "otel", name = "parent", version = "1")]
struct Parent;

#[workflow_methods]
impl Parent {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, n: u64) -> WorkflowResult<u64> {
        let n = ctx
            .call(Flaky, n)
            .retry(Retry::constant(Duration::from_millis(5)))
            .await?;
        ctx.sleep(Duration::from_millis(1)).await?;
        ctx.run(Child::run, n).await
    }
}

const SPEC: &str = r#"
openapi: 3.0.0
info: { title: doubler, version: '1' }
paths:
  /double:
    post:
      operationId: double
      responses: { '200': { description: twice the number } }
"#;

#[derive(serde::Serialize, serde::Deserialize)]
struct Number {
    n: u64,
}

#[workflow(namespace = "otel", name = "doubler", version = "1")]
struct Doubler;

#[workflow_methods]
impl Doubler {
    #[run]
    async fn run(ctx: &mut WorkflowContext<Self>, number: Number) -> WorkflowResult<Number> {
        Ok(Number {
            n: ctx.call(Double, number.n).await?,
        })
    }
}

fn install() -> (InMemorySpanExporter, SdkTracerProvider) {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = tracing_subscriber::registry()
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("otel-export-test")));
    tracing::subscriber::set_global_default(subscriber).expect("the only global subscriber");
    opentelemetry::global::set_text_map_propagator(
        opentelemetry_sdk::propagation::TraceContextPropagator::new(),
    );
    (exporter, provider)
}

fn named<'a>(spans: &'a [SpanData], name: &str) -> &'a SpanData {
    spans
        .iter()
        .find(|span| span.name == name)
        .unwrap_or_else(|| panic!("no span named {name:?} in {:?}", names(spans)))
}

fn names(spans: &[SpanData]) -> Vec<String> {
    spans.iter().map(|span| span.name.to_string()).collect()
}

fn attribute(span: &SpanData, key: &str) -> Option<String> {
    span.attributes
        .iter()
        .find(|kv| kv.key.as_str() == key)
        .map(|kv| kv.value.to_string())
}

fn is_child_of(child: &SpanData, parent: &SpanData) -> bool {
    child.parent_span_id == parent.span_context.span_id()
        && child.span_context.trace_id() == parent.span_context.trace_id()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exported_spans_read_as_what_the_workflow_did() {
    let (exporter, provider) = install();
    let engine = Engine::builder()
        .workflow::<Parent>()
        .workflow::<Child>()
        .workflow::<Doubler>()
        .task(Flaky)
        .task(Double)
        .listener(
            Listener::openapi("api", SPEC, "127.0.0.1:0")
                .expect("the spec parses")
                .bind("double", Doubler::run),
        )
        .build()
        .expect("the engine builds");

    // A workflow that retries a task, sleeps and runs a child.
    let output = engine
        .execute(Parent::run, 21)
        .await
        .expect("starts")
        .await
        .expect("completes");
    assert_eq!(output, 42);
    provider.force_flush().expect("flushed");
    let spans = exporter.get_finished_spans().expect("the spans");
    assert_no_duplicate_attributes(&spans);

    let parent = named(&spans, "workflow otel/parent@1");
    assert_eq!(
        attribute(parent, "workflow.outcome").as_deref(),
        Some("completed")
    );
    let call = named(&spans, "call otel.flaky");
    assert!(is_child_of(call, parent));
    assert_eq!(attribute(call, "attempt").as_deref(), Some("2"));
    let first = named(&spans, "attempt 1");
    assert!(is_child_of(first, call));
    assert!(
        matches!(first.status, Status::Error { .. }),
        "{:?}",
        first.status
    );
    assert!(is_child_of(named(&spans, "attempt 2"), call));
    let backoff = spans
        .iter()
        .find(|span| span.name.starts_with("retry backoff"))
        .expect("a backoff span");
    assert!(is_child_of(backoff, call));
    assert!(is_child_of(named(&spans, "sleep 1ms"), parent));
    let run = named(&spans, "run child");
    assert!(is_child_of(run, parent));
    let child = named(&spans, "workflow otel/child@1");
    assert!(
        is_child_of(child, run),
        "the child's trace hangs under the step that ran it"
    );
    assert!(is_child_of(named(&spans, "call otel.double"), child));
    // The step's events are its span events.
    let events: Vec<&str> = call
        .events
        .iter()
        .map(|event| event.name.as_ref())
        .collect();
    assert!(events.contains(&"task started"), "{events:?}");
    assert!(events.contains(&"task completed"), "{events:?}");
    assert!(
        spans
            .iter()
            .any(|span| span.name == "persist journal" && is_child_of(span, call)),
        "recording the step's outcome is part of it"
    );
    exporter.reset();

    // A request that carries a trace continues it.
    let addr = engine.listener_addr("api").expect("the listener's address");
    let trace_id = "4bf92f3577b34da6a3ce929d0e0e4736";
    let response = reqwest::Client::new()
        .post(format!("http://{addr}/double"))
        .header("content-type", "application/json")
        .header("traceparent", format!("00-{trace_id}-00f067aa0ba902b7-01"))
        .body(json!({ "n": 4 }).to_string())
        .send()
        .await
        .expect("the request is sent");
    assert_eq!(response.status().as_u16(), 200);
    let body: Value = response.json().await.expect("JSON");
    assert_eq!(body, json!({ "n": 8 }));
    provider.force_flush().expect("flushed");
    let spans = exporter.get_finished_spans().expect("the spans");
    assert_no_duplicate_attributes(&spans);
    let request = named(&spans, "POST /double");
    assert_eq!(request.span_kind, SpanKind::Server);
    assert_eq!(request.span_context.trace_id().to_string(), trace_id);
    assert_eq!(
        attribute(request, "http.response.status_code").as_deref(),
        Some("200")
    );
    let workflow = named(&spans, "workflow otel/doubler@1");
    assert!(is_child_of(workflow, request));
    assert!(is_child_of(named(&spans, "call otel.double"), workflow));
}

/// Fails if a span has two values for one attribute: a trace backend shows both, so a field recorded twice reads
/// wrong.
fn assert_no_duplicate_attributes(spans: &[SpanData]) {
    for span in spans {
        let mut keys: Vec<&str> = span.attributes.iter().map(|kv| kv.key.as_str()).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(
            before,
            keys.len(),
            "{} has an attribute twice: {:?}",
            span.name,
            span.attributes
        );
    }
}
