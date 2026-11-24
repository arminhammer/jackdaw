@tasks @mcp
Feature: Calling MCP servers from a YAML workflow
  As a workflow author
  I want "call: mcp" to talk to a Model Context Protocol server
  So that a workflow can use the tools, prompts and resources the server offers

  The scenarios run against the stub server in `examples/mcp_stub_server.rs`, over both transports the specification
  defines. The stub refuses every request that comes before the `initialize` handshake.

  Background:
    Given an engine with a Redb store

  @implemented
  Scenario: The client initializes the session before its request
    Given an MCP stub server on stdio
    When a workflow runs this MCP task:
      """
      method: tools/call
      parameters:
        name: session
      transport: {transport}
      """
    Then the output at "/structuredContent/protocolVersion" is "2025-06-18"
    And the output at "/structuredContent/clientName" is "jackdaw"

  @implemented
  Scenario: The task names the protocol version and the client
    Given an MCP stub server on stdio
    When a workflow runs this MCP task:
      """
      protocolVersion: '2025-03-26'
      method: tools/call
      parameters:
        name: session
      client:
        name: order-bot
        version: 3.1.4
      transport: {transport}
      """
    Then the output at "/structuredContent/protocolVersion" is "2025-03-26"
    And the output at "/structuredContent/clientName" is "order-bot"
    And the output at "/structuredContent/clientVersion" is "3.1.4"

  @implemented
  Scenario: tools/list over stdio
    Given an MCP stub server on stdio
    When a workflow runs this MCP task:
      """
      method: tools/list
      transport: {transport}
      """
    Then the output lists the tools "echo", "session", "fail", "slow"

  @implemented
  Scenario: tools/call over stdio returns the tool's result
    Given an MCP stub server on stdio
    When a workflow runs this MCP task:
      """
      method: tools/call
      parameters:
        name: echo
        arguments:
          text: hello from the workflow
      transport: {transport}
      """
    Then the output at "/content/0/text" is "hello from the workflow"

  @implemented
  Scenario: The stdio server gets the task's environment
    Given an MCP stub server on stdio with the environment variable "STUB_GREETING" set to "bonjour"
    When a workflow runs this MCP task:
      """
      method: tools/call
      parameters:
        name: session
      transport: {transport}
      """
    Then the output at "/structuredContent/greeting" is "bonjour"

  @implemented
  Scenario: tools/list over HTTP
    Given an MCP stub server over HTTP
    When a workflow runs this MCP task:
      """
      method: tools/list
      transport: {transport}
      """
    Then the output lists the tools "echo", "session", "fail", "slow"

  @implemented
  Scenario: tools/call over HTTP returns the tool's result
    Given an MCP stub server over HTTP
    When a workflow runs this MCP task:
      """
      method: tools/call
      parameters:
        name: echo
        arguments:
          text: hello over http
      transport: {transport}
      """
    Then the output at "/content/0/text" is "hello over http"

  @implemented
  Scenario: A server that answers with an event stream
    Given an MCP stub server over HTTP answering with event streams
    When a workflow runs this MCP task:
      """
      method: tools/call
      parameters:
        name: echo
        arguments:
          text: streamed
      transport: {transport}
      """
    Then the output at "/content/0/text" is "streamed"

  @implemented
  Scenario Outline: The prompts and resources methods
    Given an MCP stub server on stdio
    When a workflow runs this MCP task:
      """
      method: <method>
      parameters: <parameters>
      transport: {transport}
      """
    Then the output at "<pointer>" is "<value>"

    Examples:
      | method                   | parameters                                | pointer                       | value              |
      | prompts/list             | {}                                        | /prompts/0/name               | greet              |
      | prompts/get              | {name: greet, arguments: {who: Ada}}      | /messages/0/content/text      | Hello, Ada!        |
      | resources/list           | {}                                        | /resources/0/uri              | stub://readme      |
      | resources/read           | {uri: 'stub://readme'}                    | /contents/0/text              | The stub's readme  |
      | resources/templates/list | {}                                        | /resourceTemplates/0/name     | notes              |

  @implemented
  Scenario: The parameters and the HTTP endpoint are runtime expressions
    Given an MCP stub server over HTTP
    And the workflow input has the server's endpoint and the message "from the input"
    When a workflow runs this MCP task:
      """
      method: tools/call
      parameters:
        name: echo
        arguments:
          text: ${ .message }
      transport:
        http:
          endpoint: ${ .endpoint }
      """
    Then the output at "/content/0/text" is "from the input"

  @implemented
  Scenario: A server error fails the task with a runtime error
    Given an MCP stub server on stdio
    When a workflow runs this MCP task:
      """
      method: tools/call
      parameters:
        name: fail
      transport: {transport}
      """
    Then the workflow fails with a runtime error that mentions "the tool exploded"

  @implemented
  Scenario: An unknown method of the server fails the task with a runtime error
    Given an MCP stub server over HTTP
    When a workflow runs this MCP task:
      """
      method: tools/call
      parameters:
        name: nothing
      transport: {transport}
      """
    Then the workflow fails with a runtime error that mentions "unknown tool"

  @implemented
  Scenario: A server that cannot be started fails the task with a runtime error
    Given an MCP stdio server whose command does not exist
    When a workflow runs this MCP task:
      """
      method: tools/list
      transport: {transport}
      """
    Then the workflow fails with a runtime error that mentions "could not start"

  @implemented
  Scenario: A server that is not there fails the task with a runtime error
    Given an MCP HTTP server that is not listening
    When a workflow runs this MCP task:
      """
      method: tools/list
      transport: {transport}
      """
    Then the workflow fails with a runtime error that mentions "initialize failed"

  @implemented
  Scenario: A call that outlasts its timeout fails the task with a runtime error
    Given an MCP stub server on stdio
    When a workflow runs this MCP task:
      """
      method: tools/call
      parameters:
        name: slow
      timeout:
        milliseconds: 500
      transport: {transport}
      """
    Then the workflow fails with a runtime error that mentions "timed out"

  @implemented
  Scenario: The timeout may be an ISO 8601 duration, and covers an HTTP call too
    Given an MCP stub server over HTTP
    When a workflow runs this MCP task:
      """
      method: tools/call
      parameters:
        name: slow
      timeout: PT0.5S
      transport: {transport}
      """
    Then the workflow fails with a runtime error that mentions "timed out"

  @implemented
  Scenario: A method the specification does not list is refused
    Given an MCP stub server on stdio
    When a workflow runs this MCP task:
      """
      method: logging/setLevel
      transport: {transport}
      """
    Then the workflow fails with a runtime error that mentions "unsupported MCP method"
