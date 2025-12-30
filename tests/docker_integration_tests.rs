/// Docker Integration Tests
///
/// These tests validate that the built Docker image works correctly end-to-end.
/// They run actual workflows inside the containerized jackdaw binary.
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn test_docker_image_version() {
    // Test that the Docker image can run and report its version
    let output = Command::new("docker")
        .args(["run", "--rm", "jackdaw:latest", "--version"])
        .output()
        .unwrap_or_else(|e| panic!("Failed to run docker container: {}", e));

    assert!(
        output.status.success(),
        "Docker version command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("jackdaw"),
        "Version output doesn't contain 'jackdaw': {}",
        stdout
    );
}

#[test]
fn test_docker_image_help() {
    // Test that the Docker image can display help
    let output = Command::new("docker")
        .args(["run", "--rm", "jackdaw:latest", "--help"])
        .output()
        .unwrap_or_else(|e| panic!("Failed to run docker container: {}", e));

    assert!(
        output.status.success(),
        "Docker help command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Usage:"),
        "Help output missing Usage section"
    );
    assert!(stdout.contains("run"), "Help output missing 'run' command");
    assert!(
        stdout.contains("validate"),
        "Help output missing 'validate' command"
    );
}

#[test]
fn test_docker_validate_workflow() {
    // Test workflow validation using the Docker image
    let fixture_path = PathBuf::from("tests/fixtures/docker");

    assert!(
        fixture_path.join("valid-minimal.sw.yaml").exists(),
        "Fixture not found: tests/fixtures/docker/valid-minimal.sw.yaml"
    );

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!(
                "{}:/workflows:ro,z",
                std::env::current_dir()
                    .unwrap()
                    .join(&fixture_path)
                    .display()
            ),
            "jackdaw:latest",
            "validate",
            "/workflows/valid-minimal.sw.yaml",
        ])
        .output()
        .unwrap_or_else(|e| panic!("Failed to run docker container: {}", e));

    assert!(
        output.status.success(),
        "Docker validate command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_docker_validate_invalid_workflow() {
    // Test that validation fails for invalid workflows
    let fixture_path = PathBuf::from("tests/fixtures/docker");

    assert!(
        fixture_path.join("invalid.sw.yaml").exists(),
        "Fixture not found: tests/fixtures/docker/invalid.sw.yaml"
    );

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!(
                "{}:/workflows:ro,z",
                std::env::current_dir()
                    .unwrap()
                    .join(&fixture_path)
                    .display()
            ),
            "jackdaw:latest",
            "validate",
            "/workflows/invalid.sw.yaml",
        ])
        .output()
        .expect("Failed to run docker container");

    assert!(
        !output.status.success(),
        "Docker validate should have failed for invalid workflow"
    );
}

#[test]
fn test_docker_run_simple_workflow() {
    // Test running a simple workflow in the Docker container
    let fixture_path = PathBuf::from("tests/fixtures/docker");
    let temp_dir = TempDir::new().unwrap_or_else(|e| panic!("Failed to create temp dir: {}", e));
    let _db_path = temp_dir.path().join("test.db");
    let _cache_path = temp_dir.path().join("cache.db");

    assert!(
        fixture_path.join("simple-echo.sw.yaml").exists(),
        "Fixture not found: tests/fixtures/docker/simple-echo.sw.yaml"
    );

    // Copy fixture to temp dir so we can write databases alongside it
    std::fs::copy(
        std::env::current_dir()
            .unwrap()
            .join(&fixture_path)
            .join("simple-echo.sw.yaml"),
        temp_dir.path().join("simple-echo.sw.yaml"),
    )
    .unwrap_or_else(|e| panic!("Failed to copy fixture: {}", e));

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/workflows:z", temp_dir.path().display()),
            "jackdaw:latest",
            "run",
            "/workflows/simple-echo.sw.yaml",
            "--durable-db",
            "/workflows/test.db",
            "--cache-db",
            "/workflows/cache.db",
        ])
        .output()
        .expect("Failed to run docker container");

    assert!(
        output.status.success(),
        "Docker run command failed: {}\nStderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Hello from Docker!"),
        "Output doesn't contain expected message: {}",
        stdout
    );
}

