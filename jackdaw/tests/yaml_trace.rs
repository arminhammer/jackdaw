//! The library never prints: what a YAML script task writes is reported as `tracing` events, for the application's
//! subscriber to show or not.
//!
//! A test binary of its own: tracing decides once per call site whether anyone listens, and other tests running
//! YAML workflows in the same process without a subscriber made that decision race with this test's.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use jackdaw::prelude::*;
use serde_json::{Value, json};
use std::sync::Arc;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Records the `jackdaw::yaml` trace events: their `event` and `line` fields.
#[derive(Clone, Default)]
struct YamlTrace(Arc<std::sync::Mutex<Vec<(String, String)>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for YamlTrace {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        struct Fields(String, String);
        impl tracing::field::Visit for Fields {
            fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                match field.name() {
                    "event" => self.0 = value.to_string(),
                    "line" => self.1 = value.to_string(),
                    _ => {}
                }
            }
            fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
        }
        if event.metadata().target() == "jackdaw::yaml" {
            let mut fields = Fields(String::new(), String::new());
            event.record(&mut fields);
            self.0.lock().unwrap().push((fields.0, fields.1));
        }
    }
}

#[tokio::test]
async fn a_yaml_scripts_output_is_a_trace_event() -> TestResult {
    use tracing::instrument::WithSubscriber as _;
    use tracing_subscriber::layer::SubscriberExt as _;

    let trace = YamlTrace::default();
    let subscriber = tracing_subscriber::registry().with(trace.clone());
    let workflow = Workflow::<Value, Value>::from_yaml(
        r#"
document:
  dsl: '1.0.0'
  namespace: events
  name: say-hello
  version: '1.0.0'
do:
  - hello:
      run:
        shell:
          command: echo hello from the script
"#,
    )?;
    async {
        let engine = Engine::builder().build()?;
        engine.execute(&workflow, json!({})).await?.await?;
        TestResult::Ok(())
    }
    .with_subscriber(subscriber)
    .await?;

    let events = trace.0.lock().unwrap().clone();
    assert!(
        events.contains(&(
            "script_line".to_string(),
            "hello from the script".to_string()
        )),
        "{events:?}"
    );
    assert!(
        events.iter().any(|(event, _)| event == "task_start"),
        "{events:?}"
    );
    Ok(())
}
