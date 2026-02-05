# E2E Test Container
#
# This Dockerfile creates an isolated environment for running
# Shroud end-to-end tests. It includes all necessary system
# dependencies and ensures proper cleanup after tests.
#
# Build: docker build -t shroud-e2e -f tests/e2e/Dockerfile .
# Run:   docker run --rm --cap-add=NET_ADMIN shroud-e2e

FROM rust:1.85-slim-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    libdbus-1-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy source
COPY . .

# Build release binary and E2E test binary
RUN cargo build --release
RUN cargo test --test e2e --no-run --message-format=json 2>/dev/null | \
    grep -Po '"executable":"[^"]*e2e[^"]*"' | \
    head -1 | \
    sed 's/"executable":"//;s/"//' > /tmp/e2e_binary_path

# ==============================================================================
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    dbus \
    network-manager \
    iptables \
    ip6tables \
    iproute2 \
    procps \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create directories
RUN mkdir -p /app /var/run/dbus

WORKDIR /app

# Copy built binaries
COPY --from=builder /build/target/release/shroud /usr/local/bin/shroud
COPY --from=builder /tmp/e2e_binary_path /tmp/e2e_binary_path

# Copy test runner script
COPY tests/e2e/run-in-container.sh /app/run-in-container.sh
RUN chmod +x /app/run-in-container.sh

# Copy the E2E test binary (dynamically determined)
RUN --mount=type=bind,from=builder,source=/build/target,target=/build/target \
    cp "$(cat /tmp/e2e_binary_path)" /app/e2e-test 2>/dev/null || \
    find /build/target/debug/deps -name 'e2e-*' -type f -executable | head -1 | xargs -I{} cp {} /app/e2e-test

# Create non-root user for unprivileged tests
RUN useradd -m -s /bin/bash testuser

# Environment
ENV RUST_BACKTRACE=1
ENV SHROUD_TEST_MODE=1
ENV SHROUD_LOG_LEVEL=warn

ENTRYPOINT ["/app/run-in-container.sh"]
