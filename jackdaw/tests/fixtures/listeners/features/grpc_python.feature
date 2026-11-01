Feature: gRPC Calculator Service with Python TypedDict Handlers
  As an implementer of the workflow DSL
  I want to ensure that gRPC listeners can invoke Python handlers with strong typing
  So that proto message types are correctly mapped to Python TypedDict types
  # Tests gRPC listener with Python handlers for both Add and Multiply operations
  # Validates proto int32 -> Python int mapping
  # Validates TypedDict request/response handling

  Scenario: gRPC Calculator Add Operation Using Python TypedDict Handler
    Given a gRPC python workflow with definition:
      """yaml
      document:
        dsl: '1.0.1'
        namespace: calculator
        name: grpc-python-calculator-add
        version: '1.0.0'
      do:
        - performAddition:
            listen:
              to:
                any:
                  - with:
                      source:
                        uri: grpc://localhost:50051/calculator.Calculator/Add
                        schema:
                          format: proto
                          resource:
                            endpoint: tests/fixtures/listeners/specs/calculator.proto
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
                        uri: grpc://localhost:50051/calculator.Calculator/Multiply
                        schema:
                          format: proto
                          resource:
                            endpoint: tests/fixtures/listeners/specs/calculator.proto
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
    And given the gRPC add python request for "calculator.Calculator/Add" is:
      """proto
      a: 15
      b: 27
      """
    When the gRPC add python method "calculator.Calculator/Add" is called
    Then the gRPC response should be:
      """proto
      result: 42
      """

  Scenario: gRPC Calculator Multiply Operation Using Python TypedDict Handler
    Given a gRPC python workflow with definition:
      """yaml
      document:
        dsl: '1.0.1'
        namespace: calculator
        name: grpc-python-calculator-multiply
        version: '1.0.0'
      do:
        - performAddition:
            listen:
              to:
                any:
                  - with:
                      source:
                        uri: grpc://localhost:50051/calculator.Calculator/Add
                        schema:
                          format: proto
                          resource:
                            endpoint: tests/fixtures/listeners/specs/calculator.proto
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
                        uri: grpc://localhost:50051/calculator.Calculator/Multiply
                        schema:
                          format: proto
                          resource:
                            endpoint: tests/fixtures/listeners/specs/calculator.proto
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
    And given the gRPC multiply python request for "calculator.Calculator/Multiply" is:
      """proto
      a: 11
      b: 3
      """
    When the gRPC multiply python method "calculator.Calculator/Multiply" is called
    Then the gRPC response should be:
      """proto
      result: 33
      """
