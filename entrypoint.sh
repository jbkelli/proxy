#!/bin/bash
set -e

echo "=== ENTRYPOINT SCRIPT STARTING ==="
echo "Current directory: $(pwd)"
echo "Files in /app:"
ls -la /app/

echo "Checking config.toml..."
if [ -f "/app/config.toml" ]; then
    echo "✓ config.toml exists"
    echo "First few lines:"
    head -5 /app/config.toml
else
    echo "✗ config.toml NOT FOUND"
fi

echo "Checking binary..."
if [ -f "/app/secure-proxy" ]; then
    echo "✓ secure-proxy binary exists"
    ls -lh /app/secure-proxy
else
    echo "✗ secure-proxy binary NOT FOUND"
fi

echo "Environment variables:"
echo "PORT=$PORT"
echo "RUST_LOG=$RUST_LOG"

echo "=== STARTING PROXY ==="
exec /app/secure-proxy
