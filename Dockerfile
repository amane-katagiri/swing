FROM rust:1.97-slim-trixie AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY web ./web
RUN cargo build --release

FROM debian:trixie-slim
RUN groupadd --system --gid 1000 swing \
    && useradd --system --uid 1000 --gid swing --no-create-home swing \
    && mkdir -p /data \
    && chown swing:swing /data
COPY --from=builder /build/target/release/swing /usr/local/bin/swing
USER swing
VOLUME /data
ENTRYPOINT ["swing"]
CMD ["up"]
