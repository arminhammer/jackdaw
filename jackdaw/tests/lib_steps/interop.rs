//! interop/catalogs.feature, interop/yaml_workflows.feature and the provider scenarios of
//! interop/custom_extensions.feature.
use super::support::*;
use super::world::LibWorld;
use async_trait::async_trait;
use cucumber::{given, then, when};
use jackdaw::cache::{CacheEntry, CacheProvider};
use jackdaw::persistence::PersistenceProvider;
use jackdaw::prelude::*;
use jackdaw::testing::TestEngine;
use jackdaw::workflow::{WorkflowCheckpoint, WorkflowEvent};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------------------------------------------
// Catalogs
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Add {
    pub a: i64,
    pub b: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Sum {
    pub result: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Text {
    pub text: String,
}

#[catalog(source = "file://./tests/rust/fixtures/catalog")]
pub trait Shared {
    #[task(name = "add-numbers", version = "1.0.0")]
    async fn add(input: Add) -> Result<Sum, TaskError>;

    #[workflow(name = "normalize", version = "2.0.0")]
    async fn normalize(input: Text) -> WorkflowResult<Text>;
}

#[catalog(source = "file://./tests/rust/fixtures/catalog")]
pub trait Missing {
    #[task(name = "missing", version = "1.0.0")]
    async fn missing(input: Add) -> Result<Sum, TaskError>;
}

#[workflow(namespace = "interop", name = "adds", version = "1")]
pub struct Adds;

#[workflow_methods]
impl Adds {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, input: Add) -> WorkflowResult<Sum> {
        Ok(ctx.call(Shared::add, &input).await?)
    }
}

/// Runs `test` on a paused runtime of its own, the way `#[tokio::test(start_paused = true)]` would.
fn paused<T: Send + 'static>(test: impl std::future::Future<Output = T> + Send + 'static) -> T {
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .expect("a runtime")
            .block_on(test)
    })
    .join()
    .expect("the test thread")
}

#[given(
    regex = r#"^a "\#\[catalog\]" that imports the function "add-numbers@1\.0\.0" with input "Add" and output "Sum"$"#
)]
fn catalog_function(_world: &mut LibWorld) {}

#[given("an engine with the catalog registered")]
fn catalog_engine(_world: &mut LibWorld) {}

#[when(regex = r"^a workflow calls it with (\d+) and (\d+)$")]
fn calls_catalog(world: &mut LibWorld, a: i64, b: i64) {
    let sum = paused(async move {
        let engine = TestEngine::builder()
            .catalog::<Shared>()
            .workflow::<Adds>()
            .build()
            .expect("builds");
        engine
            .execute(Adds::run, Add { a, b })
            .await
            .expect("starts")
            .await
            .expect("finishes")
    });
    world.notes.insert("sum".into(), json!(sum.result));
}

#[then(regex = r"^the step returns a sum of (\d+)$")]
fn sum_is(world: &mut LibWorld, n: i64) {
    assert_eq!(world.notes["sum"], json!(n));
}

#[given(regex = r#"^a "\#\[catalog\]" that imports a workflow$"#)]
fn catalog_workflow(_world: &mut LibWorld) {}

#[when(regex = r#"^I call "Engine::execute" with the catalog workflow marker$"#)]
fn execute_catalog_workflow(world: &mut LibWorld) {
    let text = paused(async {
        let engine = TestEngine::builder()
            .catalog::<Shared>()
            .build()
            .expect("builds");
        engine
            .execute(
                Shared::normalize,
                Text {
                    text: "  Hello WORLD ".into(),
                },
            )
            .await
            .expect("starts")
            .await
            .expect("finishes")
    });
    world.notes.insert("normalized".into(), json!(text.text));
}

#[then("the catalog's workflow definition runs")]
fn catalog_workflow_ran(world: &mut LibWorld) {
    assert_eq!(world.notes["normalized"], json!("hello world"));
}

#[given(regex = r#"^a "\#\[catalog\]" that imports "missing@1\.0\.0"$"#)]
fn catalog_missing(_world: &mut LibWorld) {}

