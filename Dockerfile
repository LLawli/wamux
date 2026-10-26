# syntax=docker/dockerfile:1
#
# Default image: a static musl binary on `scratch`. There is no shell, no libc
# and no package manager in the final image, so `docker exec ... sh` does not
# work; Dockerfile.glibc is the Debian variant for anyone who needs one. See
# docs/DEPLOYMENT.md for how the Unix socket leaves the container - it is the
# whole interface, so it needs more than a `ports:` line.

# Base images by digest and the build --locked (#78): the image we ship is built
# from the inputs CI tested, not from whatever the tags point at today.
FROM rust:trixie@sha256:931c2c0531a8b143e044f0f3914dfe802f15bb3d1955f0b657e9359327ac2ffc AS build
WORKDIR /src

# musl-tools gives the musl C compiler the bundled C code (SQLite, mimalloc) is
# built with.
RUN apt-get update \
 && apt-get install -y --no-install-recommends musl-tools \
 && rm -rf /var/lib/apt/lists/*

# The pinned nightly (the one upstream whatsapp-rust builds on, #30) is
# installed from rust-toolchain.toml alone, so this layer only rebuilds when
# the pin changes - not on every source edit.
COPY rust-toolchain.toml ./
RUN rustup show && rustup target add x86_64-unknown-linux-musl

# The workspace resolver needs every member's manifest on disk, so the whole
# crates/ tree is copied even though only wamux (and wamux-proto) get built.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# -p wamux --bin wamux on purpose: the repo carries a dozen development binaries
# (pairing helpers, e2e drivers, the bench client) that have no business in a
# production image. protoc is vendored by wamux-proto's build.rs, so nothing to install.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p wamux --bin wamux \
      --target x86_64-unknown-linux-musl && \
    strip target/x86_64-unknown-linux-musl/release/wamux && \
    cp target/x86_64-unknown-linux-musl/release/wamux /wamux

# The binary as a stage of its own, so the release can replace it: the release
# workflow builds and checks the musl binary once, for the tarball, and passes
# that same file here with `--build-context binary=<dir>`. The compile stage
# above then never runs and the image carries the bytes that were tested.
FROM scratch AS binary
COPY --from=build /wamux /wamux

# The empty runtime directories, made where a `RUN` exists because the final
# stage is `scratch`. A stage of its own, not `build`: the release replaces only
# `binary`, so anything else the final stage copies from must not compile, or
# BuildKit would run the cargo build in the image job anyway.
FROM debian:trixie-slim@sha256:62bd423aeee912639ca8ee681c2b762cb3e25137e0a92b20be455898a9f034ad AS dirs
RUN mkdir -p /stage/run/wamux /stage/var/lib/wamux /stage/var/cache/wamux

# `scratch` has to stay the LAST stage: a plain `docker build` ships the last.
FROM scratch

# Must match the UID that will consume the socket on the host. The socket is
# 0660, so a mismatch here is the difference between a working setup and a
# permission denied that looks like the daemon is down. Override at build time:
#   docker compose build --build-arg WAMUX_UID=$(id -u) --build-arg WAMUX_GID=$(id -g)
ARG WAMUX_UID=10001
ARG WAMUX_GID=10001

COPY --from=binary /wamux /usr/local/bin/wamux

# Shipping a binary means shipping the notices of every crate linked into it -
# the MIT and Apache-2.0 terms of the dependencies require it.
COPY THIRD-PARTY-LICENSES.md LICENSE-MIT LICENSE-APACHE /usr/share/doc/wamux/

# Owned by the runtime user: scratch has no /run or /var, so the daemon (not
# root) could not create the socket directory, the store directory or the
# default media spool (media_spool_dir = /var/cache/wamux) itself.
COPY --from=dirs --chown=${WAMUX_UID}:${WAMUX_GID} /stage/run/wamux /run/wamux
COPY --from=dirs --chown=${WAMUX_UID}:${WAMUX_GID} /stage/var/lib/wamux /var/lib/wamux
COPY --from=dirs --chown=${WAMUX_UID}:${WAMUX_GID} /stage/var/cache/wamux /var/cache/wamux

# Numeric: scratch has no /etc/passwd to resolve a name against.
USER ${WAMUX_UID}:${WAMUX_GID}
ENV WAMUX_SOCKET_PATH=/run/wamux/wamux.sock
# No ca-certificates: TLS uses the roots compiled into the binary (webpki-roots).
# The daemon sets its own umask (077), so no shell wrapper is needed, and being
# the entrypoint it is PID 1 and receives SIGTERM.
ENTRYPOINT ["/usr/local/bin/wamux"]
