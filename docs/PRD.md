# wamux — PRD

## Context

`wamux` is the **core daemon** of an unofficial WhatsApp system in the style of
Evolution API, but: it exposes **only a Unix domain socket** (no HTTP), is built on
the Rust crate **`whatsapp-rust`** (no Baileys, no JS runtime → much lighter), and is
a **pure, functional core**. It multiplexes **many WhatsApp accounts** in one process
and exposes *everything* to whoever can open the socket. Auth, user distinction,
permissions, and any HTTP API live in a **separate edge project (out of scope here)**.

A socket client chooses which account(s)' events to receive: one, all, or none
(none = send-only). The core is **relay-pure**: it persists only the Signal/session
state required for encryption, never business message history.

## Personas / consumers
- **Edge service** (primary): a local process that owns auth and per-user routing,
  consuming the socket via gRPC.
- **Operators/devs**: `grpcurl` over the socket (reflection on in dev).

## v1 capabilities
1. **Accounts**: create, pair (QR + 8-digit code), status, connect, disconnect,
   logout, delete, list. Identity = canonical **UUID** + optional unique
   **`external_ref`**.
2. **Events (streaming)**: subscribe per account / all / none; receive messages,
   receipts, presence, group/contact updates, connection-state, pairing — plus a
   `raw` catch-all so no event type is ever silently lost. Live delivery + a short
   in-memory ring for quick-reconnect replay (no durable log).
3. **Send**: text (+reply +mentions), media (image/video/audio/doc/sticker),
   reaction, edit, delete (revoke / for-me), presence (typing/recording), mark-read.
4. **Media**: send inline (client-streamed bytes) or by URL; receive lazily (event
   carries a descriptor; client calls `DownloadMedia` later).
5. **Groups**: create, add/remove, promote/demote, subject/description, metadata,
   invite link (get/revoke), join via invite.
6. **Contacts/profile**: check-number-on-WhatsApp, profile picture (get/set/remove),
   push name/about, business profile, subscribe to a contact's presence.

## Decisions (locked)
| Topic | Decision |
|---|---|
| Wire protocol | gRPC (tonic) over UDS + reflection (dev) |
| Message history | Relay-pure — core stores none |
| Storage | Postgres via `sqlx`, one pool, `device_id`-scoped |
| Account id | Hybrid: core UUID + optional `external_ref` |
| Pairing | Both QR and pair-code |
| Event subscription | Per account (all types; edge filters); live + short ring |
| Connection policy | Per account: `on_demand` vs `always_on` (reconnect on boot) |
| Media send | Inline bytes + URL |
| Media receive | Lazy (descriptor → `DownloadMedia`) |
| Socket trust | No core auth; FS perms 0660 + owning group; SO_PEERCRED for logs |
| Config | TOML + `WAMUX_` env overrides |
| Observability | tracing logs + `AdminService.GetMetrics` (Prometheus text) |
| Toolchain | Pinned nightly + edition 2024 (`simd`) |
| Name | crate/binary `wamux` |
| Deploy | systemd and container |

## Out of scope
HTTP/auth/permissions edge; message-history persistence; voice/video calls;
communities/channels and interactive (button/list) messages.

## Status
Implemented and verified: Postgres backend (4 traits, `device_id` isolation test),
account lifecycle + transport over a real socket (gRPC integration test), event
streaming, sending + media, multi-account registry with always-on reconnect, groups/
contacts/presence. Real pairing/messaging requires live WhatsApp credentials and is
exercised manually. See `docs/SPEC.md` for the technical design.
