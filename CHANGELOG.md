# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

While the version is `0.x`, the gRPC contract in `proto/` may change in a minor
release. Breaking wire changes are called out under **Changed** with the
migration note, since the edge that consumes this socket has to follow them.

## [Unreleased]

### Added

- **The LID↔phone mapping is reachable over the contract** (issue #1). A chat
  whose only identity is a `@lid` was unnameable through the socket: the
  library learns the phone side and kept it to itself. Three reads, all pure
  relay of what the client already knows:
  - `InboundMessage.sender_alt` / `recipient_alt` carry the other-namespace jids
    the stanza itself supplied, which the core used to drop. No round trip.
  - `ContactService.ResolveLidPn` resolves a batch of jids in either direction
    against the live client (in-memory cache first, durable store on miss), so
    it also answers for mappings learned during an offline history replay.
  - `ContactService.ListLidMappings` dumps every pair persisted for the account,
    for a consumer reconciling its own store in one pass. Storage-side, so it
    answers for a disconnected account.

  The core still never rewrites a JID onto the other namespace and never
  invents a pair: an unknown jid answers `found=false`.

### Changed

- **`GetGroupMetadata` now hands back each participant whole** (issue #1). The
  JSON flattened every participant to its jid string, which in a LID-addressed
  group meant discarding `phone_number` — the only phone jid the roster carries,
  i.e. the answer to "who is this `@lid`" for every member — along with who is
  admin. `participants` entries are now objects (`jid`, `phone_number`, `type`)
  and the payload gained `addressing_mode`, so the edge can tell whether the
  roster's jids are LIDs before it tries to name anyone. Wire shape change for a
  consumer that read `participants[i]` as a string; `subject`, `id` and
  `description` are untouched.

- **Breaking: `ContactService.GetPushName` now takes an `AccountRef`, not a
  `JidRequest`** (issue #1). It always answered the *account's own* push name —
  the getter that pairs with `SetPushName` — while ignoring the `jid` field it
  asked for, and the shape read as "what is this contact called". WhatsApp
  gives a companion device no per-contact push-name store, so the field could
  not be honoured; it is gone instead. Migration: send the same `AccountRef`
  you were putting in `JidRequest.account` and drop the jid. A caller that was
  using this to name contacts was stamping its own push name on them; the push
  name of a *peer* arrives on the events that carry it
  (`InboundMessage.push_name`, `PushNameUpdate`).

## [0.1.0] - 2026-08-28

First tagged release. The daemon multiplexes many WhatsApp accounts in one
process and serves a gRPC API over a Unix domain socket.

### Added

- **Multi-account core.** One process, N accounts, each with its own Signal
  session state, connection supervisor and event stream. `AccountService`
  covers the lifecycle: create, list, pair (QR and phone code), connect,
  disconnect, logout, delete.
- **Event fan-out.** `EventService.SubscribeEvents` streams typed events per
  account or across all accounts, with the all-accounts selector staying
  dynamic (accounts paired later join an open stream). A per-account replay
  ring lets a reconnecting subscriber pick up recent events; a subscriber that
  falls too far behind gets an explicit `subscription_gap` marker rather than
  silent loss.
- **Messaging.** Text, media (image, video, audio, document, sticker, PTT voice
  notes, PTV), reactions, edits, deletions, link previews, ephemeral messages,
  contacts, polls, and status posting. Chat actions: read/unread, star,
  archive, pin, mute, delete, presence.
- **Groups.** Creation and membership, permissions and settings, ephemeral
  timers, invite previews, group photo, and membership approval.
- **Contacts.** WhatsApp presence check, profile picture get/set/remove, push
  name get/set.
- **Storage behind a `StorageEngine` trait**, with two implementations chosen
  by the `database_url` scheme: `postgres://` for many accounts, `sqlite://`
  for a single host with no database server. Both persist byte-identical
  blobs, so a store is portable between them, and a test asserts exactly that.
- **Relay-pure persistence.** Only `whatsapp-rust`'s Signal/session/device
  state is stored. No message history, ever.
- **Unix socket transport** with configurable mode (default `0660`) and owning
  group, graceful shutdown on SIGTERM/SIGINT, and socket unlink on exit.
- **Observability.** Structured `tracing` logs (text or JSON), one span per
  request with method, peer uid, latency and status, plus `AdminService` with
  a Prometheus render and a health check that reports serving and readiness
  separately.
- **Install surface.** Multi-stage Docker image, `docker-compose.yml`, and a
  hardened systemd unit in `contrib/`. See [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).
- **CI on GitHub Actions**, running `scripts/ci.sh` so there is one definition
  of "green", plus a database-free subset for fast feedback and `cargo audit`.
- **Third-party attribution** (`THIRD-PARTY-LICENSES.md`), generated from the
  resolved dependency graph and shipped inside the Docker image.

### Security

- **The socket is the security boundary.** The core has no authentication of
  its own: anyone who can open the socket controls every account. Filesystem
  permissions are the whole mechanism, by design. Authentication, permissions
  and per-user filtering belong to the edge.
- Patched advisories in the dependency tree: `h2` (RUSTSEC-2026-0258),
  `crossbeam-epoch` (RUSTSEC-2026-0204), `anyhow` (RUSTSEC-2026-0190),
  `event-listener` (RUSTSEC-2026-0221), and two yanked crates. The two
  remaining advisories are documented decisions in `.cargo/audit.toml`.
- Maintainer's personal phone numbers scrubbed from development tooling.

### Notes on scope

Deliberately **not** in the core, and belonging to the edge: retries,
timeouts, fallbacks, recipient rewriting, per-user filtering, auth, webhooks,
reconnection policy, and any HTTP API. The core exposes the primitives (a
`SendResult.message_id`, `Receipt` events, connection-state events) and lets
the edge compose the policy. See the "Core purity" section in `CLAUDE.md`.

[Unreleased]: https://github.com/LLawli/wamux/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/LLawli/wamux/releases/tag/v0.1.0
