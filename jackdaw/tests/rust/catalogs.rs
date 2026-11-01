//! Catalogs: versioned tasks and workflows imported with types. Catalog entries are plain spec YAML
//! (`fixtures/catalog/<name>/<version>/function.yaml`), so other runtimes can use them too.
//!
//! The Rust side declares the entries as a trait: the macro generates typed markers (`Local::add`,
//! `Local::normalize`) that compile to `call: add-numbers:1.0.0` / `run.workflow`.
use crate::support::*;
use jackdaw::testing::TestEngine;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AddIn {
    pub a: i64,
    pub b: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AddOut {
    pub result: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Text {
    pub text: String,
}

#[catalog(source = "file://./tests/rust/fixtures/catalog")]
pub trait Local {
    #[task(name = "add-numbers", version = "1.0.0")]
    async fn add(input: AddIn) -> Result<AddOut, TaskError>;

    #[workflow(name = "normalize", version = "2.0.0")]
    async fn normalize(input: Text) -> WorkflowResult<Text>;
}

#[tokio::test(start_paused = true)]
async fn calls_a_catalog_task_with_typed_input_and_output() -> TestResult {
    let engine = TestEngine::builder().catalog::<Local>().build()?;

    let sum: AddOut = engine
        .run_task(Local::add, AddIn { a: 40, b: 2 }, TaskOptions::new())
        .await?;

    assert_eq!(sum, AddOut { result: 42 });
    Ok(())
}

#[workflow(namespace = "catalog-tests", name = "uses-catalog", version = "1.0.0")]
pub struct UsesCatalog;

#[workflow_methods]
impl UsesCatalog {
    #[run]
    pub async fn run(
        ctx: &mut WorkflowContext<Self>,
        input: Text,
    ) -> WorkflowResult<(Text, AddOut)> {
        // A catalog task and a catalog workflow, used exactly like local ones.
        let sum = ctx.call(Local::add, AddIn { a: 1, b: 2 }).await?;
        let cleaned = ctx.run(Local::normalize, &input).await?;
        Ok((cleaned, sum))
    }
}

#[tokio::test(start_paused = true)]
async fn a_rust_workflow_uses_catalog_tasks_and_workflows() -> TestResult {
    let engine = TestEngine::builder()
        .catalog::<Local>()
        .workflow::<UsesCatalog>()
        .build()?;

    let (cleaned, sum) = engine
        .execute(
            UsesCatalog::run,
            Text {
                text: "  Hello WORLD ".into(),
            },
        )
        .await?
        .await?;

    assert_eq!(cleaned.text, "hello world");
    assert_eq!(sum.result, 3);
    Ok(())
}

// ---------------------------------------------------------------------------------------------------------------
// Mistakes are found when the engine is built, not in the middle of a run
// ---------------------------------------------------------------------------------------------------------------

#[catalog(source = "file://./tests/rust/fixtures/catalog")]
pub trait MissingVersion {
    #[task(name = "add-numbers", version = "9.9.9")]
    async fn add(input: AddIn) -> Result<AddOut, TaskError>;
}

#[test]
fn a_missing_version_is_rejected_when_the_engine_is_built() {
    let err = TestEngine::builder()
        .catalog::<MissingVersion>()
        .build()
        .unwrap_err();

    assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    assert!(err.to_string().contains("9.9.9"));
}

#[catalog(source = "file://./tests/rust/fixtures/catalog")]
pub trait WrongTypes {
    // The catalog entry's input schema needs `a` and `b`; `Text` has neither.
    #[task(name = "add-numbers", version = "1.0.0")]
    async fn add(input: Text) -> Result<AddOut, TaskError>;
}

#[test]
fn a_binding_that_contradicts_the_entrys_schema_is_rejected() {
    let err = TestEngine::builder()
        .catalog::<WrongTypes>()
        .build()
        .unwrap_err();

    assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    assert!(err.to_string().contains("add-numbers"));
}

// ---------------------------------------------------------------------------------------------------------------
// YAML uses the same catalog, through the spec's own `use.catalogs`
// ---------------------------------------------------------------------------------------------------------------

const YAML_USES_CATALOG: &str = r#"
document: { dsl: '1.0.3', namespace: catalog-tests, name: yaml-uses-catalog, version: '1.0.0' }
use:
  catalogs:
    local:
      endpoint:
        uri: file://./tests/rust/fixtures/catalog
do:
  - add:
      call: add-numbers:1.0.0
      with:
        a: '${ .a }'
        b: '${ .b }'
"#;

#[tokio::test(start_paused = true)]
async fn yaml_workflows_use_the_same_catalog_entries() -> TestResult {
    let engine = TestEngine::builder().build()?;
    let workflow = Workflow::<AddIn, AddOut>::from_yaml(YAML_USES_CATALOG)?;

    let sum = engine
        .execute(&workflow, AddIn { a: 20, b: 22 })
        .await?
        .await?;

    assert_eq!(sum.result, 42);
    Ok(())
}
