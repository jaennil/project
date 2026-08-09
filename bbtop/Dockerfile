FROM rust:1.96-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN useradd --system --uid 65532 --no-create-home bbtop
COPY --from=builder /build/target/release/bbtop /usr/local/bin/bbtop
USER bbtop
EXPOSE 9099
ENTRYPOINT ["bbtop"]
CMD ["--no-tui", "--listen", "0.0.0.0:9099"]

