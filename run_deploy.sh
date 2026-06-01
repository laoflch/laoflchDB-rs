#!/bin/bash

set -e

echo "=== laoflchDB Deployment Script ==="

if [ "$1" == "build" ]; then
    echo "Building project..."
    cargo build --release
    exit 0
fi

if [ "$1" == "docker-build" ]; then
    echo "Building Docker image..."
    docker build -t laoflchdb-rust:latest .
    exit 0
fi

if [ "$1" == "docker-run" ]; then
    echo "Running Docker container..."
    docker-compose up -d
    exit 0
fi

if [ "$1" == "docker-stop" ]; then
    echo "Stopping Docker container..."
    docker-compose down
    exit 0
fi

if [ "$1" == "docker-logs" ]; then
    echo "Showing Docker logs..."
    docker-compose logs -f
    exit 0
fi

if [ "$1" == "docker-status" ]; then
    echo "Checking Docker container status..."
    docker-compose ps
    exit 0
fi

if [ "$1" == "deploy" ]; then
    echo "=== Full Deployment ==="
    echo "1. Building project..."
    cargo build --release
    
    echo "2. Building Docker image..."
    docker build -t laoflchdb-rust:latest .
    
    echo "3. Stopping existing container..."
    docker-compose down 2>/dev/null || true
    
    echo "4. Running container..."
    docker-compose up -d
    
    echo "5. Waiting for service to start..."
    sleep 10
    
    echo "6. Checking health status..."
    curl -f http://localhost:8080/health && echo "" || echo "Service not ready yet"
    
    echo "=== Deployment completed ==="
    exit 0
fi

echo "Usage: $0 <command>"
echo ""
echo "Commands:"
echo "  build          - Build Rust project"
echo "  docker-build   - Build Docker image"
echo "  docker-run     - Run Docker container"
echo "  docker-stop    - Stop Docker container"
echo "  docker-logs    - Show Docker logs"
echo "  docker-status  - Check container status"
echo "  deploy         - Full deployment (build + docker-build + run)"