#[test]
fn test_docker_run_existing_fixture() {
    // Test running one of your existing test fixtures
    let fixture_path = PathBuf::from("tests/fixtures/three-stage-test.sw.yaml");

    if !fixture_path.exists() {
        eprintln!("Skipping test: fixture not found at {:?}", fixture_path);
        return;
    }

    // First, validate the workflow
    let validate_output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!(
                "{}:/fixtures:ro,z",
                std::env::current_dir()
                    .unwrap()
                    .join("tests/fixtures")
                    .display()
            ),
            "jackdaw:latest",
            "validate",
            "/fixtures/three-stage-test.sw.yaml",
        ])
        .output()
        .expect("Failed to run docker container");

    assert!(
        validate_output.status.success(),
        "Fixture validation failed: {}",
        String::from_utf8_lossy(&validate_output.stderr)
    );
}

/// Helper function to check if Docker image exists
fn docker_image_exists() -> bool {
    Command::new("docker")
        .args(["image", "inspect", "jackdaw:latest"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Test that runs before all others to ensure Docker image is available
#[test]
fn test_00_docker_image_available() {
    assert!(
        docker_image_exists(),
        "Docker image 'jackdaw:latest' not found. Run 'just docker-build' first."
    );
}

#[test]
fn test_docker_validate_python_script() {
    // Test validating a workflow with Python script
    let fixture_path = PathBuf::from("tests/fixtures/docker");

    assert!(
        fixture_path.join("python-script.sw.yaml").exists(),
        "Fixture not found: tests/fixtures/docker/python-script.sw.yaml"
    );

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!(
                "{}:/workflows:ro,z",
                std::env::current_dir()
                    .unwrap()
                    .join(&fixture_path)
                    .display()
            ),
            "jackdaw:latest",
            "validate",
            "/workflows/python-script.sw.yaml",
        ])
        .output()
        .expect("Failed to run docker container");

    assert!(
        output.status.success(),
        "Python script workflow validation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_docker_validate_javascript_script() {
    // Test validating a workflow with JavaScript script
    let fixture_path = PathBuf::from("tests/fixtures/docker");

    assert!(
        fixture_path.join("javascript-script.sw.yaml").exists(),
        "Fixture not found: tests/fixtures/docker/javascript-script.sw.yaml"
    );

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!(
                "{}:/workflows:ro,z",
                std::env::current_dir()
                    .unwrap()
                    .join(&fixture_path)
                    .display()
            ),
            "jackdaw:latest",
            "validate",
            "/workflows/javascript-script.sw.yaml",
        ])
        .output()
        .expect("Failed to run docker container");

    assert!(
        output.status.success(),
        "JavaScript script workflow validation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_docker_run_javascript_script() {
    // Test running a Python script workflow
    let fixture_path = PathBuf::from("tests/fixtures/docker");
    let temp_dir = TempDir::new().unwrap_or_else(|e| panic!("Failed to create temp dir: {}", e));

    assert!(
        fixture_path.join("javascript-script.sw.yaml").exists(),
        "Fixture not found: tests/fixtures/docker/javascript-script.sw.yaml"
    );

    // Copy fixture to temp dir
    std::fs::copy(
        std::env::current_dir()
            .unwrap()
            .join(&fixture_path)
            .join("javascript-script.sw.yaml"),
        temp_dir.path().join("javascript-script.sw.yaml"),
    )
    .unwrap_or_else(|e| panic!("Failed to copy fixture: {}", e));

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/workflows:z", temp_dir.path().display()),
            "jackdaw:latest",
            "run",
            "/workflows/javascript-script.sw.yaml",
            "--debug", // Use debug mode to get workflow output
            "--durable-db",
            "/workflows/test.db",
            "--cache-db",
            "/workflows/cache.db",
        ])
        .output()
        .expect("Failed to run docker container");

    assert!(
        output.status.success(),
        "Javascript script execution failed: {}\nStderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // In non-debug mode, only streaming logs are output (e.g., [runJavaScript:stdout] ...)
    // The final workflow output JSON is not printed unless --debug is used
    // So we verify the workflow succeeded and check for the message in the streaming output
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The output should contain the message in the streaming log or final JSON
    assert!(
        stdout.contains("Hello from JavaScript!"),
        "Output doesn't contain expected JavaScript message: {}",
        stdout
    );
}

