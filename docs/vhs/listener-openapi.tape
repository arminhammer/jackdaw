Output docs/vhs/listener-openapi.gif

Set Shell bash
Set FontSize 14
Set Width 1400
Set Height 900

Type "# Start OpenAPI HTTP Listener (Calculator API)"
Sleep 500ms
Enter
Sleep 500ms

Type "jackdaw run examples/python-openapi-listener/calculator-api.sw.yaml &"
Sleep 500ms
Enter

Sleep 3s

Type "# Wait for server to start"
Sleep 500ms
Enter
Sleep 2s

Type "# Test Add endpoint"
Sleep 500ms
Enter
Sleep 500ms

Type "curl -X POST http://localhost:8080/api/v1/add -H \"Content-Type: application/json\" -d '{\"a\": 5, \"b\": 3}'"
Sleep 500ms
Enter

Sleep 2s

Type "# Test Multiply endpoint"
Sleep 500ms
Enter
Sleep 500ms

Type "curl -X POST http://localhost:8080/api/v1/multiply -H \"Content-Type: application/json\" -d '{\"a\": 4, \"b\": 7}'"
Sleep 500ms
Enter

Sleep 2s

Type "# Test GetPet endpoint"
Sleep 500ms
Enter
Sleep 500ms

Type "curl http://localhost:8080/api/v1/pet/2"
Sleep 500ms
Enter

Sleep 3s

Type "# Stop the listener"
Sleep 500ms
Enter
Sleep 500ms

Type "killall jackdaw"
Sleep 500ms
Enter

Sleep 1s
