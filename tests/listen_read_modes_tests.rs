#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]

/// Tests for Listen Task Read Modes
///
/// Per the Serverless Workflow spec, listen tasks support three read modes:
/// - "data": Return only the event data field
/// - "envelope": Return the full CloudEvent structure
/// - "raw": Return the raw HTTP body (before CloudEvent parsing)
use serde_json::json;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

use jackdaw::cache::CacheProvider;
use jackdaw::durableengine::DurableEngine;
use jackdaw::persistence::PersistenceProvider;
use jackdaw::providers::cache::RedbCache;
use jackdaw::providers::persistence::RedbPersistence;
use jackdaw::workflow_source::FilesystemSource;
use jackdaw::DurableEngineBuilder;

/// Helper to set up test infrastructure
async fn setup_test_engine(temp_dir: &TempDir) -> DurableEngine {
    let db_path = temp_dir.path().join("test.db");
    let persistence = Arc::new(RedbPersistence::new(db_path.to_str().unwrap()).unwrap());
    let cache =
        Arc::new(RedbCache::new(Arc::clone(&persistence.db)).unwrap()) as Arc<dyn CacheProvider>;
    DurableEngineBuilder::new()
        .with_persistence(Arc::clone(&persistence) as Arc<dyn PersistenceProvider>)
        .with_cache(Arc::clone(&cache))
        .build()
        .unwrap()
}

#[tokio::test]
async fn test_listen_read_mode_data() {
    // Set up Python path for handler
    let handlers_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/handlers");
    unsafe {
        env::set_var("PYTHONPATH", handlers_dir.to_str().unwrap());
    }

    // Setup test infrastructure
    let temp_dir = tempfile::tempdir().unwrap();
    let engine = setup_test_engine(&temp_dir).await;

    let fixture = PathBuf::from("tests/fixtures/listen-read-modes/test-listen-read-data.sw.yaml");
    let source = FilesystemSource::new(fixture);

    // Start workflow (this initializes the listener) - keep handle alive so workflow continues
    let handle = engine.execute(source, json!({})).await.expect("Failed to start workflow");

    // Give listener time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Send a CloudEvent to the listener
    let client = reqwest::Client::new();
    let cloud_event = json!({
        "specversion": "1.0",
        "type": "test.event.v1",
        "source": "test",
        "id": "test-123",
        "data": {
            "message": "Hello from CloudEvent data"
        }
    });

    let response = client
        .post("http://localhost:8081/webhook")
        .json(&cloud_event)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200, "Listener request failed");

    let response_body: serde_json::Value = response.json().await.unwrap();

    // In "data" mode, the handler should only receive the data field
    // The echo handler returns what it received, so we verify it got only the data
    assert_eq!(
        response_body.get("message").and_then(|v| v.as_str()),
        Some("Hello from CloudEvent data"),
        "Read mode 'data' should extract only the data field"
    );

    // Should NOT contain CloudEvent envelope fields
    assert!(
        response_body.get("specversion").is_none(),
        "Data mode should not include CloudEvent envelope"
    );
}

#[tokio::test]
async fn test_listen_read_mode_envelope() {
    // Set up Python path for handler
    let handlers_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/handlers");
    unsafe {
        env::set_var("PYTHONPATH", handlers_dir.to_str().unwrap());
    }

    // Setup test infrastructure
    let temp_dir = tempfile::tempdir().unwrap();
    let engine = setup_test_engine(&temp_dir).await;

    let fixture =
        PathBuf::from("tests/fixtures/listen-read-modes/test-listen-read-envelope.sw.yaml");
    let source = FilesystemSource::new(fixture);

    // Start workflow - keep handle alive so workflow continues
    let handle = engine.execute(source, json!({})).await.expect("Failed to start workflow");

    // Give listener time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Send a CloudEvent to the listener
    let client = reqwest::Client::new();
    let cloud_event = json!({
        "specversion": "1.0",
        "type": "test.event.v1",
        "source": "test-source",
        "id": "test-456",
        "data": {
            "message": "Nested data"
        }
    });

    let response = client
        .post("http://localhost:8082/webhook")
        .json(&cloud_event)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200, "Listener request failed");

    let response_body: serde_json::Value = response.json().await.unwrap();

    // In "envelope" mode, the handler should receive the full CloudEvent structure
    assert_eq!(
        response_body.get("specversion").and_then(|v| v.as_str()),
        Some("1.0"),
        "Envelope mode should include CloudEvent specversion"
    );
    assert_eq!(
        response_body.get("type").and_then(|v| v.as_str()),
        Some("test.event.v1"),
        "Envelope mode should include CloudEvent type"
    );
    assert_eq!(
        response_body.get("source").and_then(|v| v.as_str()),
        Some("test-source"),
        "Envelope mode should include CloudEvent source"
    );
    assert_eq!(
        response_body.get("id").and_then(|v| v.as_str()),
        Some("test-456"),
        "Envelope mode should include CloudEvent id"
    );
    assert_eq!(
        response_body
            .get("data")
            .and_then(|v| v.get("message"))
            .and_then(|v| v.as_str()),
        Some("Nested data"),
        "Envelope mode should include CloudEvent data"
    );
}

