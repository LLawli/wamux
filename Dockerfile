# syntax=docker/dockerfile:1
#
# Two stages: build with the full Rust toolchain, ship a slim runtime with just
# the daemon. See docs/DEPLOYMENT.md for how the Unix socket leaves the
# container - it is the whole interface, so it needs more than a `ports:` line.

FROM rust:bookworm AS build
WORKDIR /src

# The pinned nightly (the one upstream whatsapp-rust builds on, #30) is
# installed from rust-toolchain.toml alone, so this layer only rebuilds when
# the pin changes - not on every source edit.
COPY rust-toolchain.toml ./
RUN rustup show

# The workspace resolver needs every member's manifest on disk, so the whole
# crates/ tree is copied even though only wamux (and wamux-proto) get built.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# -p wamux --bin wamux on purpose: the repo carries a dozen development binaries
# (pairing helpers, e2e drivers, the bench client) that have no business in a
# production image. protoc is vendored by wamux-proto's build.rs, so nothing to install.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p wamux --bin wamux && \
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
# The store is a secret (#75): it holds every account's keys in plaintext. The
# umask makes anything the daemon or its engine creates owner-only, and `exec`
# keeps wamux as PID 1 so it still receives SIGTERM. Debian-slim ships sh.
ENTRYPOINT ["/bin/sh", "-c", "umask 077 && exec /usr/local/bin/wamux"]
