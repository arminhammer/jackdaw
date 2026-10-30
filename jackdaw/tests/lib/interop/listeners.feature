@interop @listeners
Feature: Listeners registered on the engine
  As a library user
  I want the engine to start workflows from HTTP and gRPC requests
  So that I can serve an API from workflows without the CLI

  @todo @http
  Scenario: An OpenAPI listener starts a workflow per request
    Given an engine built with "EngineBuilder::listener(Listener::openapi(spec, \"127.0.0.1:0\"))"
    And the operation "addNumbers" is bound to a workflow
    When I POST "{\"a\":2,\"b\":3}" to "/add"
    Then the response body is "{\"sum\":5}"
    And an instance of the bound workflow is in the store

  @todo @grpc
  Scenario: A gRPC listener starts a workflow per call
    Given an engine built with a gRPC listener for "calculator.proto"
    When I call "Calculator/Add" with a=2 and b=3
    Then the response has sum 5

  @todo
  Scenario: Listeners stop on engine shutdown
    Given an engine with an HTTP listener
    When I call "Engine::shutdown()"
    Then the listener's port is closed
    And in-flight requests complete before shutdown returns

  @todo
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
