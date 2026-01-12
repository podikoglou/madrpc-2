#!/bin/bash
set -e

echo "Building MaDRPC test container image..."
docker build -t madrpc:test .
echo "Done! Image tagged as: madrpc:test"

# Verify the image was created
docker images madrpc:test
