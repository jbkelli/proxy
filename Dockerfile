# Build stage
FROM rust:1.83-slim as builder

# Install build dependencies
RUN apt-get update && \
    apt-get install -y pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifests
COPY Cargo.toml ./

# Create a dummy main.rs to cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs

# Build dependencies (this layer will be cached)
RUN cargo build --release
RUN rm -rf src

# Copy source code
COPY src ./src

# Build release binary
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

# Install CA certificates for HTTPS
RUN apt-get update && \
    apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy the binary from builder
COPY --from=builder /app/target/release/secure-proxy /app/secure-proxy

# Copy config file (use example as template)
COPY config.toml.example /app/config.toml

# Copy entrypoint script
COPY entrypoint.sh /app/entrypoint.sh

# Make binary and entrypoint executable
RUN chmod +x /app/secure-proxy /app/entrypoint.sh

# Expose the proxy port
EXPOSE 8080

# Run via entrypoint script for better logging
CMD ["/app/entrypoint.sh"]
