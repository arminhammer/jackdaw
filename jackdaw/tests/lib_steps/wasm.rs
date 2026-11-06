//! platform/wasm.feature: programs that use jackdaw and are built for WebAssembly.
//!
//! The programs are crates under `tests/wasm`, each with its own `[workspace]` so they build for the wasm target on
//! their own. A crate is built once per run (all its programs together), and a program is run with wasmtime.
use super::world::LibWorld;
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use tokio::process::Command;
use tokio::sync::OnceCell;

pub const TARGET: &str = "wasm32-wasip2";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn crates_dir() -> PathBuf {
    manifest_dir().join("tests/wasm")
}

/// Where the programs are built, apart from the workspace's own target directory.
fn target_dir() -> PathBuf {
    manifest_dir().join("../target/wasm-fixtures")
}

/// Whether the wasm target and wasmtime are installed here (the `@wasm` scenarios are skipped otherwise).
pub fn available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let wasmtime = std::process::Command::new("wasmtime")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        let sysroot = std::process::Command::new("rustc")
            .args(["--print", "sysroot"])
            .current_dir(crates_dir())
            .output()
            .ok()
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .map(|sysroot| {
                PathBuf::from(sysroot.trim())
                    .join("lib/rustlib")
                    .join(TARGET)
            });
        let available = wasmtime && sysroot.is_some_and(|dir| dir.is_dir());
        // The CI job for WebAssembly sets this, so a missing tool fails it instead of skipping every scenario.
        assert!(
            available || std::env::var_os("JACKDAW_REQUIRE_WASM").is_none(),
            "JACKDAW_REQUIRE_WASM is set, but the {TARGET} target or wasmtime is missing"
        );
        available
    })
}

type Built = Arc<OnceCell<Result<(), String>>>;

static BUILDS: LazyLock<Mutex<HashMap<String, Built>>> = LazyLock::new(Mutex::default);

/// Builds every program of the crate `name` for the wasm target, once per run.
async fn build(name: &str) -> Result<(), String> {
    let cell = Arc::clone(
        BUILDS
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(name.to_string())
            .or_default(),
    );
    let name = name.to_string();
    cell.get_or_init(|| async move {
        let dir = crates_dir().join(&name);
        // Start from the workspace's lock file, so the program uses the versions the rest of the tests do.
        let lock = dir.join("Cargo.lock");
        if !lock.exists() {
            let _ = std::fs::copy(manifest_dir().join("../Cargo.lock"), &lock);
        }
        let output = Command::new("cargo")
            .args(["build", "--bins", "--target", TARGET])
            .current_dir(&dir)
            .env("CARGO_TARGET_DIR", target_dir())
            .output()
            .await
            .map_err(|e| format!("cargo could not run: {e}"))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned())
        }
    })
    .await
    .clone()
}

fn program(world: &LibWorld) -> (String, String) {
    let krate = world.notes["wasm-crate"].as_str().expect("a crate");
    let name = world.notes["wasm-program"].as_str().expect("a program");
    (krate.to_string(), name.to_string())
}

#[given(regex = r#"^the program "(\w+)" of the fixture crate "(\w+)"$"#)]
fn the_program(world: &mut LibWorld, name: String, krate: String) {
    world.notes.insert("wasm-program".into(), json!(name));
    world.notes.insert("wasm-crate".into(), json!(krate));
}

#[when("I build it for WASI")]
async fn build_it(world: &mut LibWorld) {
    let (krate, _) = program(world);
    let result = build(&krate).await;
    world
        .notes
        .insert("wasm-build".into(), json!(result.err().unwrap_or_default()));
}

#[then("the WASI build succeeds")]
fn build_succeeds(world: &mut LibWorld) {
    let errors = world.notes["wasm-build"].as_str().expect("a build");
    assert!(errors.is_empty(), "the build failed:\n{errors}");
}

