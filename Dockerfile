# Multi-stage Dockerfile for MaDRPC
# Stage 1: Build the Rust binary
FROM rust:1.88-alpine AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev

# Set working directory
WORKDIR /build

# Copy Cargo files
COPY Cargo.toml Cargo.lock* ./
COPY crates ./crates
COPY tests ./tests

# Build the project in release mode
RUN cargo build --release

# Stage 2: Create a minimal runtime image
FROM alpine:3.20

# Install ca-certificates for HTTPS support
RUN apk add --no-cache ca-certificates

# Create a non-root user
RUN addgroup -g 1000 madrpc && \
    adduser -D -u 1000 -G madrpc madrpc

# Set working directory
WORKDIR /app

# Copy the binary from builder
COPY --from=builder /build/target/release/madrpc /usr/local/bin/madrpc

# Copy example scripts
COPY --chown=madrpc:madrpc examples /app/examples

# Switch to non-root user
USER madrpc

# Set entrypoint so commands can be specified in docker-compose
ENTRYPOINT ["madrpc"]
