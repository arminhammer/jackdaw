Feature: gRPC Calculator Service with TypeScript Interface Handlers
  As an implementer of the workflow DSL
  I want to ensure that gRPC listeners can invoke TypeScript handlers with strong typing
  So that proto message types are correctly mapped to TypeScript interface types
  # Tests gRPC listener with TypeScript handlers for both Add and Multiply operations
  # Validates proto int32 -> TypeScript number mapping
  # Validates interface request/response handling

  Scenario: gRPC Calculator Add Operation Using TypeScript Interface Handler
    Given a gRPC typescript workflow with definition:
      """yaml
      document:
        dsl: '1.0.1'
        namespace: calculator
        name: grpc-typescript-calculator-add
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
                    call: typescript
                    with:
                      module: tests/fixtures/listeners/handlers/typescript-handlers/src/multiply.ts
                      function: handler
                      arguments:
                        - ${ $event }
      """
    And given the gRPC typescript add request for "calculator.Calculator/Add" is:
      """proto
      a: 100
      b: 50
      """
    When the gRPC typescript add method "calculator.Calculator/Add" is called
    Then the gRPC response should be:
      """proto
      result: 150
      """

  Scenario: gRPC Calculator Multiply Operation Using TypeScript Interface Handler
    Given a gRPC typescript workflow with definition:
      """yaml
      document:
        dsl: '1.0.1'
        namespace: calculator
        name: grpc-typescript-calculator-multiply
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
                    call: typescript
                    with:
                      module: tests/fixtures/listeners/handlers/typescript-handlers/src/multiply.ts
                      function: handler
                      arguments:
                        - ${ $event }
      """
    And given the gRPC typescript multiply request for "calculator.Calculator/Multiply" is:
      """proto
      a: 8
      b: 7
      """
    When the gRPC typescript multiply method "calculator.Calculator/Multiply" is called
    Then the gRPC response should be:
      """proto
      result: 56
      """