#[when("I run it with wasmtime")]
async fn run_it(world: &mut LibWorld) {
    run(world, false, &[]).await;
}

/// The program gets the scenario's directory (as its first argument, with access to it) and `argument`. The directory
/// stays for the scenario's later runs, so a second run finds what the first left.
#[when(regex = r#"^I run it with wasmtime, given a directory and the argument "(\w+)"$"#)]
async fn run_it_with_directory(world: &mut LibWorld, argument: String) {
    run(world, true, &[argument]).await;
}

async fn run(world: &mut LibWorld, directory: bool, arguments: &[String]) {
    let (krate, name) = program(world);
    if let Err(errors) = build(&krate).await {
        panic!("the build failed:\n{errors}");
    }
    let module = target_dir()
        .join(TARGET)
        .join("debug")
        .join(format!("{name}.wasm"));
    let mut command = Command::new("wasmtime");
    command.arg("run");
    let mut program_arguments = Vec::new();
    if directory {
        let dir = world.dir.path().display().to_string();
        command.args(["--dir", &dir]);
        program_arguments.push(dir);
    }
    program_arguments.extend_from_slice(arguments);
    command
        .arg(&module)
        .args(&program_arguments)
        .kill_on_drop(true);
    let output = tokio::time::timeout(std::time::Duration::from_secs(60), command.output())
        .await
        .unwrap_or_else(|_| panic!("{name} ran for over 60 seconds"))
        .unwrap_or_else(|e| panic!("wasmtime could not run: {e}"));
    world.notes.insert(
        "wasm-run".into(),
        json!({
            "stdout": String::from_utf8_lossy(&output.stdout),
            "stderr": String::from_utf8_lossy(&output.stderr),
            "success": output.status.success(),
        }),
    );
}

#[then(regex = r#"^it prints "(.+)"$"#)]
fn it_prints(world: &mut LibWorld, line: String) {
    let run = &world.notes["wasm-run"];
    let stdout = run["stdout"].as_str().expect("stdout");
    assert!(
        stdout.lines().any(|printed| printed == line),
        "expected the line {line:?}\nstdout:\n{stdout}\nstderr:\n{}",
        run["stderr"].as_str().expect("stderr")
    );
}

#[then("it exits successfully")]
fn exits_successfully(world: &mut LibWorld) {
    let run = &world.notes["wasm-run"];
    assert_eq!(
        run["success"],
        json!(true),
        "the program failed\nstdout:\n{}\nstderr:\n{}",
        run["stdout"].as_str().expect("stdout"),
        run["stderr"].as_str().expect("stderr")
    );
}

// ---- YAML workflows ----

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Item {
    pub sku: String,
}

/// The same task as the `yaml` program's, so the native engine can run its workflow.
#[task(name = "examples.price")]
pub async fn price(item: Item) -> Result<u64, TaskError> {
    Ok(item.sku.len() as u64 * 10)
}

/// The input the `yaml` program gives each workflow, and the label it prints the output under.
fn yaml_case(file: &str) -> (&'static str, serde_json::Value) {
    match file {
        "orders.sw.yaml" => ("orders", json!({ "items": [10, 20, 30] })),
        "switch.sw.yaml" => ("switch", json!({ "n": 60 })),
        other => panic!("no case for {other}"),
    }
}

/// The output the native engine gives for a workflow of the `defaults` fixture crate.
async fn native_output(file: &str, input: serde_json::Value) -> serde_json::Value {
    let yaml = std::fs::read_to_string(crates_dir().join("defaults").join(file))
        .unwrap_or_else(|e| panic!("{file}: {e}"));
    let workflow = Workflow::<serde_json::Value, serde_json::Value>::from_yaml(&yaml)
        .expect("the workflow parses");
    let engine = Engine::builder()
        .task(Price)
        .yaml_workflow(&workflow)
        .build()
        .expect("the engine builds");
    engine
        .execute(&workflow, input)
        .await
        .expect("the workflow starts")
        .await
        .expect("the workflow completes")
}

