# wamux

[![CI](https://github.com/LLawli/wamux/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/LLawli/wamux/actions/workflows/ci.yml)

**WhatsApp multiplexer core daemon: many accounts, one Unix socket, gRPC.**

`wamux` is the core daemon of an unofficial WhatsApp system (in the style of
Evolution API), built directly on the Rust crate
[`whatsapp-rust`](https://github.com/oxidezap/whatsapp-rust) (no Baileys, no JS
runtime). It multiplexes **many WhatsApp accounts in one process** and exposes a
gRPC API over a **Unix domain socket only** (no TCP, no TLS). It is fully async
on tokio.

> ⚠️ **Unofficial.** This project is not affiliated with, endorsed by, or
> connected to WhatsApp or Meta. It talks to WhatsApp through the unofficial
> `whatsapp-rust` library. Using it may violate WhatsApp's Terms of Service and
> can get an account banned. Use it only with accounts and in ways you are
> authorized to, and at your own risk.

## What it is (and is not)

`wamux` is a **pure relay**. It exposes raw WhatsApp *mechanisms* and nothing
else, and it has **no auth of its own**: no tokens, no users. By default anyone
who can open the socket sees and controls everything; an optional allowlist by
peer uid/gid (`allowed_uids`, `allowed_gids`) narrows who is served. A socket
client picks which account(s)' events it wants
(one / all / none = send-only).

Every stateful or opinionated behavior (auth, permissions, per-user filtering,
retries, timeouts, fallbacks, webhooks, reconnection policy, an HTTP API) lives
in a **separate edge project, out of scope here**. The core hands the edge the
primitives it needs (the message id in `SendResult.key`, the send's
`recipient_fanout`, `ReceiptEvent` and `ConnectionStateChanged` events) and lets
the edge compose the policy.

**Relay-pure persistence:** the core persists *only* `whatsapp-rust`'s
protocol state (Signal sessions and keys, the device, app-state sync keys,
contact and message secrets, and the library's short-lived copy of recently
sent messages, kept for retries), one row per
account and a few store-wide markers, in Postgres or SQLite (Turso
experimentally). It keeps no message history.

### Security model

- **Transport is the filesystem's job.** The socket is local, so security is
  enforced by file permissions: the socket is given its mode (`socket_mode`,
  default `0660`) and, optionally, its group (`socket_group`, default the
  daemon's own) before it appears at its path; it is not protected on the wire.
  There is no token, no TLS, no TCP listener in this binary.
- **No in-process auth.** Authentication and authorization are the edge's
  responsibility. To gate access locally, set `allowed_uids` / `allowed_gids`:
  a peer is served only when its `SO_PEERCRED` uid is in `allowed_uids` or its
  effective gid is in `allowed_gids`; anyone else (including a peer with no
  credentials) gets `PermissionDenied`. Both empty, the default, serves everyone
  who can open the socket. See
  [Allowlist by peer credentials](docs/DEPLOYMENT.md#allowlist-by-peer-credentials-77).
- **The store is a secret.** It holds every paired account's keys, in plaintext
  unless `store_key_file` turns on encryption at rest: see
  [The store is a secret](docs/DEPLOYMENT.md#the-store-is-a-secret) and
  [Encryption at rest](docs/DEPLOYMENT.md#encryption-at-rest).
- Network exposure, if ever needed, is a separate front (a reverse proxy or a
  second binary), never a flag flipped here.

## Requirements

- **Rust nightly**: pinned in `rust-toolchain.toml`, the nightly upstream
  `whatsapp-rust` builds and tests on (the lib is pinned to a git revision of
  its main branch, see #30). `rustup` will pick up the pinned toolchain
  automatically.
- **Postgres or SQLite** for the Signal/device store. The `database_url`
  scheme picks the engine: `postgres://` (or `postgresql://`) for the
  multi-account deployment, `sqlite://` for a single file with no server
  process. `turso://` is an experimental third engine, available only in a
  binary built with `--features turso` (see
  [Turso (experimental)](docs/DEPLOYMENT.md#turso-experimental)).
- `protoc` is **not** required on the host: it is vendored and run by
  `crates/wamux-proto/build.rs`, which regenerates the Rust gRPC code from
  `crates/wamux-proto/proto/` whenever a file there changes.

## Install

### Docker (fastest)

```sh
mkdir -p run    # yours: if Docker creates it, root owns it and the bind fails
WAMUX_UID=$(id -u) WAMUX_GID=$(id -g) docker compose up -d --build
# socket at ./run/wamux.sock
```

Brings up the daemon and Postgres. `WAMUX_UID`/`WAMUX_GID` are build arguments
(compose defaults both to 1000) that set the image's `wamux` user, so change
them together with `--build`. The socket is `0660`,
so whoever opens it must share that uid or gid; otherwise every connection
fails with permission denied, which looks like the daemon being down.

### Native, no database server

```sh
cargo build --release -p wamux --bin wamux
cp target/release/wamux ~/.local/bin/
cp contrib/wamux.service ~/.config/systemd/user/
systemctl --user daemon-reload && systemctl --user enable --now wamux
# socket at ~/.local/state/wamux/wamux.sock
```

The shipped unit uses the SQLite engine, so there is nothing else to install.
Point `WAMUX_DATABASE_URL` at a `postgres://` DSN when you outgrow it.

### macOS (PKG)

Each release carries `wamux-<version>-macos-universal.pkg`, one installer for
Apple silicon and Intel. It is **not signed or notarized by Apple**, so a PKG
downloaded in a browser is blocked by Gatekeeper: open System Settings, Privacy
& Security, and choose **Open Anyway** next to the blocked installer. From a
terminal there is no quarantine flag to get past:

```sh
curl -LO https://github.com/LLawli/wamux/releases/download/v<version>/wamux-<version>-macos-universal.pkg
sudo installer -pkg wamux-<version>-macos-universal.pkg -target /
```

The PKG installs `/usr/local/bin/wamux` and starts nothing. The docs, the
licenses, `wamux.toml.example` and an example LaunchAgent land in
`/usr/local/share/wamux/`; the
[macOS section of the deployment guide](docs/DEPLOYMENT.md#macos-launchagent)
shows how to run it at login. The socket defaults to `/var/run/wamux.sock` on
macOS (`/run` cannot exist there), which only root can create: as a regular
user, set `WAMUX_SOCKET_PATH`, as the LaunchAgent does.

The CI installs this PKG on a macOS runner on every pull request and starts the
installed daemon. It has not been run on a real Mac by the maintainer: treat the
Gatekeeper steps above, and the LaunchAgent, as untested by hand.

[docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) covers the setups in full
([native](docs/DEPLOYMENT.md#native-systemd),
[Docker](docs/DEPLOYMENT.md#docker-consumer-on-the-host)), reaching the socket
[from another container](docs/DEPLOYMENT.md#docker-consumer-in-another-container),
and the [failure modes](docs/DEPLOYMENT.md#troubleshooting).

## Quick start (from source)

```sh
# 1. Postgres (Docker example; port 5433 is what the tests and CI expect)
docker run -d --name wamux-pg \
  -e POSTGRES_USER=wamux -e POSTGRES_PASSWORD=wamux -e POSTGRES_DB=wamux \
  -p 5433:5432 postgres:16

# 2. Configure. The defaults (/run/wamux.sock, /var/cache/wamux) assume a
#    system install; as a regular user, point the socket and the media spool
#    somewhere you can write. Every key can be set in wamux.toml (copied from
#    wamux.toml.example) or overridden by an env var, WAMUX_<UPPERCASE_KEY>.
mkdir -p "$HOME/.local/state/wamux"
export WAMUX_DATABASE_URL=postgres://wamux:wamux@localhost:5433/wamux
export WAMUX_SOCKET_PATH="$HOME/.local/state/wamux/wamux.sock"
export WAMUX_MEDIA_SPOOL_DIR="$HOME/.cache/wamux"
#    For a serverless setup, skip step 1 and use instead:
#      export WAMUX_DATABASE_URL="sqlite://$HOME/.local/state/wamux/wamux.db"
#    (sqlite creates the file, not its directory: hence the mkdir above.)

# 3. Build (also regenerates protobuf) and run
cargo run
```

The server binds the Unix socket from `socket_path` with mode `socket_mode`
(default `0660`), then serves the gRPC API. It creates the socket's parent
directory and the media spool if they are missing, and refuses to start if
either cannot be written. It shuts down gracefully on SIGTERM/SIGINT and unlinks
the socket on the way out.

Poke the socket by hand (needs `grpcurl`). gRPC reflection, which lets it list
and describe the services, is development tooling: it is on by default, and
`enable_reflection = false` turns it off.

```sh
grpcurl -unix -plaintext "$HOME/.local/state/wamux/wamux.sock" list
```

## Configuration

Copy `wamux.toml.example` to `wamux.toml` (or point `WAMUX_CONFIG` at a path).
Keys include `socket_path`, `socket_mode`, `socket_group`, `allowed_uids`,
`allowed_gids`, `database_url`, `db_max_connections`, `store_key_file`,
`media_spool_dir`, `event_ring_capacity` and `broadcast_capacity`. Each
key can be overridden by a `WAMUX_<UPPERCASE_KEY>` environment variable. See the
example file for the full annotated list.

`socket_mode` is a number, not an octal string: in the environment write it in
decimal (`WAMUX_SOCKET_MODE=432` is `0660`); in TOML `0o660` also works.
`WAMUX_SOCKET_MODE=0660` is read as decimal 660 and starts with the wrong mode,
without an error.

## Development

```sh
cargo build                                    # build + regenerate proto
cargo fmt                                      # format (law)
cargo clippy --workspace --all-targets -- -D warnings  # lint, zero tolerance
# tests, Postgres engine (the daemon only: it is the default member)
DATABASE_URL=postgres://wamux:wamux@localhost:5433/wamux cargo test
# same suite, SQLite engine; a test named for its engine builds that engine
# itself, so it already ran in the pass above and is skipped here
WAMUX_TEST_ENGINE=sqlite cargo test -- --skip postgres_ --skip sqlite_ --skip both_engines_
scripts/ci.sh                    # every gate (--full adds the scale tests)
scripts/ci.sh --no-postgres      # a database-free subset, for fast feedback
```

`scripts/ci.sh` runs every gate but the dependency audit in one command,
including the tests of `wamux-types` and `wamux-tools`; hosted CI adds the
audit. Run it before declaring work done. Hosted CI
(`.github/workflows/ci.yml`, GitHub Actions) runs on every pull request and
every push to `main`, in three jobs: `scripts/ci.sh` on Postgres,
`scripts/ci.sh --no-postgres` (fmt, clippy, the static checks, and every test
that needs no Postgres: unit tests, the SQLite and Turso storage cases, a few
service suites on SQLite and Turso, socket binding, `wamux-types` and the
`wamux-tools` suites), and `cargo audit`. It runs
`ci.sh` without `--full`, so the scale tests stay a local step: run
`scripts/ci.sh --full` before merging a change that could affect them.

The `crates/wamux-proto/proto/` files are the **source of truth** for the API
contract: change the proto, rebuild, then fix the Rust, never the other way
around. Generated code is never edited by hand.

### Layout

```
crates/wamux-proto/     # generated gRPC types
  proto/                # .proto contracts (source of truth)
  build.rs              # tonic-prost-build: regenerates Rust from proto/
crates/wamux-types/     # named domain types and the error type, shared by every layer
crates/wamux-tools/     # development binaries (src/bin/), not shipped
crates/wamux/           # the daemon
  migrations/, migrations_sqlite/   # store schema: Postgres; SQLite and Turso
  tests/
  src/
    main.rs       # load config, open the store, bind socket, run server, handle signals
    cli.rs        # one-shot commands: `wamux store decrypt`, `wamux store rotate-key`
    server.rs     # assemble the gRPC Server: services + tower layers
    observe.rs    # metrics and the per-request log layer
    services/     # one gRPC service impl per file (thin handlers)
    domain/       # business logic, transport-agnostic, unit-tested
    state/        # account registry, per-account handles, replay ring, event bridge
    transport/    # UDS listener, socket lifecycle, peer-cred checks
    storage/      # StorageEngine trait; sql/ (Postgres + SQLite via sqlx),
                  #   turso/ (feature `turso`), statements/ (shared SQL)
    spool/        # media transfers staged on disk
    stress/       # mock WhatsApp server (`stress` feature only)
    config.rs     # Config struct + load-at-startup
    error.rs      # builds WamuxError from whatsapp-rust errors (the type and
                  #   its Status mapping live in wamux-types)
```

The binaries under `crates/wamux-tools/src/bin/` (pairing, end-to-end checks,
live probes, benchmarks) are development and diagnostic tools, not part of the
daemon. Each one's purpose, environment, what a green run proves and what it
writes to WhatsApp is in
[`crates/wamux-tools/README.md`](crates/wamux-tools/README.md).

## Documentation

- [CONTRIBUTING.md](CONTRIBUTING.md): how to get the gates green, and the one
  rule that overrides the others (the core is a pure relay).
- [CHANGELOG.md](CHANGELOG.md): what changed per release.
- [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md): reaching the socket (native,
  Docker, container-to-container), who may open it (mode, group, peer
  allowlist), the media spool, the storage engines, encryption at rest and
  verifying a release.
- `docs/BREAKING-CHANGES-*.md`: what each breaking change asks of an edge or an
  operator, and how to migrate. With the comments in the `.proto` files, they
  are where the core's promises to an edge are written down.
- [docs/PRD.md](docs/PRD.md): product requirements and the relay-pure rationale.
- [docs/SPEC.md](docs/SPEC.md): the technical spec.
- [docs/store-trait-defaults.md](docs/store-trait-defaults.md): which default
  bodies of the `wacore` store traits the engines keep, and why.
- `docs/crate-notes/`: a snapshot of the `whatsapp-rust` 0.6.0 API taken during
  the port; historical, so check the pinned revision's source before relying
  on it.
- `CLAUDE.md` / `AGENTS.md`: contributor conventions (also consumed by coding
  agents).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

`whatsapp-rust` and its companion crates are MIT, which imposes no constraint
on this choice. [THIRD-PARTY-LICENSES.md](THIRD-PARTY-LICENSES.md) carries the
notices of every crate linked into the binary, as their terms require; it is
regenerated with `scripts/gen-third-party.py` and ships inside the Docker
image.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.

> Note: this license covers wamux's own source. Third-party dependencies
> (including `whatsapp-rust`) carry their own licenses; review them before
> redistribution.
