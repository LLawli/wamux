# wamux: PRD

## Context

`wamux` is the **core daemon** of an unofficial WhatsApp system in the style of
Evolution API, but: it exposes **only a Unix domain socket** (no HTTP), is built on
the Rust crate **`whatsapp-rust`** (no Baileys, no JS runtime → much lighter), and is
a **pure, functional core**. It multiplexes **many WhatsApp accounts** in one process
and exposes *everything* to whoever can open the socket (narrowed, if configured, by
an optional `SO_PEERCRED` uid/gid allowlist). Auth, user distinction, permissions, and
any HTTP API live in a **separate edge project (out of scope here)**.

A socket client chooses which account(s)' events to receive: one, all, or none
(none = send-only). The core is **relay-pure**: it persists only what an account
needs to run (wamux's account index and store-format bookkeeping, plus
whatsapp-rust's store: Signal sessions and keys, device and app-state sync state,
LID/PN mappings, privacy (tc) tokens, message secrets, and the library's short-lived
copies of recently sent payloads for retry receipts), never business message history.

See [`docs/SPEC.md`](SPEC.md) for the technical design.

## Personas / consumers
- **Edge service** (primary): a local process that owns auth and per-user routing,
  consuming the socket via gRPC.
- **Operators/devs**: `grpcurl` over the socket. gRPC reflection exists for this dev
  tooling; it is on by default, and a production deployment can set
  `enable_reflection = false`.

## Capabilities
1. **Accounts**: create, pair (QR, or an 8-character pair code for a phone number),
   status, connect, disconnect, logout, delete, list. Identity = canonical **UUID** +
   optional unique **`external_ref`**.
2. **Events (streaming)**: subscribe to one account or all (none = do not
   subscribe); receive the library's account events as typed messages (messages,
   receipts, presence, group/contact updates, connection state, pairing, calls,
   history sync, app-state changes and more), with a `raw` catch-all for any variant
   that has no typed mapping, so a new library event is never silently lost; a few
   internal ones are dropped on purpose (listed in
   `crates/wamux/src/domain/event_mapping.rs`). The stream also carries an echo of
   every message the core itself sent (`key.from_me` true), to every subscriber
   including the caller, so the edge dedupes on `key.id`. Besides that echo, the
   core makes up only two `raw` markers: `gap` replaces the events a slow
   subscriber missed (resync with `GetAccountStatus`), and `subscription_gap` means
   an all-accounts stream missed some new accounts (re-subscribe or reconcile with
   `ListAccounts`). Live delivery, lossy past `broadcast_capacity`, + a short
   in-memory ring for quick-reconnect replay (no durable log).
3. **Send**: text (+reply +mentions), media (image/video/audio/doc/sticker),
   reaction, edit, delete (revoke / for-me), presence (online/offline, and
   typing/recording/paused in a chat), mark-read.
4. **Rich sends**: contacts, polls (create, vote, aggregate the votes), and replies
   to interactive offers (buttons, list, template, native flow).
5. **Chat actions**: mark a chat read or unread, star a message, archive, pin, mute
   and delete a chat.
6. **Status posts**: post a text or media status, revoke one.
7. **History on demand**: ask the phone for older messages of a chat; they arrive
   as history-sync events.
8. **Media**: send as client-streamed bytes only (the core never fetches a URL for a
   send; the edge downloads remote media itself); receive lazily (event carries a
   descriptor; client calls `DownloadMedia` later).
9. **Groups**: create, list the ones the account is in, leave, add/remove,
   promote/demote, subject/description/photo, settings (announce, locked,
   ephemeral, membership approval, who may add members, share the invite link
   and share the history), metadata, invite link (get/revoke/preview), join via
   invite, membership requests (list/approve/reject).
10. **Contacts/profile**: check-number-on-WhatsApp, profile picture (get anyone's,
    set/remove the account's own), the account's own push name (get/set), a
    contact's about text and the account's own (set), business profile, subscribe
    to a contact's presence, LID/PN resolution, block/unblock and the blocklist.
11. **Channels (newsletters)**: list followed channels, read metadata and messages
    (with the server-side reaction and poll tallies), vote in channel polls, read
    back the account's own reactions and votes, subscribe to live updates, follow,
    unfollow and mute a channel, create one.
12. **Admin**: metrics and a liveness/readiness check over the socket.

The complete, current contract is
[`crates/wamux-proto/proto/`](../crates/wamux-proto/proto/).

## Decisions (locked)
| Topic | Decision |
|---|---|
| Wire protocol | gRPC (tonic) over UDS only; reflection behind `enable_reflection` (default on) |
| Message history | Relay-pure: no business history; only the library's short-lived copies of recently sent payloads, for retry receipts |
| Storage | One engine per deployment, chosen by the `database_url` scheme: Postgres or SQLite (sqlx), Turso experimental behind the `turso` cargo feature; every account's rows `device_id`-scoped; secret columns optionally encrypted at rest (`store_key_file`). Schema: [`crates/wamux/migrations/`](../crates/wamux/migrations/) and [`crates/wamux/migrations_sqlite/`](../crates/wamux/migrations_sqlite/) |
| Account id | Hybrid: core UUID + optional `external_ref` |
| Pairing | Both QR and pair-code |
| Event subscription | One account or all (dynamic); every event type, the edge filters; live plus a short in-memory replay ring |
| Connection | Edge-driven: the edge calls `ConnectAccount`/`DisconnectAccount` (also after a restart); the core keeps no per-account policy and adds no reconnect beyond the library's own; a soft `max_connected_accounts` budget guards file descriptors |
| Media send | Inline client-streamed bytes only; no URL fetch in the core |
| Media receive | Lazy (descriptor → `DownloadMedia`) |
| Socket trust | No core auth; FS perms (`socket_mode`, default 0660) + optional owning group (`socket_group`); SO_PEERCRED uid in request logs and an optional allowlist (`allowed_uids`, `allowed_gids`): a peer passes on either list, and the gid matched is the peer's effective one, not its supplementary groups |
| Config | TOML + `WAMUX_` env overrides |
| Observability | tracing logs; `AdminService.GetMetrics` (Prometheus text) and `AdminService.Check` (liveness/readiness) over the socket |
| Toolchain | Pinned nightly + edition 2024, aligned with upstream whatsapp-rust |
| Name | crate/binary `wamux` |
| Deploy | systemd and container |

## What the tests do not prove
The service-level suites run a real tonic client over a real Unix socket, and the
connection-level ones run against a mock WhatsApp server (the `stress` cargo feature,
never in the production daemon); storage and domain logic have their own tests
without a socket.
Real pairing and delivery need live WhatsApp credentials and are exercised manually.

## Out of scope
HTTP/auth/permissions edge; message-history persistence; placing or answering
voice/video calls (call signaling reaches the edge as events, and the edge can
reject one with `CallService.RejectCall`);
community management (community data reaches the edge only through group metadata
and events); administering a channel (editing it, its picture, its admins,
deleting it) and posting to one; sending interactive (button/list)
offers (answering one is in scope).
