//! # Traces and metrics
//!
//! jackdaw reports what it does through [`tracing`](https://docs.rs/tracing) spans and the
//! [`metrics`](https://docs.rs/metrics) facade (feature `metrics`). It installs no subscriber, exporter or recorder:
//! your program decides where traces and metrics go. With
//! [`tracing-opentelemetry`](https://docs.rs/tracing-opentelemetry), the spans become an OpenTelemetry trace you can
//! open in Grafana (Tempo), Jaeger or any other backend, one trace per workflow run, and you can drill down from the
//! run to a step, a retry and the HTTP call it made.
//!
//! ## Sending traces to OpenTelemetry
//!
//! Add a `tracing-opentelemetry` layer to your subscriber, with the exporter of your choice (`opentelemetry-otlp` for a
//! collector):
//!
//! ```no_run
//! use opentelemetry::trace::TracerProvider as _;
//! use opentelemetry_sdk::trace::{SdkTracerProvider, SpanExporter};
//! use tracing_subscriber::layer::SubscriberExt;
//!
//! fn install(exporter: impl SpanExporter + 'static) -> SdkTracerProvider {
//!     let provider = SdkTracerProvider::builder()
//!         .with_batch_exporter(exporter)
//!         .build();
//!     let subscriber = tracing_subscriber::registry()
//!         .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("my-app")));
//!     tracing::subscriber::set_global_default(subscriber).expect("one global subscriber");
//!     // For the `otel` feature: trace context in and out of listeners and HTTP calls.
//!     opentelemetry::global::set_text_map_propagator(
//!         opentelemetry_sdk::propagation::TraceContextPropagator::new(),
//!     );
//!     provider // call `shutdown` on it before the program exits, to send the last spans
//! }
//! ```
//!
//! The `jackdaw` CLI does this for you: `jackdaw run --otlp-endpoint http://localhost:4318 workflow.yaml` sends the
//! run's spans to a collector over OTLP/HTTP (also read from `OTEL_EXPORTER_OTLP_ENDPOINT`), and `--metrics-addr
//! 127.0.0.1:9464` serves its metrics for Prometheus at `/metrics`.
//!
//! ## What a trace looks like
//!
//! Each span has a static `tracing` name (the first column) and an `otel.name`, which `tracing-opentelemetry` exports
//! as the span's name, so a trace reads as what the workflow did:
//!
//! ```text
//! POST /orders                       request    (with an api::Listener)
//! └─ workflow shop/order@1           workflow
//!    ├─ replay 12 steps              replay     (on resume: the steps answered from the journal)
//!    ├─ call charge                  step
//!    │  ├─ attempt 1                 attempt    ✗ retryable error
//!    │  ├─ retry backoff 200ms       retry.backoff
//!    │  ├─ attempt 2                 attempt
//!    │  │  └─ HTTP POST pay.example  executor   (ctx.http)
//!    │  └─ persist journal           persist
//!    ├─ sleep 1h                     step
//!    ├─ run shipping                 step
//!    │  └─ workflow shop/shipping@1  workflow   (the child, under the step that runs it)
//!    └─ wait order.paid              step
//! ```
//!
//! | Span | Named | Fields |
//! |---|---|---|
//! | `workflow` | `workflow {namespace}/{name}@{version}` | `workflow.id`, `.namespace`, `.name`, `.version`, `.engine` (`native` or `yaml`), `.run`, `.outcome`; `instance.id`, `parent.instance.id`, `replay` (it resumed) |
//! | `step` | `{kind} {name}`: `call charge`, `run shipping`, `start shipping`, `sleep 5s`, `wait order.paid`, `emit order.shipped` | `step.kind`, `.name`, `.seq`, `.ordinal`, `.key`, `.outcome` (`ok`, `fault`, `timeout`, `cancelled`, `error`), `.timeout_ms`, `.shielded`, `.input.bytes`, `.output.bytes`; `attempt`, `cache.hit`, `child.instance.id`, `workflow.run`, `code.filepath`, `code.lineno` |
//! | `attempt` | `attempt {n}` | `task.name`, `attempt`, `attempt.outcome` (`ok`, `retryable_error`, `error`), `attempt.retryable`, `task.idempotency_key` |
//! | `retry.backoff` | `retry backoff {delay}` | `retry.after_attempt`, `retry.delay_ms` |
//! | `limit.wait` | `wait for limit {limit}` | `limit.name` (only when the call had to wait for a permit) |
//! | `persist` | `persist journal` | `journal.records` |
//! | `replay` | `replay {n} steps` | `replay.steps`; an event per replayed step |
//! | `task` (YAML) | `{type} {name}`: `call fetchOrder`, `for items`, `fork`, `try` | `task.name`, `.type`, `.reference` (a JSON pointer: `/do/1/items/do/0/fetch`), `.call`, `.outcome`; `cache.hit` |
//! | `iteration` (YAML) | `iteration {i}` | `for.index`, `for.each` |
//! | `executor` | `HTTP GET example.com`, `python script`, `shell make`, `container alpine` | `http.request.method`, `url.full` (without its query), `server.address`, `http.response.status_code`, `process.command`, `process.exit_code`, `container.image.name` |
//! | `request` | `POST /orders`, `grpc shop.Orders/Place` | `http.route`, `http.request.method`, `http.response.status_code`, `rpc.method` |
//! | `message` | `signal approve`, `update rename`, `query status`, `event order.paid` | `message.kind`, `.name`, `.remote`, `instance.id` |
//! | `schedule.fire` | `schedule {name}` | `schedule.id`, `.due`, `.lag_ms`, `.result`, `instance.id` |
//! | `engine` | `recover instances`, `verify replay {id}`, `replay {id}`, `run task {name}` | `engine.operation`, `.subject`, `.count` |
//! | `lease`, `store` | `lease renew`, `store append_journal` | at `DEBUG` level, for chasing a slow store |
//!
//! A failed step, attempt, workflow or call sets `otel.status_code = "ERROR"`, `otel.status_message`, `error.type` and
//! `exception.message`, so the backend marks it red. Every [`Event`](crate::api::Event) of a run is also logged, on the
//! `jackdaw::event` target, inside the span it happened in (`task started`, `task completed`, ...): it shows as an
//! event of that span. Logs from your task code land inside its `attempt` span.
//!
//! A started workflow (`ctx.start`) and each instance [`Engine::recover`](crate::api::Engine::recover) resumes get a
//! trace of their own, which follows from the span that started them; a child (`ctx.run`) is part of its parent's
//! trace. A YAML run started from Rust is under the caller's span.
//!
//! ## Payloads
//!
//! Step inputs and outputs are not on spans, only their sizes: they can hold personal or secret data, and every
//! subscriber receives what a span carries. [`EngineBuilder::trace_payloads`](crate::api::EngineBuilder::trace_payloads)
//! puts them on (`step.input`, `step.output`), as JSON cut to the size you give.
//!
//! ## Trace context
//!
//! With the feature `otel`, a request to an [`api::Listener`](crate::api::Listener) (or a YAML `listen` endpoint) that
//! carries a W3C `traceparent` header continues the caller's trace, and the HTTP calls the engine makes (`ctx.http`,
//! YAML `call: http`) send the current one on. Both use the propagator your program installs, as above.
//!
//! ## Metrics
//!
//! With the feature `metrics`, the engine reports through whichever `metrics` recorder your program installs
//! (`metrics-exporter-prometheus`, for one). Labels name workflows, steps, tasks and stores, never instances, so the
//! number of series stays bounded.
//!
//! | Metric | Type | Labels |
//! |---|---|---|
//! | `jackdaw_workflows_started_total`, `_resumed_total`, `_completed_total`, `_failed_total`, `_cancelled_total`, `_continued_as_new_total` | counter | `workflow` |
//! | `jackdaw_workflow_duration_seconds` | histogram | `workflow`, `outcome` |
//! | `jackdaw_workflows_running` | gauge | `workflow` |
//! | `jackdaw_step_duration_seconds` | histogram | `step`, `kind`, `outcome` (YAML tasks: their type as `kind`) |
//! | `jackdaw_step_timeouts_total` | counter | `step`, `kind` |
//! | `jackdaw_steps_replayed_total` | counter | `workflow` |
//! | `jackdaw_task_attempt_duration_seconds` | histogram | `task`, `outcome` |
//! | `jackdaw_task_retries_total`, `jackdaw_task_heartbeat_timeouts_total` | counter | `task` |
//! | `jackdaw_task_cache_requests_total` | counter | `task`, `result` (`hit`, `miss`) |
//! | `jackdaw_task_limit_waiting` | gauge | `limit` |
//! | `jackdaw_task_limit_wait_seconds` | histogram | `limit` |
//! | `jackdaw_tasks_active` | gauge | |
//! | `jackdaw_persistence_operation_duration_seconds` | histogram | `operation`, `backend` |
//! | `jackdaw_persistence_errors_total` | counter | `operation`, `backend` |
//! | `jackdaw_lease_renewals_total` | counter | `result` (`renewed`, `lost`, `error`) |
//! | `jackdaw_lease_renewal_lateness_seconds` | histogram | |
//! | `jackdaw_leases_held` | gauge | |
//! | `jackdaw_leases_lost_total` | counter | `workflow` |
//! | `jackdaw_messages_delivered_total` | counter | `kind` (`signal`, `update`, `query`, `event`), `result` |
//! | `jackdaw_events_emitted_total` | counter | `type` |
//! | `jackdaw_schedule_fires_total` | counter | `schedule`, `result` (`started`, `already_started`, `skipped_overlap`, `error`) |
//! | `jackdaw_schedule_lag_seconds` | histogram | `schedule` |
//! | `jackdaw_replay_divergences_total` | counter | `workflow` |
//! | `jackdaw_listener_requests_total` | counter | `protocol`, `route`, `status` |
//! | `jackdaw_listener_request_duration_seconds` | histogram | `protocol`, `route` |
//! | `jackdaw_expression_evaluations_total` | counter | `result` |
//! | `jackdaw_expression_duration_seconds` | histogram | |
//!
//! ## In Grafana
//!
//! With Tempo, TraceQL finds runs by what happened in them:
//!
//! - `{ span.instance.id = "order-1234" }`: one instance's traces.
//! - `{ name = "call charge" && status = error }`: the runs whose `charge` step failed.
//! - `{ span.task.reference = "/do/1/items/do/0/fetch" && duration > 5s }`: slow passes of one YAML task.
//! - `{ name =~ "attempt .*" && span.attempt > 1 }`: retries.
//!
//! And PromQL over the metrics:
//!
//! - `histogram_quantile(0.95, sum by (le, step) (rate(jackdaw_step_duration_seconds_bucket[5m])))`: slow steps.
//! - `sum by (task) (rate(jackdaw_task_retries_total[5m]))`: flaky tasks.
//! - `sum by (operation) (rate(jackdaw_persistence_operation_duration_seconds_sum[5m]))`: where store time goes.
//!
//! ---
//!
//! Back to the [guide](super) for the list of chapters.
