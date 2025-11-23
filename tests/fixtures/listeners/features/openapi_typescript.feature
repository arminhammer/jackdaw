Feature: HTTP REST Calculator API with TypeScript Interface Handlers
  As an implementer of the workflow DSL
  I want to ensure that HTTP listeners can invoke TypeScript handlers with strong typing
  So that OpenAPI schema types are correctly mapped to TypeScript interface types
  # Tests HTTP listener with TypeScript handlers for both Add and Multiply operations
  # Validates OpenAPI int32 -> TypeScript number mapping
  # Validates interface request/response handling via REST POST

  Scenario: HTTP REST Calculator Add Operation Using TypeScript Interface Handler
    Given an HTTP typescript workflow with definition:
      """yaml
      document:
        dsl: '1.0.1'
        namespace: calculator
        name: http-typescript-calculator-add
        version: '1.0.0'
      do:
        - performAddition:
            listen:
              to:
                any:
                  - with:
                      source:
                        uri: http://localhost:8080/api/v1/add
                        schema:
                          format: openapi
                          resource:
                            endpoint: tests/fixtures/listeners/specs/calculator.yaml
                            name: AddRequest
                until: '${ false }'
            foreach:
              item: event
              do:
                - executeAdd:
                    call: typescript
                    with:
                      module: tests/fixtures/listeners/handlers/typescript-handlers/src/add.ts
                      function: handler
                      arguments:
                        - ${ $event }
        - performMultiplication:
            listen:
              to:
                any:
                  - with:
                      source:
                        uri: http://localhost:8080/api/v1/multiply
                        schema:
                          format: openapi
                          resource:
                            endpoint: tests/fixtures/listeners/specs/calculator.yaml
                            name: MultiplyRequest
                until: '${ false }'
            foreach:
              item: event
              do:
                - executeMultiply:
                    call: typescript
                    with:
                      module: tests/fixtures/listeners/handlers/typescript-handlers/src/multiply.ts
                      function: handler
                      arguments:
                        - ${ $event }
      """
    And given the HTTP POST typescript add request body for "/api/v1/add" is:
      """json
      {
        "a": 100,
        "b": 50
      }
      """
    When the HTTP typescript add endpoint "POST /api/v1/add" is called
    Then the HTTP response status should be 200
    And the HTTP response body should be:
      """json
      {
        "result": 150
      }
      """

  Scenario: HTTP REST Calculator Multiply Operation Using TypeScript Interface Handler
    Given an HTTP typescript workflow with definition:
      """yaml
      document:
        dsl: '1.0.1'
        namespace: calculator
        name: http-typescript-calculator-multiply
        version: '1.0.0'
      do:
        - performAddition:
            listen:
              to:
                any:
                  - with:
                      source:
                        uri: http://localhost:8080/api/v1/add
                        schema:
                          format: openapi
                          resource:
                            endpoint: tests/fixtures/listeners/specs/calculator.yaml
                            name: AddRequest
                until: '${ false }'
            foreach:
              item: event
              do:
                - executeAdd:
                    call: typescript
                    with:
                      module: tests/fixtures/listeners/handlers/typescript-handlers/src/add.ts
                      function: handler
                      arguments:
                        - ${ $event }
        - performMultiplication:
            listen:
              to:
                any:
                  - with:
                      source:
                        uri: http://localhost:8080/api/v1/multiply
                        schema:
                          format: openapi
                          resource:
                            endpoint: tests/fixtures/listeners/specs/calculator.yaml
                            name: MultiplyRequest
                until: '${ false }'
            foreach:
              item: event
              do:
                - executeMultiply:
                    call: typescript
                    with:
                      module: tests/fixtures/listeners/handlers/typescript-handlers/src/multiply.ts
                      function: handler
                      arguments:
                        - ${ $event }
      """
    And given the HTTP POST typescript multiply request body for "/api/v1/multiply" is:
      """json
      {
        "a": 8,
        "b": 7
      }
      """
    When the HTTP typescript multiply endpoint "POST /api/v1/multiply" is called
    Then the HTTP response status should be 200
    And the HTTP response body should be:
      """json
      {
        "result": 56
      }
      """
