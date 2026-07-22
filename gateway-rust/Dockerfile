FROM rust:1.96.0-bookworm AS builder

WORKDIR /usr/src/gateway-rust

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --locked --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder \
    /usr/src/gateway-rust/target/release/gateway-rust \
    /usr/local/bin/gateway-rust

USER 65532:65532

EXPOSE 1234

ENTRYPOINT ["/usr/local/bin/gateway-rust"]
