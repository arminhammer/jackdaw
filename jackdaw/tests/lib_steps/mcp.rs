//! tasks/mcp_calls.feature: `call: mcp` against the stub server in `examples/mcp_stub_server.rs`.
use super::world::LibWorld;
use cucumber::{given, then, when};
use jackdaw::prelude::*;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

/// The HTTP stub servers a scenario started; they are killed with the scenario.
#[derive(Default)]
pub struct State {
    servers: Vec<Child>,
}

impl Drop for State {
    fn drop(&mut self) {
        for server in &mut self.servers {
            let _ = server.kill();
            let _ = server.wait();
        }
    }
}

impl std::fmt::Debug for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("State").finish_non_exhaustive()
    }
}

/// The stub server's binary: cargo builds the examples next to the test binaries.
fn stub() -> String {
    let exe = std::env::current_exe().expect("the test binary");
    let path = exe
        .parent()
        .and_then(std::path::Path::parent)
        .expect("target/debug")
        .join("examples")
        .join("mcp_stub_server");
    assert!(
        path.exists(),
        "build the stub with `cargo build --example mcp_stub_server`: {path:?}"
    );
    path.display().to_string()
}

fn set_transport(world: &mut LibWorld, transport: &Value) {
    world
        .notes
        .insert("mcp-transport".into(), transport.clone());
}

#[given("an MCP stub server on stdio")]
fn stdio(world: &mut LibWorld) {
    set_transport(
        world,
        &json!({"stdio": {"command": stub(), "arguments": ["stdio"]}}),
    );
}

#[given(
    regex = r#"^an MCP stub server on stdio with the environment variable "(.+)" set to "(.+)"$"#
)]
fn stdio_env(world: &mut LibWorld, name: String, value: String) {
    set_transport(
        world,
        &json!({"stdio": {"command": stub(), "arguments": ["stdio"], "environment": {name: value}}}),
    );
}

fn start_http(world: &mut LibWorld, mode: &str) {
    let mut child = Command::new(stub())
        .arg(mode)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("the stub starts");
    let mut line = String::new();
    BufReader::new(child.stdout.take().expect("stdout"))
        .read_line(&mut line)
        .expect("the stub says where it listens");
    let port = line.trim().trim_start_matches("listening ").to_string();
    world.mcp.servers.push(child);
    let endpoint = format!("http://127.0.0.1:{port}/mcp");
    world.notes.insert("mcp-endpoint".into(), json!(endpoint));
    set_transport(world, &json!({"http": {"endpoint": endpoint}}));
}

#[given("an MCP stub server over HTTP")]
fn http(world: &mut LibWorld) {
    start_http(world, "http");
}

#[given("an MCP stub server over HTTP answering with event streams")]
fn http_sse(world: &mut LibWorld) {
    start_http(world, "http-sse");
}

#[given("an MCP stdio server whose command does not exist")]
fn missing_command(world: &mut LibWorld) {
    set_transport(
        world,
        &json!({"stdio": {"command": "/nonexistent/mcp-server"}}),
    );
}

#[given("an MCP HTTP server that is not listening")]
fn not_listening(world: &mut LibWorld) {
    // A port that was free a moment ago.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("a free port")
        .local_addr()
        .expect("an address")
        .port();
    set_transport(
        world,
        &json!({"http": {"endpoint": format!("http://127.0.0.1:{port}/mcp")}}),
    );
}

#[given(regex = r#"^the workflow input has the server's endpoint and the message "(.+)"$"#)]
fn input(world: &mut LibWorld, message: String) {
    let endpoint = world.notes["mcp-endpoint"].clone();
    world.notes.insert(
        "mcp-input".into(),
        json!({"endpoint": endpoint, "message": message}),
    );
}

#[when("a workflow runs this MCP task:")]
async fn run(world: &mut LibWorld, step: &cucumber::gherkin::Step) {
    let body = step.docstring.as_ref().expect("a task body");
    let transport = world
        .notes
        .get("mcp-transport")
        .map_or_else(String::new, Value::to_string);
    let body = body.replace("{transport}", &transport);
    let indented: String = body
        .lines()
        .map(|line| format!("        {line}\n"))
        .collect();
    let yaml = format!(
        "document:\n  dsl: '1.0.2'\n  namespace: test\n  name: mcp-call\n  version: '0.1.0'\ndo:\n  - mcpCall:\n      call: mcp\n      with:\n{indented}"
    );
    let workflow = Workflow::<Value, Value>::from_yaml(&yaml).expect("parses");
    let input = world
        .notes
        .get("mcp-input")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let engine = world.engine();
    let result = match engine.execute(&workflow, input).await {
        Ok(handle) => handle.await,
        Err(error) => Err(error),
    };
    match result {
        Ok(output) => world.notes.insert("mcp-output".into(), output),
        Err(error) => world
            .notes
            .insert("mcp-error".into(), json!(format!("{error:?}"))),
    };
}

fn output(world: &LibWorld) -> &Value {
    world
        .notes
        .get("mcp-output")
        .unwrap_or_else(|| panic!("the workflow failed: {:?}", world.notes.get("mcp-error")))
}

#[then(regex = r#"^the output at "(.+)" is "(.*)"$"#)]
fn output_at(world: &mut LibWorld, pointer: String, expected: String) {
    let output = output(world);
    assert_eq!(output.pointer(&pointer), Some(&json!(expected)), "{output}");
}

#[then(regex = r#"^the output lists the tools (.+)$"#)]
fn lists_tools(world: &mut LibWorld, names: String) {
    let expected: Vec<String> = names
        .split(", ")
        .map(|n| n.trim_matches('"').to_string())
        .collect();
    let tools: Vec<String> = output(world)["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|tool| tool["name"].as_str().expect("a name").to_string())
        .collect();
    assert_eq!(tools, expected);
}

#[then(regex = r#"^the workflow fails with a runtime error that mentions "(.+)"$"#)]
fn fails(world: &mut LibWorld, text: String) {
    let error = world
        .notes
        .get("mcp-error")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            panic!(
                "the workflow succeeded: {:?}",
                world.notes.get("mcp-output")
            )
        });
    assert!(error.contains("errors/runtime"), "{error}");
    assert!(error.contains(&text), "{error}");
}