#[when("I build an engine with it")]
fn build_with_missing(world: &mut LibWorld) {
    world.build_error = TestEngine::builder()
        .catalog::<Missing>()
        .build()
        .err()
        .map(|e| match e {
            Error::Invalid(message) => format!("Invalid: {message}"),
            other => format!("other: {other}"),
        });
}

// ---------------------------------------------------------------------------------------------------------------
// YAML workflows
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Charge {
    pub order: String,
    pub cents: u64,
}

/// YAML calls the `payments.charge` task of api.rs (`Api::charge`) by name: it answers "paid <input>".
pub fn defaults(_probe: &Probe) {}

const LABEL_YAML: &str = r#"
document: { dsl: '1.0.3', namespace: interop, name: label, version: '1.0.0' }
do:
  - label:
      set:
        text: '${ "SHIP TO: " + .order_id }'
"#;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LabelRequest {
    pub order_id: String,
}

#[workflow(namespace = "interop", name = "labelled", version = "1")]
pub struct Labelled;

#[workflow_methods]
impl Labelled {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, order_id: String) -> WorkflowResult<Text> {
        let label = Workflow::<LabelRequest, Text>::from_yaml(LABEL_YAML)?;
        Ok(ctx
            .run_definition(&label, &LabelRequest { order_id })
            .await?)
    }
}

pub fn fixtures() -> Vec<super::world::Fixture> {
    vec![super::world::Fixture::of::<Labelled>("interop.labelled").input(json!("o-9"))]
}

const CHARGE_YAML: &str = r#"
document: { dsl: '1.0.3', namespace: interop, name: charge, version: '1.0.0' }
do:
  - charge:
      call: payments.charge
      with:
        order: '${ .order }'
        cents: '${ .cents }'
"#;

const BROKEN_YAML: &str = r#"
document: { dsl: '1.0.3', namespace: interop, name: broken, version: '1.0.0' }
do:
  - first:
      call: payments.charge
      with: { order: o-1, cents: 5 }
      then: nowhere
"#;

#[given(regex = r#"^the YAML workflow "hello-world\.sw\.yaml" loaded with "Workflow::from_yaml"$"#)]
fn hello_world(world: &mut LibWorld) {
    let yaml = std::fs::read_to_string("../jackdaw-cli/examples/hello-world/hello-world.sw.yaml")
        .expect("the example");
    world.notes.insert("yaml".into(), json!(yaml));
}

#[given(regex = r#"^a YAML workflow that refers to an undefined task in a "then" directive$"#)]
fn broken_yaml(world: &mut LibWorld) {
    world.notes.insert("yaml".into(), json!(BROKEN_YAML));
}

#[when("I execute it from Rust")]
async fn execute_yaml(world: &mut LibWorld) {
    let yaml = world.notes["yaml"]
        .as_str()
        .expect("a workflow")
        .to_string();
    let workflow = Workflow::<Value, Value>::from_yaml(&yaml).expect("parses");
    let engine = world.engine();
    let result = match engine.execute(&workflow, json!({})).await {
        Ok(handle) => handle.await,
        Err(error) => Err(error),
    };
    match result {
        Ok(output) => world.notes.insert("yaml-output".into(), output),
        Err(error) => world
            .notes
            .insert("yaml-error".into(), json!(format!("{error:?}"))),
    };
}

#[then(regex = r#"^the output is "(.+)"$"#)]
fn yaml_output(world: &mut LibWorld, expected: String) {
    assert_eq!(
        world.notes.get("yaml-output"),
        Some(&json!(expected)),
        "{:?}",
        world.notes.get("yaml-error")
    );
}

#[then(regex = r#"^it fails with "Error::Invalid" naming the directive$"#)]
fn invalid_directive(world: &mut LibWorld) {
    let error = world
        .notes
        .get("yaml-error")
        .and_then(Value::as_str)
        .expect("the run failed");
    assert!(error.starts_with("Invalid"), "{error}");
    assert!(error.contains("nowhere"), "{error}");
}

#[then("no task of the workflow ran")]
fn nothing_ran(world: &mut LibWorld) {
    assert_eq!(world.probe.count("api_payments"), 0);
}

#[given(regex = r#"^a Rust workflow that calls "ctx\.run_definition\(&yaml_workflow, &input\)"$"#)]
fn runs_definition(world: &mut LibWorld) {
    world.set_fixture(super::fixture("interop.labelled"));
}

