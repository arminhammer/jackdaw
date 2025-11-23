Feature: HTTP REST Calculator API with Python TypedDict Handlers
  As an implementer of the workflow DSL
  I want to ensure that HTTP listeners can invoke Python handlers with strong typing
  So that OpenAPI schema types are correctly mapped to Python TypedDict types
  # Tests HTTP listener with Python handlers for both Add and Multiply operations
  # Validates OpenAPI int32 -> Python int mapping
  # Validates TypedDict request/response handling via REST POST

  Scenario: HTTP REST Calculator Add Operation Using Python TypedDict Handler
    Given an HTTTP python workflow with definition:
      """yaml
      document:
        dsl: '1.0.1'
        namespace: calculator
        name: http-python-calculator-add
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
                    call: python
                    with:
                      module: calculator.add
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
                    call: python
                    with:
                      module: calculator.multiply
                      function: handler
                      arguments:
                        - ${ $event }
      """
    And given the HTTP POST python add request body for "/api/v1/add" is:
      """json
      {
        "a": 15,
        "b": 27
      }
      """
    When the HTTP python add endpoint "POST /api/v1/add" is called
    Then the HTTP response status should be 200
    And the HTTP response body should be:
      """json
      {
        "result": 42
      }
      """

  Scenario: HTTP REST Calculator Multiply Operation Using Python TypedDict Handler
    Given an HTTTP python workflow with definition:
      """yaml
      document:
        dsl: '1.0.1'
        namespace: calculator
        name: http-python-calculator-multiply
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
                    call: python
                    with:
                      module: calculator.add
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
                    call: python
                    with:
                      module: calculator.multiply
                      function: handler
                      arguments:
                        - ${ $event }
      """
    And given the HTTP POST python multiply request body for "/api/v1/multiply" is:
      """json
      {
        "a": 11,
        "b": 3
      }
      """
    When the HTTP python multiply endpoint "POST /api/v1/multiply" is called
    Then the HTTP response status should be 200
    And the HTTP response body should be:
      """json
      {
        "result": 33
      }
      """
