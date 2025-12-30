Output docs/vhs/listener-grpc.gif

Set Shell bash
Set FontSize 14
Set Width 1400
Set Height 900

Type "# Start gRPC Listener (Calculator API)"
Sleep 500ms
Enter
Sleep 500ms

Type "jackdaw run examples/python-grpc-listener/calculator-api.sw.yaml &"
Sleep 500ms
Enter

Sleep 3s

Type "# Wait for gRPC server to start"
Sleep 500ms
Enter
Sleep 2s

Type "# Test Add RPC"
Sleep 500ms
Enter
Sleep 500ms

Type 'grpcurl -plaintext -d '"'"'{"a": 5, "b": 3}'"'"' localhost:50051 calculator.Calculator/Add'
Sleep 500ms
Enter

Sleep 2s

Type "# Test Multiply RPC"
Sleep 500ms
Enter
Sleep 500ms

Type 'grpcurl -plaintext -d '"'"'{"a": 4, "b": 7}'"'"' localhost:50051 calculator.Calculator/Multiply'
Sleep 500ms
Enter

Sleep 2s

Type "# Test GetPet RPC"
Sleep 500ms
Enter
Sleep 500ms

Type 'grpcurl -plaintext -d '"'"'{"petId": "2"}'"'"' localhost:50051 calculator.Calculator/GetPet'
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