#[tokio::test]
async fn test_listen_read_mode_raw() {
    // Set up Python path for handler
    let handlers_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/handlers");
    unsafe {
        env::set_var("PYTHONPATH", handlers_dir.to_str().unwrap());
    }

    // Setup test infrastructure
    let temp_dir = tempfile::tempdir().unwrap();
    let engine = setup_test_engine(&temp_dir).await;

    let fixture = PathBuf::from("tests/fixtures/listen-read-modes/test-listen-read-raw.sw.yaml");
    let source = FilesystemSource::new(fixture);

    // Start workflow - don't wait for completion as it's perpetual
    let _handle = engine.execute(source, json!({})).await.expect("Failed to start workflow");

    // Give listener time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Send raw JSON (not necessarily a CloudEvent) to the listener
    let client = reqwest::Client::new();
    let raw_payload = json!({
        "customField": "custom value",
        "nested": {
            "data": "arbitrary structure"
        }
    });

    let response = client
        .post("http://localhost:8083/webhook")
        .json(&raw_payload)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200, "Listener request failed");

    let response_body: serde_json::Value = response.json().await.unwrap();

    // In "raw" mode, the handler should receive the raw HTTP body without CloudEvent parsing
    assert_eq!(
        response_body.get("customField").and_then(|v| v.as_str()),
        Some("custom value"),
        "Raw mode should pass through raw request body"
    );
    assert_eq!(
        response_body
            .get("nested")
            .and_then(|v| v.get("data"))
            .and_then(|v| v.as_str()),
        Some("arbitrary structure"),
        "Raw mode should preserve nested structure"
    );
}

#[tokio::test]
async fn test_listen_read_mode_default_is_envelope() {
    // Set up Python path for handler
    let handlers_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/handlers");
    unsafe {
        env::set_var("PYTHONPATH", handlers_dir.to_str().unwrap());
    }

    // Setup test infrastructure
    let temp_dir = tempfile::tempdir().unwrap();
    let engine = setup_test_engine(&temp_dir).await;

    let fixture =
        PathBuf::from("tests/fixtures/listen-read-modes/test-listen-read-default.sw.yaml");
    let source = FilesystemSource::new(fixture);

    // Start workflow - don't wait for completion as it's perpetual
    let _handle = engine.execute(source, json!({})).await.expect("Failed to start workflow");

    // Give listener time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    // Send a CloudEvent to the listener
    let client = reqwest::Client::new();
    let cloud_event = json!({
        "specversion": "1.0",
        "type": "test.event.v1",
        "source": "default-test",
        "id": "test-789",
        "data": {
            "message": "Default mode test"
        }
    });

    let response = client
        .post("http://localhost:8084/webhook")
        .json(&cloud_event)
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200, "Listener request failed");

    let response_body: serde_json::Value = response.json().await.unwrap();

    // Default should be "envelope" mode - full CloudEvent structure
    assert_eq!(
        response_body.get("specversion").and_then(|v| v.as_str()),
        Some("1.0"),
        "Default read mode should be envelope with CloudEvent specversion"
    );
    assert_eq!(
        response_body.get("id").and_then(|v| v.as_str()),
        Some("test-789"),
        "Default read mode should be envelope with CloudEvent id"
    );
}
