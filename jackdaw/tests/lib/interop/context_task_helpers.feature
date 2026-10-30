@interop @ctx
Feature: Workflow context helpers for the other task types
  As a library user writing Rust workflows
  I want the Serverless Workflow task types as ctx methods
  So that I do not have to write a YAML workflow to use them

  Background:
    Given an engine with a Redb store

  @implemented @http
  Scenario: ctx.http calls an endpoint and records the response
    Given a mock HTTP server that returns 200 with body "{\"ok\":true}" for "GET /status"
    When a workflow calls "ctx.http(Request::get(url))"
    Then the step returns status 200 and the JSON body
    And replaying the instance does not call the server again

  @implemented @http
  Scenario: ctx.http applies the step's retry policy to retryable statuses
    Given a mock HTTP server that returns 503 twice and then 200
    When a workflow calls "ctx.http(..).retry(Retry::exponential(5))"
    Then the step returns 200 after 3 requests

  @implemented
  Scenario: ctx.emit publishes an event
    When a workflow calls "ctx.emit(event)"
    Then the event appears once on the run's event stream and in its history
    And replaying the instance does not emit it again

  @implemented @docker
  Scenario: ctx.run_container runs a container and returns its output
    Given Docker is available
    When a workflow calls "ctx.run_container(Container::new(\"alpine\").command([\"echo\", \"hi\"]))"
    Then the step returns stdout "hi\n" and exit code 0

  @implemented
  Scenario: ctx.run_shell runs a command and returns its output
    When a workflow calls "ctx.run_shell(Shell::new(\"echo\").args([\"hi\"]))"
    Then the step returns stdout "hi\n" and exit code 0

  @implemented
  Scenario: A non-zero exit code is a fault
    When a workflow calls "ctx.run_shell(Shell::new(\"false\"))"
    Then the step fails with a fault that carries the exit code

  @implemented
  Scenario Outline: ctx.run_script runs a script in a language
    Given "<runtime>" is installed
    When a workflow calls "ctx.run_script(Script::<language>(\"<code>\"))"
    Then the step returns stdout "<stdout>"

    Examples:
      | runtime | language   | code              | stdout |
      | python3 | python     | print(1 + 1)      | 2\n    |
      | node    | javascript | console.log(1+1)  | 2\n    |

  @implemented
  Scenario Outline: The helpers support caching like task calls
    Given a workflow that calls "<helper>" with ".cached()" twice in two instances with equal input
    Then the underlying effect happened once

    Examples:
      | helper          |
      | ctx.http        |
      | ctx.run_shell   |
      | ctx.run_script  |