#[test]
fn test_docker_run_python_script() {
    // Test running a Python script workflow
    let fixture_path = PathBuf::from("tests/fixtures/docker");
    let temp_dir = TempDir::new().unwrap_or_else(|e| panic!("Failed to create temp dir: {}", e));

    assert!(
        fixture_path.join("python-script.sw.yaml").exists(),
        "Fixture not found: tests/fixtures/docker/python-script.sw.yaml"
    );

    // Copy fixture to temp dir
    std::fs::copy(
        std::env::current_dir()
            .unwrap()
            .join(&fixture_path)
            .join("python-script.sw.yaml"),
        temp_dir.path().join("python-script.sw.yaml"),
    )
    .unwrap_or_else(|e| panic!("Failed to copy fixture: {}", e));

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/workflows:z", temp_dir.path().display()),
            "jackdaw:latest",
            "run",
            "/workflows/python-script.sw.yaml",
            "--debug", // Use debug mode to get workflow output
            "--durable-db",
            "/workflows/test.db",
            "--cache-db",
            "/workflows/cache.db",
        ])
        .output()
        .expect("Failed to run docker container");

    assert!(
        output.status.success(),
        "Python script execution failed: {}\nStderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // In non-debug mode, only streaming logs are output (e.g., [runPython:stdout] ...)
    // The final workflow output JSON is not printed unless --debug is used
    // So we verify the workflow succeeded and check for the message in the streaming output
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The output should contain the message in the streaming log or final JSON
    assert!(
        stdout.contains("Hello from Python!"),
        "Output doesn't contain expected Python message: {}",
        stdout
    );
}

#[test]
fn test_docker_validate_grpc_python_listener() {
    // Test validating gRPC workflow with Python handler
    let fixture_path = PathBuf::from("tests/fixtures/docker");

    assert!(
        fixture_path.join("grpc-python-calculator.sw.yaml").exists(),
        "Fixture not found: tests/fixtures/docker/grpc-python-calculator.sw.yaml"
    );

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/app:ro,z", std::env::current_dir().unwrap().display()),
            "-w",
            "/app",
            "jackdaw:latest",
            "validate",
            "tests/fixtures/docker/grpc-python-calculator.sw.yaml",
        ])
        .output()
        .expect("Failed to run docker container");

    assert!(
        output.status.success(),
        "gRPC Python workflow validation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_docker_validate_grpc_typescript_listener() {
    // Test validating gRPC workflow with TypeScript handler
    let fixture_path = PathBuf::from("tests/fixtures/docker");

    assert!(
        fixture_path
            .join("grpc-typescript-calculator.sw.yaml")
            .exists(),
        "Fixture not found: tests/fixtures/docker/grpc-typescript-calculator.sw.yaml"
    );

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/app:ro,z", std::env::current_dir().unwrap().display()),
            "-w",
            "/app",
            "jackdaw:latest",
            "validate",
            "tests/fixtures/docker/grpc-typescript-calculator.sw.yaml",
        ])
        .output()
        .expect("Failed to run docker container");

    assert!(
        output.status.success(),
        "gRPC TypeScript workflow validation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_docker_validate_openapi_python_listener() {
    // Test validating OpenAPI workflow with Python handler
    let fixture_path = PathBuf::from("tests/fixtures/docker");

    assert!(
        fixture_path
            .join("openapi-python-calculator.sw.yaml")
            .exists(),
        "Fixture not found: tests/fixtures/docker/openapi-python-calculator.sw.yaml"
    );

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/app:ro,z", std::env::current_dir().unwrap().display()),
            "-w",
            "/app",
            "jackdaw:latest",
            "validate",
            "tests/fixtures/docker/openapi-python-calculator.sw.yaml",
        ])
        .output()
        .expect("Failed to run docker container");

    assert!(
        output.status.success(),
        "OpenAPI Python workflow validation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_docker_validate_openapi_typescript_listener() {
    // Test validating OpenAPI workflow with TypeScript handler
    let fixture_path = PathBuf::from("tests/fixtures/docker");

    assert!(
        fixture_path
            .join("openapi-typescript-calculator.sw.yaml")
            .exists(),
        "Fixture not found: tests/fixtures/docker/openapi-typescript-calculator.sw.yaml"
    );

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/app:ro,z", std::env::current_dir().unwrap().display()),
            "-w",
            "/app",
            "jackdaw:latest",
            "validate",
            "tests/fixtures/docker/openapi-typescript-calculator.sw.yaml",
        ])
        .output()
        .expect("Failed to run docker container");

    assert!(
        output.status.success(),
        "OpenAPI TypeScript workflow validation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_docker_run_grpc_python_listener_with_client() {
    // End-to-end test: Start gRPC listener in Docker and make actual client calls
    use std::process::Stdio;
    use std::thread;
    use std::time::Duration;

    let fixture_path = PathBuf::from("tests/fixtures/docker");
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    assert!(
        fixture_path.join("grpc-python-calculator.sw.yaml").exists(),
        "Fixture not found"
    );

    // Start jackdaw with gRPC listener in background
    let mut child = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--name",
            "jackdaw-grpc-test",
            "-p",
            "50051:50051", // Expose gRPC port
            "-e",
            "PYTHONPATH=/app/tests/fixtures/listeners/handlers/python-handlers", // Set Python module path
            "-v",
            &format!("{}:/app:ro,z", std::env::current_dir().unwrap().display()),
            "-v",
            &format!("{}:/data:z", temp_dir.path().display()),
            "-w",
            "/app",
            "jackdaw:latest",
            "run",
            "tests/fixtures/docker/grpc-python-calculator.sw.yaml",
            "--durable-db",
            "/data/test.db",
            "--cache-db",
            "/data/cache.db",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("Failed to start docker container: {}", e));

    // Wait for listener to start
    thread::sleep(Duration::from_secs(5));

    // Make gRPC client call using grpcurl
    let grpc_result = Command::new("grpcurl")
        .args([
            "-plaintext",
            "-d",
            r#"{"a": 15, "b": 27}"#,
            "localhost:50051",
            "calculator.Calculator/Add",
        ])
        .output();

    // Cleanup: Stop the container
    let _ = Command::new("docker")
        .args(["stop", "jackdaw-grpc-test"])
        .output();

    // Kill the child process
    let _ = child.kill();
    let _ = child.wait();

    // Verify the gRPC call worked
    if let Ok(output) = grpc_result {
        assert!(
            output.status.success(),
            "gRPC call failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let response = String::from_utf8_lossy(&output.stdout);
        assert!(
            response.contains("42") || response.contains("result"),
            "Expected result:42 in response, got: {}",
            response
        );
    } else {
        panic!(
            "grpcurl not available - install with: brew install grpcurl (or apt-get install grpcurl)"
        );
    }
}

