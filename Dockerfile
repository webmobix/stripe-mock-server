# syntax=docker/dockerfile:1

# ---------------------------------------------------------------------------
# Build stage: compile a static-ish release binary for the runtime image.
# ---------------------------------------------------------------------------
FROM rust:1-slim-bookworm AS builder

WORKDIR /build

# Compile dependencies first so that source-only changes reuse the cached
# dependency layer instead of rebuilding the whole crate graph.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && echo 'fn main() {}' > src/main.rs \
    && echo '' > src/lib.rs \
    && cargo build --release --locked \
    && rm -rf src

COPY src ./src
RUN touch src/main.rs src/lib.rs \
    && cargo build --release --locked

# ---------------------------------------------------------------------------
# Runtime stage: minimal image that runs the mock server.
# ---------------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# ca-certificates for outbound webhook TLS, curl for the healthcheck.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --system --uid 10001 --create-home --home-dir /home/stripe-mock stripe-mock

COPY --from=builder /build/target/release/stripe-mock /usr/local/bin/stripe-mock
COPY docker/config.toml /etc/stripe-mock/config.toml

# SQLite state lives here; the image sets ownership so the non-root user and
# named volumes both work out of the box.
RUN mkdir -p /data && chown -R stripe-mock:stripe-mock /data
VOLUME ["/data"]

USER stripe-mock
WORKDIR /home/stripe-mock

ENV STRIPE_MOCK_CONFIG=/etc/stripe-mock/config.toml \
    RUST_LOG=stripe_mock=info

EXPOSE 12111

HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD curl -fsS http://127.0.0.1:12111/health || exit 1

# The container config binds 0.0.0.0:12111 and stores state in /data.
ENTRYPOINT ["stripe-mock"]
