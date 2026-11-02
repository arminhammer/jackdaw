@interop @listeners
Feature: Listeners registered on the engine
  As a library user
  I want the engine to start workflows from HTTP and gRPC requests
  So that I can serve an API from workflows without the CLI

  @implemented @http
  Scenario: An OpenAPI listener starts a workflow per request
    Given an OpenAPI listener "api" on "127.0.0.1:0" whose operation "addNumbers" is bound to a workflow
    And an engine built with it through "EngineBuilder::listener"
    When I POST "{\"a\":2,\"b\":3}" to "/add"
    Then the response body is "{\"sum\":5}"
    And an instance of the bound workflow is in the store

  @implemented @http
  Scenario: A request that does not match the OpenAPI document is rejected
    Given an engine with an OpenAPI listener whose "addNumbers" operation has a body schema and a boolean query parameter "verbose"
    When I POST "{\"a\":2,\"b\":\"three\"}" to "/add?verbose=maybe"
    Then the response is a 400 that names "/b" and "verbose"
    And no instance of the bound workflow was started
    When I POST "{\"a\":2,\"b\":3}" to "/add?verbose=true"
    Then the response body is "{\"sum\":5}"

  @implemented @grpc
  Scenario: A gRPC listener starts a workflow per call
    Given an engine built with a gRPC listener for "calculator.proto"
    When I call "Calculator/Add" with a=2 and b=3
    Then the response's result is 5

  @implemented
  Scenario: Listeners stop on engine shutdown
    Given an engine with an HTTP listener
    When I call "Engine::shutdown()" while a request is in flight
    Then the in-flight request completes before shutdown returns
    And the listener's port is closed

  @implemented
  Scenario: The listener's bound address is available
    Given a listener on port 0
    When the engine is built
    Then "Engine::listener_addr(name)" returns the port the OS assigned

  # Covered by tests/listener_tests.rs: the YAML engine's listeners dispatch to Python and TypeScript handlers.
  @external @yaml
  Scenario: YAML workflows with listen tasks still work through the YAML engine
    Given a YAML workflow with an OpenAPI "listen" task
    When I run it through the YAML engine
    Then requests to its spec's routes start the workflow
