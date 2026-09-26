FROM rust:1.97-slim-trixie AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock build.rs ./
COPY assets ./assets
COPY src ./src
COPY tray ./tray
COPY web ./web
RUN cargo build --release

FROM debian:trixie-slim
RUN groupadd --system --gid 1000 swing \
    && useradd --system --uid 1000 --gid swing --no-create-home swing \
    && mkdir -p /data \
    && chown swing:swing /data
COPY --from=builder /build/target/release/swing /usr/local/bin/swing
ENV SWING_NO_PORT_SHIFT=true
USER swing
WORKDIR /data
VOLUME /data
ENTRYPOINT ["swing"]
CMD ["up"]
