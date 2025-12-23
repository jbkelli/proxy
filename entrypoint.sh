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
    echo ""
    echo "Checking binary dependencies..."
    ldd /app/secure-proxy || echo "ldd check failed"
    echo ""
    echo "Testing if binary can execute..."
    /app/secure-proxy --help 2>&1 || echo "Binary test failed with exit code: $?"
else
    echo "✗ secure-proxy binary NOT FOUND"
fi

echo "Environment variables:"
echo "PORT=$PORT"
echo "RUST_LOG=$RUST_LOG"

# Enable Rust backtraces for better error messages
export RUST_BACKTRACE=1

# Set basic logging if not set
if [ -z "$RUST_LOG" ]; then
    export RUST_LOG=info
    echo "Set RUST_LOG=info (was empty)"
fi

echo "=== STARTING PROXY ==="
echo "Running: /app/secure-proxy"
echo "With stderr redirected to stdout for full visibility..."
exec /app/secure-proxy 2>&1
