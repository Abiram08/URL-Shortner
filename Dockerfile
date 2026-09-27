# ============================================
# Shorty — production Docker image
# Builds a static Rust binary and runs it as a non-root user.
# ============================================

FROM rust:1.89-slim AS builder

WORKDIR /app

# Install build dependencies for native-tls (OpenSSL).
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Cache dependency build layer.
# Dependencies only rebuild when Cargo.toml or Cargo.lock changes.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Copy the rest of the application source and rebuild.
COPY . .
# The dependency cache layer above leaves a dummy binary with a newer mtime,
# so touch main.rs to force cargo to rebuild the actual application.
RUN touch src/main.rs
RUN cargo build --release

FROM debian:bookworm-slim

# Runtime TLS support and CA certs for Postgres/Redis TLS (e.g. Upstash, Railway).
RUN apt-get update && apt-get install -y ca-certificates curl libssl3 && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Running as root inside a container is a security risk — if the app is ever
# compromised an attacker gets root. We create a dedicated user instead.
RUN groupadd --system shorty && useradd --system --gid shorty shorty

COPY --from=builder /app/target/release/shorty .
COPY public ./public
RUN chown -R shorty:shorty /app

USER shorty

EXPOSE 3000

# In production we want fast startup, no auto-restart watchers.
CMD ["./shorty"]
