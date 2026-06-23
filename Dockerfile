# Multi-stage build for rusty-blox
# Cargo >= 1.85 required: the lockfile pins edition2024 crates (time-core 0.1.8)
FROM rust:1.93-bookworm AS builder

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

# Expose API port (matches config.toml.example server.port)
EXPOSE 3005

# Health check
HEALTHCHECK --interval=30s --timeout=10s --start-period=60s --retries=3 \
    CMD curl -f http://localhost:3005/api/v2/health || exit 1

# Run the application
CMD ["rustyblox"]