#[then(regex = r#"^it prints the output that the native engine gives for "(.+)"$"#)]
async fn prints_native_output(world: &mut LibWorld, file: String) {
    let (label, input) = yaml_case(&file);
    let run = &world.notes["wasm-run"];
    let stdout = run["stdout"].as_str().expect("stdout");
    let prefix = format!("{label}=");
    let printed = stdout
        .lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| {
            panic!(
                "no `{prefix}` line\nstdout:\n{stdout}\nstderr:\n{}",
                run["stderr"].as_str().expect("stderr")
            )
        });
    let on_wasm: serde_json::Value = serde_json::from_str(printed).expect("the output is JSON");
    assert_eq!(on_wasm, native_output(&file, input).await);
    world.notes.insert("wasm-output".into(), on_wasm);
}

#[then(regex = r#"^that output has "(\w+)" "(\w+)"$"#)]
fn output_has_text(world: &mut LibWorld, field: String, expected: String) {
    let output = &world.notes["wasm-output"];
    assert_eq!(output[&field], json!(expected), "{output}");
}

/// Whether `number` appears anywhere in `value`.
fn mentions(value: &serde_json::Value, number: i64) -> bool {
    match value {
        serde_json::Value::Number(n) => n.as_i64() == Some(number),
        serde_json::Value::Array(items) => items.iter().any(|item| mentions(item, number)),
        serde_json::Value::Object(fields) => fields.values().any(|item| mentions(item, number)),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::String(_) => {
            false
        }
    }
}

#[then(regex = r"^that output has the prices (\d+) and (\d+)$")]
fn output_has_prices(world: &mut LibWorld, first: i64, second: i64) {
    let output = &world.notes["wasm-output"];
    assert!(
        mentions(output, first) && mentions(output, second),
        "expected the prices {first} and {second} in {output}"
    );
}

#[then(regex = r"^that output does not have the price (\d+)$")]
fn output_lacks_price(world: &mut LibWorld, price: i64) {
    let output = &world.notes["wasm-output"];
    assert!(
        !mentions(output, price),
        "unexpected price {price} in {output}"
    );
}

// ---- what wasm cannot do ----

#[then(regex = r#"^the "(\w+)" workflow fails with "(.+)"$"#)]
fn workflow_fails_with(world: &mut LibWorld, label: String, message: String) {
    let run = &world.notes["wasm-run"];
    let stdout = run["stdout"].as_str().expect("stdout");
    let prefix = format!("{label}=");
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| {
            panic!(
                "no `{prefix}` line\nstdout:\n{stdout}\nstderr:\n{}",
                run["stderr"].as_str().expect("stderr")
            )
        });
    assert!(line.contains(&message), "expected {message:?} in {line:?}");
}

#[when(regex = r#"^I build it for WASI with the feature "([\w-]+)"$"#)]
async fn build_with_feature(world: &mut LibWorld, feature: String) {
    let (krate, _) = program(world);
    let output = Command::new("cargo")
        .args([
            "build",
            "--bins",
            "--target",
            TARGET,
            "--features",
            &feature,
        ])
        .current_dir(crates_dir().join(&krate))
        .env("CARGO_TARGET_DIR", target_dir())
        .output()
        .await
        .unwrap_or_else(|e| panic!("cargo could not run: {e}"));
    let errors = if output.status.success() {
        String::new()
    } else {
        String::from_utf8_lossy(&output.stderr).into_owned()
    };
    world.notes.insert("wasm-build".into(), json!(errors));
}

#[then(regex = r#"^the WASI build fails with "(.+)"$"#)]
fn build_fails_with(world: &mut LibWorld, message: String) {
    let errors = world.notes["wasm-build"].as_str().expect("a build");
    assert!(!errors.is_empty(), "the build succeeded");
    assert!(
        errors.contains(&message),
        "expected {message:?} in the build's errors:\n{errors}"
    );
}
