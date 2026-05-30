#!/bin/bash

set -e

echo "=== Building Project ==="
cargo build

echo ""
echo "=== Running Unit Tests ==="
cargo test

echo ""
echo "=== Running REST API Tests ==="
cargo test --test rest_tests

echo ""
echo "=== All Tests Passed! ==="
