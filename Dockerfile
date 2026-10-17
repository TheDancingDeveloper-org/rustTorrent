# syntax=docker/dockerfile:1
# Multi-stage build: compile rtbit with webui from source, output minimal scratch image.

FROM rust:1.99-alpine AS builder

RUN apk update && apk add clang lld npm pkgconf musl-dev openssl-dev openssl-libs-static curl git

COPY . /src/
WORKDIR /src/

ENV OPENSSL_STATIC=1

# Remove desktop workspace member (excluded by .dockerignore, needs glib)
RUN sed -i '/"desktop\/src-tauri"/d' Cargo.toml

RUN --mount=type=cache,target=/usr/local/cargo/git/db \
    --mount=type=cache,target=/usr/local/cargo/registry/cache \
    --mount=type=cache,target=/usr/local/cargo/registry/index \
    cargo build --profile release-github && \
    cp target/release-github/rtbit /bin/rtbit

# ---

FROM lscr.io/linuxserver/baseimage-alpine:3.21

LABEL org.opencontainers.image.source="https://github.com/TheDancingDeveloper-org/rustTorrent"
LABEL org.opencontainers.image.url="https://github.com/TheDancingDeveloper-org/rustTorrent"
LABEL org.opencontainers.image.title="rustTorrent"
LABEL org.opencontainers.image.description="A modern BitTorrent client written in Rust"

COPY --from=builder /bin/rtbit /bin/rtbit
COPY root/ /

RUN mkdir -p /home/rtbit/db /home/rtbit/cache /home/rtbit/downloads /home/rtbit/completed

WORKDIR /home/rtbit

ENV HOME=/home/rtbit
ENV XDG_DATA_HOME=/home/rtbit/db
ENV XDG_CACHE_HOME=/home/rtbit/cache
ENV RTBIT_HTTP_API_LISTEN_ADDR=0.0.0.0:3030
ENV RTBIT_LISTEN_PORT=4240
ENV RTBIT_DOWNLOAD_FOLDER=/home/rtbit/downloads

VOLUME /home/rtbit/db
VOLUME /home/rtbit/cache
VOLUME /home/rtbit/downloads
VOLUME /home/rtbit/completed

EXPOSE 3030
EXPOSE 4240

# Liveness probe against the unauthenticated /healthz route. The port is taken
# from RTBIT_HTTP_API_LISTEN_ADDR so an overridden listen address still probes
# the right socket. Needs the /healthz route registered in the HTTP API router.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD wget -q -O /dev/null "http://127.0.0.1:${RTBIT_HTTP_API_LISTEN_ADDR##*:}/healthz"