#[when("I execute the Rust workflow")]
async fn execute_rust(world: &mut LibWorld) {
    world.start("it", json!("o-9")).await;
}

#[then("the YAML workflow's output is returned to the Rust workflow as a typed value")]
async fn typed_yaml_output(world: &mut LibWorld) {
    let last = world.last();
    assert_eq!(
        world.outcome(last).await.value(),
        &json!({"text": "SHIP TO: o-9"})
    );
}

#[given(regex = r#"^the Rust task "payments\.charge" registered on the engine$"#)]
fn charge_registered(_world: &mut LibWorld) {}

#[given(regex = r#"^a YAML workflow with a "call: payments\.charge" task$"#)]
fn yaml_calls_charge(world: &mut LibWorld) {
    world.notes.insert("yaml".into(), json!(CHARGE_YAML));
}

#[when("I execute the YAML workflow")]
async fn execute_yaml_workflow(world: &mut LibWorld) {
    let workflow = Workflow::<Charge, String>::from_yaml(CHARGE_YAML).expect("parses");
    let receipt = world
        .engine()
        .execute(
            &workflow,
            Charge {
                order: "o-1".into(),
                cents: 250,
            },
        )
        .await
        .expect("starts")
        .await
        .expect("finishes");
    world.notes.insert("receipt".into(), json!(receipt));
}

#[then("the Rust task runs with the YAML task's arguments as its typed input")]
fn typed_input(world: &mut LibWorld) {
    let calls = world.probe.calls_of("api_payments");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].input, json!({"order": "o-1", "cents": 250}));
}

#[then("its output becomes the YAML task's output")]
fn yaml_task_output(world: &mut LibWorld) {
    let input = json!({"order": "o-1", "cents": 250});
    assert_eq!(world.notes["receipt"], json!(format!("paid {input}")));
}

#[when("I execute one YAML and one Rust workflow")]
async fn one_of_each(world: &mut LibWorld) {
    let engine = world.engine();
    let workflow = Workflow::<Charge, String>::from_yaml(CHARGE_YAML).expect("parses");
    let yaml = engine
        .execute(
            &workflow,
            Charge {
                order: "o-2".into(),
                cents: 5,
            },
        )
        .await
        .expect("starts");
    let yaml_id = yaml.id().to_string();
    yaml.await.expect("finishes");
    world.notes.insert("yaml-id".into(), json!(yaml_id));
    world.set_fixture(super::fixture("interop.labelled"));
    let run = world.start("rust", json!("o-3")).await;
    world.outcome(run).await.value();
}