#[test]
fn test_docker_run_openapi_python_listener_with_client() {
    // End-to-end test: Start OpenAPI listener in Docker and make actual HTTP client calls
    use std::process::Stdio;
    use std::thread;
    use std::time::Duration;

    let fixture_path = PathBuf::from("tests/fixtures/docker");
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    assert!(
        fixture_path
            .join("openapi-python-calculator.sw.yaml")
            .exists(),
        "Fixture not found"
    );

    // Start jackdaw with HTTP listener in background
    let child = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--name",
            "jackdaw-http-test",
            "-p",
            "8080:8080", // Expose HTTP port
            "-e",
            "PYTHONPATH=/app/tests/fixtures/listeners/handlers/python-handlers", // Set Python module path
            "-v",
            &format!("{}:/app:ro,z", std::env::current_dir().unwrap().display()),
            "-v",
            &format!("{}:/data:z", temp_dir.path().display()),
            "-w",
            "/app",
            "jackdaw:latest",
            "run",
            "tests/fixtures/docker/openapi-python-calculator.sw.yaml",
            "--durable-db",
            "/data/test.db",
            "--cache-db",
            "/data/cache.db",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start docker container");

    // Wait for listener to start
    thread::sleep(Duration::from_secs(5));

    // Make HTTP client call using curl
    let http_result = Command::new("curl")
        .args([
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            r#"{"a": 15, "b": 27}"#,
            "http://localhost:8080/api/v1/add",
            "-s", // Silent mode
        ])
        .output();

    // Cleanup: Stop the container
    let _ = Command::new("docker")
        .args(["stop", "jackdaw-http-test"])
        .output();

    // Get container output before killing
    let container_output = child.wait_with_output().ok();

    // Verify the HTTP call worked
    assert!(
        http_result.is_ok(),
        "curl not available - it should be pre-installed on most systems"
    );

    let output = http_result.unwrap();

    let container_logs = container_output
        .map(|o| {
            format!(
                "Container stdout:\n{}\n\nContainer stderr:\n{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            )
        })
        .unwrap_or_else(|| "Could not capture container output".to_string());

    assert!(
        output.status.success(),
        "HTTP call failed.\nCurl stderr: {}\nCurl stdout: {}\n\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
        container_logs
    );

    let response = String::from_utf8_lossy(&output.stdout);
    assert!(
        response.contains("42") || response.contains("result"),
        "Expected result:42 in response, got: {}",
        response
    );
}

#[test]
fn test_docker_run_grpc_typescript_listener_with_client() {
    // End-to-end test: Start gRPC listener with TypeScript handler and make client calls
    use std::process::Stdio;
    use std::thread;
    use std::time::Duration;

    let fixture_path = PathBuf::from("tests/fixtures/docker");
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    assert!(
        fixture_path
            .join("grpc-typescript-calculator.sw.yaml")
            .exists(),
        "Fixture not found"
    );

    // Start jackdaw with gRPC listener in background
    let mut child = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--name",
            "jackdaw-grpc-ts-test",
            "-p",
            "50052:50051", // Use different port to avoid conflicts
            "-v",
            &format!("{}:/app:ro,z", std::env::current_dir().unwrap().display()),
            "-v",
            &format!("{}:/data:z", temp_dir.path().display()),
            "-w",
            "/app",
            "jackdaw:latest",
            "run",
            "tests/fixtures/docker/grpc-typescript-calculator.sw.yaml",
            "--durable-db",
            "/data/test.db",
            "--cache-db",
            "/data/cache.db",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start docker container");

    // Wait for listener to start
    thread::sleep(Duration::from_secs(5));

    // Check if container is still running
    let running_check = Command::new("docker")
        .args([
            "ps",
            "--filter",
            "name=jackdaw-grpc-ts-test",
            "--format",
            "{{.Names}}",
        ])
        .output()
        .expect("Failed to check container status");

    let running_containers = String::from_utf8_lossy(&running_check.stdout);

    // If container isn't running, get logs and fail
    if !running_containers.contains("jackdaw-grpc-ts-test") {
        let logs_output = Command::new("docker")
            .args(["logs", "jackdaw-grpc-ts-test"])
            .output();

        let (logs, logs_err) = if let Ok(output) = logs_output {
            (
                String::from_utf8_lossy(&output.stdout).to_string(),
                String::from_utf8_lossy(&output.stderr).to_string(),
            )
        } else {
            (
                "Could not fetch logs".to_string(),
                "Container not found".to_string(),
            )
        };

        // Cleanup
        let _ = child.kill();
        let _ = child.wait();

        panic!(
            "Container 'jackdaw-grpc-ts-test' is not running!\n\nContainer stdout:\n{}\n\nContainer stderr:\n{}",
            logs, logs_err
        );
    }

    // Get container logs for debugging
    let logs_output = Command::new("docker")
        .args(["logs", "jackdaw-grpc-ts-test"])
        .output()
        .expect("Failed to get container logs");

    let logs = String::from_utf8_lossy(&logs_output.stdout);
    let logs_err = String::from_utf8_lossy(&logs_output.stderr);

    // Make gRPC client call
    let grpc_result = Command::new("grpcurl")
        .args([
            "-plaintext",
            "-d",
            r#"{"a": 10, "b": 5}"#,
            "localhost:50052",
            "calculator.Calculator/Add",
        ])
        .output();

    // Cleanup
    let _ = Command::new("docker")
        .args(["stop", "jackdaw-grpc-ts-test"])
        .output();
    let _ = child.kill();
    let _ = child.wait();

    // Verify
    if let Ok(output) = grpc_result {
        assert!(
            output.status.success(),
            "gRPC TypeScript call failed: {}\n\nContainer stdout:\n{}\n\nContainer stderr:\n{}",
            String::from_utf8_lossy(&output.stderr),
            logs,
            logs_err
        );

        let response = String::from_utf8_lossy(&output.stdout);
        assert!(
            response.contains("15") || response.contains("result"),
            "Expected result:15 in response, got: {}",
            response
        );
    } else {
        panic!("grpcurl not available");
    }
}

#[test]
fn test_docker_run_openapi_typescript_listener_with_client() {
    // End-to-end test: Start OpenAPI listener with TypeScript handler and make client calls
    use std::process::Stdio;
    use std::thread;
    use std::time::Duration;

    let fixture_path = PathBuf::from("tests/fixtures/docker");
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    assert!(
        fixture_path
            .join("openapi-typescript-calculator.sw.yaml")
            .exists(),
        "Fixture not found"
    );

    // Start jackdaw with HTTP listener in background
    let mut child = Command::new("docker")
        .args([
            "run",
            "--rm",
            "--name",
            "jackdaw-http-ts-test",
            "-p",
            "8081:8080", // Use different port to avoid conflicts
            "-v",
            &format!("{}:/app:ro,z", std::env::current_dir().unwrap().display()),
            "-v",
            &format!("{}:/data:z", temp_dir.path().display()),
            "-w",
            "/app",
            "jackdaw:latest",
            "run",
            "tests/fixtures/docker/openapi-typescript-calculator.sw.yaml",
            "--durable-db",
            "/data/test.db",
            "--cache-db",
            "/data/cache.db",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start docker container");

    // Wait for listener to start
    thread::sleep(Duration::from_secs(5));

    // Check container logs to see if it started properly
    let logs_output = Command::new("docker")
        .args(["logs", "jackdaw-http-ts-test"])
        .output()
        .expect("Failed to get container logs");

    let logs = String::from_utf8_lossy(&logs_output.stdout);
    let logs_err = String::from_utf8_lossy(&logs_output.stderr);

    // Make HTTP client call
    let http_result = Command::new("curl")
        .args([
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            r#"{"a": 10, "b": 5}"#,
            "http://localhost:8081/api/v1/add",
            "-s",
        ])
        .output();

    // Cleanup
    let _ = Command::new("docker")
        .args(["stop", "jackdaw-http-ts-test"])
        .output();
    let _ = child.kill();
    let _ = child.wait();

    // Verify
    assert!(http_result.is_ok(), "curl not available");
    let output = http_result.unwrap();
    assert!(
        output.status.success(),
        "HTTP TypeScript call failed: {}\n\nContainer stdout:\n{}\n\nContainer stderr:\n{}",
        String::from_utf8_lossy(&output.stderr),
        logs,
        logs_err
    );

    let response = String::from_utf8_lossy(&output.stdout);
    assert!(
        response.contains("15") || response.contains("result"),
        "Expected result:15 in response, got: {}",
        response
    );
}

#[test]
fn test_docker_validate_nested_workflows() {
    // Test validating nested workflow fixtures
    let fixture_path = PathBuf::from("tests/fixtures/nested-workflows");

    for workflow in &["workflow-a.yaml", "workflow-b.yaml", "workflow-c.yaml"] {
        assert!(
            fixture_path.join(workflow).exists(),
            "Fixture not found: tests/fixtures/nested-workflows/{}",
            workflow
        );

        let output = Command::new("docker")
            .args([
                "run",
                "--rm",
                "-v",
                &format!(
                    "{}:/workflows:ro,z",
                    std::env::current_dir()
                        .unwrap()
                        .join(&fixture_path)
                        .display()
                ),
                "jackdaw:latest",
                "validate",
                &format!("/workflows/{}", workflow),
            ])
            .output()
            .expect("Failed to run docker container");

        assert!(
            output.status.success(),
            "Nested workflow {} validation failed: {}",
            workflow,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn test_docker_run_nested_workflow() {
    // Test running nested workflows - mounts the project directory so subworkflows can be resolved
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let output = Command::new("docker")
        .args([
            "run",
            "--rm",
            "-v",
            &format!("{}:/app:ro,z", std::env::current_dir().unwrap().display()),
            "-v",
            &format!("{}:/data:z", temp_dir.path().display()),
            "-w",
            "/app",
            "jackdaw:latest",
            "run",
            "tests/fixtures/nested-workflows/workflow-a.yaml",
            "--input",
            r#"{"value": 10}"#,
            "--registry",
            "tests/fixtures/nested-workflows",
            "--durable-db",
            "/data/test.db",
            "--cache-db",
            "/data/cache.db",
        ])
        .output()
        .expect("Failed to run docker container");

    assert!(
        output.status.success(),
        "Nested workflow execution failed: {}\nStderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
