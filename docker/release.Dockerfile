FROM debian:trixie-slim
ARG TARGETARCH
RUN groupadd --system --gid 1000 swing \
    && useradd --system --uid 1000 --gid swing --no-create-home swing \
    && mkdir -p /data \
    && chown swing:swing /data
COPY ${TARGETARCH}/swing /usr/local/bin/swing
USER swing
WORKDIR /data
VOLUME /data
ENTRYPOINT ["swing"]
CMD ["up"]
