# Multi-stage Dockerfile for the Gyre agent (cloud deployment).
#
# Build:
#   docker build --platform linux/amd64 -t gyre:latest .
#
# Run:
#   docker run --env-file .env -p 3000:3000 gyre:latest

# Stage 1: Build
FROM rust:1.92-slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev cmake gcc g++ \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifests first for layer caching
COPY Cargo.toml Cargo.lock ./

# Copy source and build artifacts
COPY src/ src/
COPY migrations/ migrations/
COPY wit/ wit/

RUN cargo build --release --bin gyre

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/gyre /usr/local/bin/gyre
COPY --from=builder /app/migrations /app/migrations

# Non-root user
RUN useradd -m -u 1000 -s /bin/bash gyre
USER gyre

EXPOSE 3000

ENV RUST_LOG=gyre=info

HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD gyre health || exit 1

ENTRYPOINT ["gyre"]