#[then(regex = r#"^"Engine::history" returns events for both instances$"#)]
async fn history_for_both(world: &mut LibWorld) {
    let engine = world.engine();
    let yaml_id = world.notes["yaml-id"].as_str().expect("an id").to_string();
    let rust_id = world.runs[world.last()].id.clone();
    for id in [yaml_id, rust_id] {
        let history = engine.history(&id).await.expect("history");
        assert!(history.len() >= 2, "{id}: {history:?}");
        assert!(
            history.last().expect("events").is_terminal(),
            "{id}: {history:?}"
        );
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Custom providers
// ---------------------------------------------------------------------------------------------------------------

/// A store kept in plain maps: only the four required methods.
#[derive(Debug, Default)]
pub struct MapStore {
    events: Mutex<HashMap<String, Vec<WorkflowEvent>>>,
    checkpoints: Mutex<HashMap<String, WorkflowCheckpoint>>,
}

#[async_trait]
impl PersistenceProvider for MapStore {
    async fn save_event(&self, event: WorkflowEvent) -> jackdaw::persistence::Result<()> {
        lock(&self.events)
            .entry(event.instance_id().to_string())
            .or_default()
            .push(event);
        Ok(())
    }

    async fn get_events(
        &self,
        instance_id: &str,
    ) -> jackdaw::persistence::Result<Vec<WorkflowEvent>> {
        Ok(lock(&self.events)
            .get(instance_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn save_checkpoint(
        &self,
        checkpoint: WorkflowCheckpoint,
    ) -> jackdaw::persistence::Result<()> {
        lock(&self.checkpoints).insert(checkpoint.instance_id.clone(), checkpoint);
        Ok(())
    }

    async fn get_checkpoint(
        &self,
        instance_id: &str,
    ) -> jackdaw::persistence::Result<Option<WorkflowCheckpoint>> {
        Ok(lock(&self.checkpoints).get(instance_id).cloned())
    }
}

/// A cache that counts lookups and hits.
#[derive(Debug, Default)]
pub struct CountingCache {
    entries: Mutex<HashMap<String, CacheEntry>>,
    lookups: AtomicUsize,
    hits: AtomicUsize,
}

#[async_trait]
impl CacheProvider for CountingCache {
    async fn get(&self, key: &str) -> jackdaw::cache::Result<Option<CacheEntry>> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        let hit = lock(&self.entries).get(key).cloned();
        if hit.is_some() {
            self.hits.fetch_add(1, Ordering::SeqCst);
        }
        Ok(hit)
    }

    async fn set(&self, entry: CacheEntry) -> jackdaw::cache::Result<()> {
        lock(&self.entries).insert(entry.key.clone(), entry);
        Ok(())
    }

    async fn invalidate(&self, key: &str) -> jackdaw::cache::Result<()> {
        lock(&self.entries).remove(key);
        Ok(())
    }
}

#[given(regex = r#"^a type that implements "PersistenceProvider" with in-memory maps$"#)]
fn map_store(world: &mut LibWorld) {
    world.persistence = Some(Arc::new(MapStore::default()));
}

#[when(regex = r#"^I build an engine with "EngineBuilder::persistence\(Arc::new\(provider\)\)"$"#)]
fn build_with_store(world: &mut LibWorld) {
    world.engine();
}

#[when("a workflow runs to completion")]
async fn runs_to_completion(world: &mut LibWorld) {
    world.set_fixture(super::fixture("interop.labelled"));
    let run = world.start("it", json!("o-4")).await;
    world.outcome(run).await.value();
}

#[then("the provider holds the instance's checkpoint and events")]
async fn provider_holds(world: &mut LibWorld) {
    let store = world.persistence.clone().expect("the provider");
    let id = world.runs[world.last()].id.clone();
    assert!(store.get_checkpoint(&id).await.expect("reads").is_some());
    assert!(!store.get_events(&id).await.expect("reads").is_empty());
}

#[given(regex = r#"^a type that implements "CacheProvider" that counts lookups$"#)]
fn counting_cache(world: &mut LibWorld) {
    let cache = Arc::new(CountingCache::default());
    let for_builder = Arc::clone(&cache);
    world.extras.push(Arc::new(move |builder: EngineBuilder| {
        builder.cache(Arc::clone(&for_builder) as Arc<dyn CacheProvider>)
    }));
    lock(&CACHES).push((scenario_key(world), cache));
}

/// The counting cache of each scenario, by the scenario's directory.
static CACHES: Mutex<Vec<(String, Arc<CountingCache>)>> = Mutex::new(Vec::new());

fn scenario_key(world: &LibWorld) -> String {
    world.dir.path().display().to_string()
}

fn scenario_cache(world: &LibWorld) -> Arc<CountingCache> {
    let key = scenario_key(world);
    lock(&CACHES)
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, cache)| Arc::clone(cache))
        .expect("the scenario's cache")
}

#[when(regex = r#"^I build an engine with "EngineBuilder::cache\(Arc::new\(provider\)\)"$"#)]
fn build_with_cache(world: &mut LibWorld) {
    world.engine();
}

#[when("a cached task runs twice with equal input")]
async fn cached_twice(world: &mut LibWorld) {
    let engine = world.engine();
    for _ in 0..2 {
        let output: String = engine
            .run_task(
                super::api::Api::charge,
                json!({"order": "o-5", "cents": 7}),
                TaskOptions::new().cached(),
            )
            .await
            .expect("runs");
        assert!(output.starts_with("paid"), "{output}");
    }
}

#[then("the provider served the second result")]
fn cache_served(world: &mut LibWorld) {
    assert_eq!(world.probe.count("api_payments"), 1, "the task ran once");
    let cache = scenario_cache(world);
    assert_eq!(cache.lookups.load(Ordering::SeqCst), 2);
    assert_eq!(cache.hits.load(Ordering::SeqCst), 1);
}
