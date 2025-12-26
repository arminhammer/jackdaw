#!/bin/bash
set -e

# Check if jackdaw:latest exists
if ! docker images | grep -q "jackdaw.*latest"; then
    echo "Error: jackdaw:latest image not found"
    echo "Please build it first with: just docker-build"
    exit 1
fi

echo "Building calculator-js-api image..."
docker build -t calculator-js-api -f examples/javascript-openapi-listener/Dockerfile examples/javascript-openapi-listener

echo ""
echo "Starting calculator-js-api container..."
echo "HTTP server will be available on localhost:8080"
echo ""
echo "Test with curl:"
echo "  curl -X POST http://localhost:8080/api/v1/add -H 'Content-Type: application/json' -d '{\"a\": 5, \"b\": 3}'"
echo "  curl -X POST http://localhost:8080/api/v1/multiply -H 'Content-Type: application/json' -d '{\"a\": 7, \"b\": 6}'"
echo "  curl http://localhost:8080/api/v1/pet/1"
echo ""

docker run --rm -p 8080:8080 calculator-js-api
