# Multi-stage build for rusty-blox
FROM rust:1.83-bookworm AS builder

# Install dependencies
RUN apt-get update && apt-get install -y \
    clang \
    libclang-dev \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy manifest files
COPY Cargo.toml Cargo.lock ./
COPY build.rs ./

# Copy source code
COPY src ./src
COPY sleepy2 ./sleepy2

# Build release binary
RUN cargo build --release --bin rustyblox

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    curl \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create non-root user
RUN useradd -m -u 1000 rustyblox

# Create data directory
RUN mkdir -p /data && chown rustyblox:rustyblox /data

WORKDIR /app

# Copy binary from builder
COPY --from=builder /app/target/release/rustyblox /usr/local/bin/rustyblox

# Copy default config
COPY config.toml.example /app/config.toml.example

# Switch to non-root user
USER rustyblox

# Expose API port
EXPOSE 3001

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=60s --retries=3 \
    CMD curl -f http://localhost:3001/api/v2/health || exit 1

# Run the application
CMD ["rustyblox"]
