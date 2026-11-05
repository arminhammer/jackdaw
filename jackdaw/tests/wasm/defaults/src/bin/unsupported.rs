//! YAML tasks that need a child process or an HTTP client: wasm has neither, so each workflow fails, and the failure
//! says why. Prints `<name>=<error>` for each.
use jackdaw::prelude::*;
use serde_json::{Value, json};

const SHELL: &str = r#"
document: { dsl: '1.0.3', namespace: examples, name: shell, version: '1.0.0' }
do:
  - say:
      run:
        shell:
          command: echo
          arguments: [hello]
"#;

const HTTP: &str = r#"
document: { dsl: '1.0.3', namespace: examples, name: http, version: '1.0.0' }
do:
  - fetch:
      call: http
      with:
        method: get
        endpoint: https://example.com/
"#;

const PYTHON: &str = r#"
document: { dsl: '1.0.3', namespace: examples, name: python, version: '1.0.0' }
do:
  - script:
      run:
        script:
          language: python
          code: print(1)
"#;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Error> {
    for (name, yaml) in [("shell", SHELL), ("http", HTTP), ("python", PYTHON)] {
        let workflow = Workflow::<Value, Value>::from_yaml(yaml)?;
        let engine = Engine::builder().yaml_workflow(&workflow).build()?;
        match engine.execute(&workflow, json!({})).await?.await {
            Ok(output) => println!("{name}=it ran: {output}"),
            Err(error) => println!("{name}={error}"),
        }
    }
    Ok(())
}
