//! interop/listeners.feature: listeners registered on the engine.
use super::world::LibWorld;
use cucumber::{given, then, when};
use jackdaw::api::{ListFilter, Listener};
use jackdaw::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Pair {
    pub a: i64,
    pub b: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Sum {
    pub sum: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AddResponse {
    pub result: i64,
}

#[workflow(namespace = "listen", name = "add", version = "1")]
pub struct AddNumbers;

#[workflow_methods]
impl AddNumbers {
    #[run]
    pub async fn run(_ctx: &mut WorkflowContext<Self>, pair: Pair) -> WorkflowResult<Sum> {
        Ok(Sum {
            sum: pair.a + pair.b,
        })
    }
}

/// Takes a moment, so a request can be in flight when the engine shuts down.
#[workflow(namespace = "listen", name = "slow-add", version = "1")]
pub struct SlowAdd;

#[workflow_methods]
impl SlowAdd {
    #[run]
    pub async fn run(ctx: &mut WorkflowContext<Self>, pair: Pair) -> WorkflowResult<Sum> {
        ctx.sleep(500.millis()).await?;
        Ok(Sum {
            sum: pair.a + pair.b,
        })
    }
}

#[workflow(namespace = "listen", name = "grpc-add", version = "1")]
pub struct GrpcAdd;

#[workflow_methods]
impl GrpcAdd {
    #[run]
    pub async fn run(_ctx: &mut WorkflowContext<Self>, pair: Pair) -> WorkflowResult<AddResponse> {
        Ok(AddResponse {
            result: pair.a + pair.b,
        })
    }
}

const SPEC: &str = r#"
openapi: 3.0.0
info: { title: calculator, version: '1' }
paths:
  /add:
    post:
      operationId: addNumbers
      responses: { '200': { description: the sum } }
"#;

fn proto() -> String {
    format!(
        "{}/tests/fixtures/listeners/specs/calculator.proto",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// The scenario's engine: in memory, on the system clock (the listener's requests run in real time).
fn build(world: &mut LibWorld, listener: Listener) {
    let engine = Engine::builder()
        .workflow::<AddNumbers>()
        .workflow::<SlowAdd>()
        .workflow::<GrpcAdd>()
        .listener(listener)
        .build()
        .expect("the engine builds");
    world.engine = Some(engine);
}

fn addr(world: &mut LibWorld) -> std::net::SocketAddr {
    world
        .engine()
        .listener_addr("api")
        .expect("the listener is serving")
}

async fn post(addr: std::net::SocketAddr, path: &str, body: &str) -> (u16, String) {
    let response = reqwest::Client::new()
        .post(format!("http://{addr}{path}"))
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .expect("the request is sent");
    let status = response.status().as_u16();
    (status, response.text().await.expect("a body"))
}

#[given(
    regex = r#"^an OpenAPI listener "api" on "127\.0\.0\.1:0" whose operation "addNumbers" is bound to a workflow$"#
)]
fn openapi_listener(world: &mut LibWorld) {
    world.notes.insert("kind".into(), json!("openapi"));
}

#[given(regex = r#"^an engine built with it through "EngineBuilder::listener"$"#)]
fn built_with_listener(world: &mut LibWorld) {
    let listener = Listener::openapi("api", SPEC, "127.0.0.1:0")
        .expect("the spec parses")
        .bind("addNumbers", AddNumbers::run);
    build(world, listener);
}

#[when(regex = r#"^I POST "(.+)" to "(/\w+)"$"#)]
async fn post_to(world: &mut LibWorld, body: String, path: String) {
    let addr = addr(world);
    let (status, text) = post(addr, &path, &body.replace("\\\"", "\"")).await;
    world.notes.insert("status".into(), json!(status));
    world.notes.insert("response".into(), json!(text));
}

#[then(regex = r#"^the response body is "(.+)"$"#)]
fn response_body(world: &mut LibWorld, expected: String) {
    assert_eq!(
        world.notes["status"],
        json!(200),
        "{}",
        world.notes["response"]
    );
    let body: Value =
        serde_json::from_str(world.notes["response"].as_str().expect("a body")).expect("JSON");
    let expected: Value = serde_json::from_str(&expected.replace("\\\"", "\"")).expect("JSON");
    assert_eq!(body, expected);
}

#[then("an instance of the bound workflow is in the store")]
async fn instance_in_store(world: &mut LibWorld) {
    let instances = world.engine().list(&ListFilter::new()).await.expect("list");
    assert!(
        instances
            .iter()
            .any(|instance| instance.workflow.name == "add" && instance.status == Status::Completed),
        "{instances:?}"
    );
}

#[given(regex = r#"^an engine built with a gRPC listener for "calculator\.proto"$"#)]
fn grpc_listener(world: &mut LibWorld) {
    let listener = Listener::grpc("api", proto(), "calculator.Calculator", "127.0.0.1:0")
        .expect("the proto compiles")
        .bind("Add", GrpcAdd::run);
    build(world, listener);
}

#[when(regex = r#"^I call "Calculator/Add" with a=(\d+) and b=(\d+)$"#)]
async fn call_add(world: &mut LibWorld, a: i64, b: i64) {
    use prost::Message as _;
    let addr = addr(world);
    let files = protox::compile(
        ["calculator.proto"],
        [format!(
            "{}/tests/fixtures/listeners/specs",
            env!("CARGO_MANIFEST_DIR")
        )],
    )
    .expect("the proto compiles");
    let pool = prost_reflect::DescriptorPool::from_file_descriptor_set(files).expect("a pool");
    let method = pool
        .get_service_by_name("calculator.Calculator")
        .expect("the service")
        .methods()
        .find(|method| method.name() == "Add")
        .expect("Add");
    let request =
        prost_reflect::DynamicMessage::deserialize(method.input(), json!({ "a": a, "b": b }))
            .expect("a request")
            .encode_to_vec();
    // A gRPC frame: no compression, the length (big-endian), the message.
    let mut frame = vec![0];
    frame.extend_from_slice(&u32::try_from(request.len()).expect("small").to_be_bytes());
    frame.extend_from_slice(&request);
    let response = reqwest::Client::builder()
        .http2_prior_knowledge()
        .build()
        .expect("a client")
        .post(format!("http://{addr}/calculator.Calculator/Add"))
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .body(frame)
        .send()
        .await
        .expect("the call is sent")
        .bytes()
        .await
        .expect("a body");
    let message = prost_reflect::DynamicMessage::decode(
        method.output(),
        response.get(5..).unwrap_or_default(),
    )
    .expect("an AddResponse");
    world.notes.insert(
        "response".into(),
        serde_json::to_value(&message).expect("JSON"),
    );
}

#[then(regex = r#"^the response's result is (\d+)$"#)]
fn grpc_result(world: &mut LibWorld, expected: i64) {
    assert_eq!(
        world.notes["response"]["result"],
        json!(expected),
        "{}",
        world.notes["response"]
    );
}

#[given("an engine with an HTTP listener")]
fn http_listener(world: &mut LibWorld) {
    let listener = Listener::openapi("api", SPEC, "127.0.0.1:0")
        .expect("the spec parses")
        .bind("addNumbers", SlowAdd::run);
    build(world, listener);
}

#[when(regex = r#"^I call "Engine::shutdown\(\)" while a request is in flight$"#)]
async fn shutdown_in_flight(world: &mut LibWorld) {
    let addr = addr(world);
    world.notes.insert("addr".into(), json!(addr.to_string()));
    let engine = world.engine();
    let request = tokio::spawn(post(addr, "/add", r#"{"a":2,"b":3}"#));
    // In flight: its workflow is running.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while engine
        .list(&ListFilter::new())
        .await
        .expect("list")
        .iter()
        .all(|instance| instance.status != Status::Running)
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the request never started its workflow"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    engine.shutdown(Duration::from_secs(10)).await;
    world.notes.insert(
        "finished-before-shutdown-returned".into(),
        json!(request.is_finished()),
    );
    let (status, text) = request.await.expect("the request task");
    world.notes.insert("status".into(), json!(status));
    world.notes.insert("response".into(), json!(text));
}

#[then("the in-flight request completes before shutdown returns")]
fn in_flight_completed(world: &mut LibWorld) {
    assert_eq!(
        world.notes["finished-before-shutdown-returned"],
        json!(true)
    );
    assert_eq!(
        world.notes["status"],
        json!(200),
        "{}",
        world.notes["response"]
    );
    let body: Value =
        serde_json::from_str(world.notes["response"].as_str().expect("a body")).expect("JSON");
    assert_eq!(body, json!({ "sum": 5 }));
}

#[then("the listener's port is closed")]
async fn port_closed(world: &mut LibWorld) {
    let addr: std::net::SocketAddr = world.notes["addr"]
        .as_str()
        .expect("an address")
        .parse()
        .expect("an address");
    assert!(
        tokio::net::TcpStream::connect(addr).await.is_err(),
        "something still accepts on {addr}"
    );
    assert_eq!(world.engine().listener_addr("api"), None);
}

#[given("a listener on port 0")]
fn port_zero(world: &mut LibWorld) {
    world.notes.insert("kind".into(), json!("port-0"));
}

#[when("the engine is built")]
fn engine_is_built(world: &mut LibWorld) {
    let listener = Listener::openapi("api", SPEC, "127.0.0.1:0")
        .expect("the spec parses")
        .bind("addNumbers", AddNumbers::run);
    build(world, listener);
}

#[then(regex = r#"^"Engine::listener_addr\(name\)" returns the port the OS assigned$"#)]
async fn assigned_port(world: &mut LibWorld) {
    let addr = addr(world);
    assert_ne!(addr.port(), 0);
    let (status, text) = post(addr, "/add", r#"{"a":1,"b":1}"#).await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(world.engine().listener_addr("nope"), None);
}
