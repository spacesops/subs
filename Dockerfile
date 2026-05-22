# syntax=docker/dockerfile:1

# Rust toolchain on Alpine (musl) for release binaries.
FROM rust:1-alpine3.21 AS builder-base

ARG CARGO_BUILD_JOBS=1

RUN apk add --no-cache \
    build-base \
    musl-dev \
    git \
    openssl-dev \
    openssl-libs-static \
    pkgconf \
    clang \
    llvm-dev \
    lld \
    libatomic \
    ca-certificates

WORKDIR /app
ENV CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS}
ENV CARGO_NET_GIT_FETCH_WITH_CLI=true
ENV RUSTFLAGS="-C link-arg=-fuse-ld=lld"

# subs + registry share a target/ tree (small compared to subs-prover).
FROM builder-base AS builder-subs

ARG ENABLE_REGISTRY=true

COPY . .

RUN set -eux; \
    cargo build --release -p subs; \
    if [ "$ENABLE_REGISTRY" != "false" ]; then \
        cargo build --release -p registry-server; \
    fi; \
    mkdir -p /out; \
    cp target/release/subs /out/; \
    if [ "$ENABLE_REGISTRY" != "false" ]; then \
        cp target/release/registry-server /out/; \
    fi; \
    cargo clean

# subs-prover (RISC Zero) in a fresh stage so target/ does not stack on top of subs.
FROM builder-base AS builder-prover

ARG ENABLE_PROVER=true
ARG GPU_ACCELERATION=none
ARG TARGETARCH

COPY . .

RUN set -eux; \
    if [ "$ENABLE_PROVER" = "false" ]; then \
        mkdir -p /out; \
        exit 0; \
    fi; \
    if [ "$TARGETARCH" = "arm64" ]; then \
        export CFLAGS="-mno-outline-atomics"; \
        export CXXFLAGS="-mno-outline-atomics"; \
        export CMAKE_C_FLAGS="-mno-outline-atomics"; \
        export CMAKE_CXX_FLAGS="-mno-outline-atomics"; \
        export RUSTFLAGS="-C link-arg=-fuse-ld=lld -C target-feature=-outline-atomics"; \
    else \
        export RUSTFLAGS="-C link-arg=-fuse-ld=lld"; \
    fi; \
    case "$GPU_ACCELERATION" in \
        none) cargo build --release -p subs-prover ;; \
        metal) cargo build --release -p subs-prover --features metal ;; \
        cuda) cargo build --release -p subs-prover --features cuda ;; \
        *) echo "Invalid GPU_ACCELERATION=$GPU_ACCELERATION (expected none, metal, or cuda)" >&2; exit 1 ;; \
    esac; \
    mkdir -p /out; \
    cp target/release/subs-prover /out/; \
    cargo clean

FROM alpine:3.21

ARG ENABLE_PROVER=true
ARG ENABLE_REGISTRY=true
ARG GPU_ACCELERATION=none

RUN apk add --no-cache ca-certificates libgcc tini \
    && addgroup -S subs \
    && adduser -S subs -G subs

COPY --from=builder-subs /out/subs /usr/local/bin/subs

RUN --mount=type=bind,from=builder-subs,source=/out,target=/subs-out \
    --mount=type=bind,from=builder-prover,source=/out,target=/prover-out \
    set -eux; \
    if [ "$ENABLE_REGISTRY" != "false" ] && [ -f /subs-out/registry-server ]; then \
        cp /subs-out/registry-server /usr/local/bin/registry-server; \
    fi; \
    if [ "$ENABLE_PROVER" != "false" ] && [ -f /prover-out/subs-prover ]; then \
        cp /prover-out/subs-prover /usr/local/bin/subs-prover; \
    fi; \
    : > /etc/subs-image.env; \
    if [ "$ENABLE_PROVER" != "false" ]; then \
        echo "SUBS_START_PROVER=1" >> /etc/subs-image.env; \
        echo "SUBS_PROVER_ENDPOINT=http://127.0.0.1:8888" >> /etc/subs-image.env; \
        echo "SUBS_PROVER_SERVER=1" >> /etc/subs-image.env; \
        echo "SUBS_PROVER_GPU_ACCELERATION=${GPU_ACCELERATION}" >> /etc/subs-image.env; \
    else \
        echo "SUBS_START_PROVER=0" >> /etc/subs-image.env; \
    fi; \
    if [ "$ENABLE_REGISTRY" != "false" ]; then \
        echo "SUBS_START_REGISTRY=1" >> /etc/subs-image.env; \
        echo "SUBS_REGISTRY_ENDPOINT=http://127.0.0.1:8080" >> /etc/subs-image.env; \
    else \
        echo "SUBS_START_REGISTRY=0" >> /etc/subs-image.env; \
    fi

COPY docker/entrypoint.sh /entrypoint.sh

RUN chmod +x /entrypoint.sh \
    && mkdir -p /data \
    && chown -R subs:subs /data

WORKDIR /data
USER subs

ENV SUBS_DATA_DIR=/data
ENV SUBS_PORT=7777
ENV SUBS_PROVER_PORT=8888
ENV REGISTRY_SERVER_PORT=8080

EXPOSE 7777 8888 8080

ENTRYPOINT ["/sbin/tini", "--", "/entrypoint.sh"]
CMD ["subs"]
