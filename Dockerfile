# syntax=docker/dockerfile:1
#
# Two stages: build with the full Rust toolchain, ship a slim runtime with just
# the daemon. See docs/DEPLOYMENT.md for how the Unix socket leaves the
# container - it is the whole interface, so it needs more than a `ports:` line.

FROM rust:bookworm AS build
WORKDIR /src

# The pinned nightly (whatsapp-rust needs core::simd and edition 2024) is
# installed from rust-toolchain.toml alone, so this layer only rebuilds when
# the pin changes - not on every source edit.
COPY rust-toolchain.toml ./
RUN rustup show

COPY Cargo.toml Cargo.lock build.rs ./
COPY proto ./proto
COPY src ./src
COPY migrations ./migrations
COPY migrations_sqlite ./migrations_sqlite

# --bin wamux on purpose: the repo carries a dozen development binaries
# (pairing helpers, e2e drivers, the bench client) that have no business in a
# production image. protoc is vendored by build.rs, so nothing to install.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --bin wamux && \
    cp target/release/wamux /usr/local/bin/wamux

FROM debian:bookworm-slim AS runtime

# Must match the UID that will consume the socket on the host. The socket is
# 0660, so a mismatch here is the difference between a working setup and a
# permission denied that looks like the daemon is down. Override at build time:
#   docker compose build --build-arg WAMUX_UID=$(id -u) --build-arg WAMUX_GID=$(id -g)
ARG WAMUX_UID=10001
ARG WAMUX_GID=10001

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

RUN groupadd --gid "${WAMUX_GID}" wamux \
 && useradd --uid "${WAMUX_UID}" --gid "${WAMUX_GID}" --no-create-home \
            --shell /usr/sbin/nologin wamux \
 && mkdir -p /run/wamux /var/lib/wamux \
 && chown wamux:wamux /run/wamux /var/lib/wamux

COPY --from=build /usr/local/bin/wamux /usr/local/bin/wamux

# Shipping a binary means shipping the notices of every crate linked into it -
# the MIT and Apache-2.0 terms of ~356 dependencies require it.
COPY THIRD-PARTY-LICENSES.md LICENSE-MIT LICENSE-APACHE /usr/share/doc/wamux/

USER wamux
ENV WAMUX_SOCKET_PATH=/run/wamux/wamux.sock
ENTRYPOINT ["/usr/local/bin/wamux"]
