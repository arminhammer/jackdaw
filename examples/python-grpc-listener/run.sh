#!/bin/bash
set -e

# Check if jackdaw:latest exists
if ! docker images | grep -q "jackdaw.*latest"; then
    echo "Error: jackdaw:latest image not found"
    echo "Please build it first with: just docker-build"
    exit 1
fi

echo "Building calculator-grpc-api image..."
docker build -t calculator-grpc-api -f examples/python-grpc-listener/Dockerfile examples/python-grpc-listener

echo ""
echo "Starting calculator-grpc-api container..."
echo "gRPC server will be available on localhost:50051"
echo ""
echo "Test with grpcurl:"
echo "  grpcurl -plaintext -d '{\"a\": 5, \"b\": 3}' localhost:50051 calculator.Calculator/Add"
echo "  grpcurl -plaintext -d '{\"a\": 7, \"b\": 6}' localhost:50051 calculator.Calculator/Multiply"
echo "  grpcurl -plaintext -d '{\"pet_id\": 1}' localhost:50051 calculator.Calculator/GetPet"
echo ""

docker run --rm -p 50051:50051 calculator-grpc-api