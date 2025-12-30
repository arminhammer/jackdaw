#!/bin/bash
# Build and run the Calculator API example

set -e

echo "=== Python OpenAPI Listener Example ==="
echo ""

# Check if jackdaw:latest exists
if ! docker images | grep -q "jackdaw.*latest"; then
    echo "Error: jackdaw:latest image not found"
    echo "Please build it first with: just docker-build"
    exit 1
fi

# Build the calculator API image
echo "Building calculator-api image..."
docker build -t calculator-api -f examples/python-openapi-listener/Dockerfile examples/python-openapi-listener

echo ""
echo "Starting calculator API server..."
echo "API will be available at http://localhost:8080"
echo ""
echo "Test with:"
echo "  curl -X POST http://localhost:8080/api/v1/add -H 'Content-Type: application/json' -d '{\"a\": 15, \"b\": 27}'"
echo "  curl -X POST http://localhost:8080/api/v1/multiply -H 'Content-Type: application/json' -d '{\"a\": 7, \"b\": 6}'"
echo ""
echo "Press Ctrl+C to stop"
echo ""

# Run the container
docker run --rm -p 8080:8080 calculator-api
