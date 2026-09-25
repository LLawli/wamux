# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

While the version is `0.x`, the gRPC contract in `proto/` may change in a minor
release. Breaking wire changes are called out under **Changed** with the
migration note, since the edge that consumes this socket has to follow them.

## [Unreleased]

### Added

- **`NewsletterMessage` says when a row was edited** (issue #51). #44 relayed
  the row's `edit` token, so an edited row could be told apart but not dated.
  `GetNewsletterMessages` now reads the two times the server puts on `<meta>`:
  `original_timestamp` (field 9, from `original_msg_t`) and
  `last_edit_timestamp` (field 10, from `msg_edit_t`). The server counts the
  first in seconds and the second in milliseconds; both cross in milliseconds.
  Measured live: an edited row carries both, a revoked row only the original,
  an untouched row neither (0). An edited row's `message.timestamp` is the
  edit time, so `original_timestamp` is where its posting time survives.

- **`RevokeStatus`: a posted status can be taken down** (issue #41). A status
  revoke is encrypted to the status's own recipients, so the request carries
  them: `message_id` (the `PostStatus*` answer's `key.id`) and `recipients`,
  the same list the status was posted with. The core keeps no record of it.
  The answer's key is the revoke stanza's own, like `EditMessage`; the revoke
  echoes in `status@broadcast` as a delete whose `protocol_target` is the
  status.

- **The six library-built sends echo too** (issue #38, upstream #1406, the
  #15 queue). `SendPoll`, `SendPollVote`, `EditMessage`, `DeleteMessage` (for
  everyone) and `PostStatusText`/`PostStatusMedia` now publish the same
  from-me `InboundMessage` the other five sends do, so every consumer of the
  socket sees them, not only the caller. The library now hands back the message
  it built, which is what these lacked. An edit or revoke echoes as its
  protocol message (`is_edit`/`is_delete`, `protocol_target`, `key.id` the
  stanza's own id); a vote echoes encrypted, read from `raw_message` like any
  vote; a status echoes in `status@broadcast`. Delete-for-me sends nothing to
  the chat and still echoes nothing. No RPC response changed.

- **`OfflineSyncInterrupted`: a resume that was cut says so** (issue #38,
  upstream #1380, the #15 queue). When the connection ends mid-drain, the
  library now emits `total` (what the preview announced) and `delivered` (what
  was processed before the cut). It reached the socket as an untyped
  `RawEvent`; it is now `EventEnvelope.offline_sync_interrupted` (field 25),
  typed like its two siblings. It means "not caught up, the remainder comes
  back", never "lost": the drain acked nothing, so the server redelivers all of
  it behind a fresh `OfflineSyncPreview`. A consumer matching the `Raw` kind
  `OfflineSyncInterrupted` should switch to the typed event.

- **`PresenceUpdate.chat`: the conversation a chat state happened in** (issue
  #24). `Event::ChatPresence` carries a whole `MessageSource`, and only
  `sender` survived the mapping. Somebody typing in a group therefore arrived
  as identical bytes to the same person typing in the direct chat, so a client
  drawing a typing indicator had no choice but to draw it on the DM. Field 5
  now carries `source.chat` (the group jid in a group, the contact's jid in a
  direct chat) and is empty on real presence, which is not scoped to a
  conversation.

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

### Fixed

- **App-state writes work again after the main bump** (issue #36).
  `MarkChatRead`, pin, archive, mute and star were all refused with "its
  bootstrap has not completed" (330 `unavailable`, zero `ok`, since the #30
  deploy). The #30 migration stored every app-state collection as not
  bootstrapped, and whatsapp-rust never marked a collection that already has a
  baseline and is at the server's head, so the flag stayed unset. Fixed
  upstream in #1545, which the pin now includes: the first sync of such a
  collection marks it, leaving its version and ltHash alone. The palliative
  that bridged the gap (the #31 conversion marking every collection with
  `version > 0`) is gone, so the conversion carries `bootstrapped` over as it
  was. `tests/appstate_bootstrap.rs` runs the library's processor over both
  engines and fails if the pin ever loses the fix.

- **SIGTERM now stops the daemon** (issue #35). With any `SubscribeEvents`
  stream open, a stop used to hang until systemd SIGKILLed the process after
  90 s: tonic waits, with no deadline, for every connection to close, and an
  event subscription never ends on its own. The socket file was left behind and
  the WhatsApp clients died mid-write. Now every subscription ends with a clean
  end of stream when shutdown starts (a subscriber sees status OK, not a torn
  connection, and should reconnect when the socket is back), anything else gets
  at most `shutdown_grace_ms` (default 10 s), then the accounts stop through
  their graceful stop, the socket is unlinked and `wamux stopped` is logged.

### Changed

- **whatsapp-rust moves to git main `f7468ae2`** (issue #36), same nightly.
  Brings upstream #1545 (see **Fixed**), two keepalive fixes (#1543: pending
  IQs are probed before the watchdog reconnects; #1547: an IQ's write is
  bounded by its deadline), #1542 (a call offer teaches the caller's LID-PN
  pair) and #1544, a new `FavoritesUpdate` event for the favorite-chats sync.
  That event is not typed by the core yet and reaches subscribers as a
  `RawEvent`. The gRPC contract is unchanged.

- **Channel history stays on the core's own IQ** (issue #40). Upstream fixed
  both reasons `GetNewsletterMessages` built the history IQ itself (#1523, the
  addressing; #1518, the dropped `<votes>`), so the library's
  `get_messages` was compared against it over the same page. They agree on
  ids, payloads, reactions, votes and forwards. The library still turns an
  absent row `type` into `text`, drops a `<meta polltype>` it has no variant
  for (or that sits on a non-poll row), fails the whole call on an answer
  without `<messages>` (`Unavailable` through the core), and skips a
  malformed `<vote>`. The first two lose what the server sent (reported as
  upstream #1548), so the core keeps its path. Nothing changes on the wire. The mock now plays back
  `newsletter` IQs, and `tests/stress_newsletter_history.rs` (in
  `scripts/ci.sh`) pins both the agreement and each difference.

- **`DeleteMessage` on a status answers `InvalidArgument`** (issue #41).
  With `for_everyone` on a `status@broadcast` key it used to reach the chat
  revoke, which the library refuses there, and came back `Unavailable`, which
  reads as the core being down. It now names `RevokeStatus`, and is checked
  before the account is, so it answers the same whether or not the account is
  connected. Delete-for-me on a status is unchanged.

- **Channel metadata stays on the core's own projection** (issue #38, the
  #15 queue). `ListSubscribedNewsletters` / `GetNewsletterMetadata` were
  queued to move onto whatsapp-rust's calls once upstream #1372 was fixed. It
  is fixed, but the library folds the answer: it matches the channel state in
  lowercase while the server sends uppercase, so every channel, suspended ones
  included, reads as active (reported as upstream #1546). It also folds unknown
  values, fails a whole list on one malformed node, and turns a missing channel
  into `Unavailable`. The core keeps issuing the queries itself and relays the
  server's tokens verbatim. Nothing changes on the wire. A new stress suite
  (`tests/stress_newsletter_parse.rs`, in `scripts/ci.sh`) pins both halves:
  the core's promise, and a canary that fails when upstream changes.

- **The store's structured blobs are protobuf, not bincode** (issue #31).
  `Device`, app-state versions and app-state sync keys were positional
  bincode: every field whatsapp-rust appended made every stored blob
  unreadable until a one-shot migration, linking the old `wacore` next to the
  new one, rewrote them. That happened on both of the last two upgrades. They
  are now protobuf (`proto/store/blobs.proto`), field-tagged, so a field a blob
  predates decodes as its default; a field upstream adds is a compile error in
  `storage/blob_codec`, answered with a new field number, not an outage.
  - **Operators: nothing to run.** The daemon converts the store on open,
    before any account loads, in one transaction: every blob is converted and
    decoded back against the original (for a `Device`, every persisted field
    including the key material) before anything is written, and one that will
    not convert stops the daemon with nothing changed, naming the row. A new
    `blob_format` table records the result, so later starts skip it. Back up
    the store before the first start, as with any storage change.
  - **Removed:** the `migrate-0-7` and `migrate-0-7-main` features, their bins,
    and the `wacore` 0.6.0 / 0.7.0 dependencies they linked. See the #30 entry
    for a store that still needs them.

- **whatsapp-rust moves from the crates.io 0.7.0 release to git main
  `f4d73ebe`** (issue #30). No new upstream feature is exposed; the gRPC
  contract is unchanged except as noted here.
  - **Operators on a store written before this bump: migrate it with commit
    `ac4c21b` first.** `Device` and `HashState` were positional bincode and main
    changed both layouts. The one-shot bins that bridge them
    (`migrate_0_7_main`, and `migrate_0_7` for a store still on 0.6) were
    deleted by #31, so check out `ac4c21b`, stop the daemon, back up, run
    `cargo run --release --features migrate-0-7-main --bin migrate_0_7_main`
    (dry run) and again with `-- --apply`, then start this build, which
    converts the result to protobuf on open (see #31 below). After it, each
    account's first connect does one full Noise XX handshake (the cached server
    chain is re-verified).
  - **A `@c.us` recipient now echoes as `@s.whatsapp.net`.** The library parses
    the legacy spelling as a phone user (upstream #1371), which is the fix for
    #4: such sends used to be encrypted for nobody. The core still rewrites no
    jid itself.
  - `ListGroups` keeps its full metadata (participants, description): main's
    new listing call returns a slim overview, so the core issues the full query
    itself. An absent group subject still projects as `""`.
  - Media downloads now verify the hashes the message declares (upstream
    #1541): a file that used to download with a mismatching hash now fails.
  - New upstream events (chat lock, sticker sync, and others) reach the socket
    as `RawEvent`, like any variant the core does not type.

- **Breaking (behaviour): `PresenceUpdate.online` is optional and absent on a
  chat state** (issue #24). The field was a hardcoded `true` on every
  `composing`/`recording`/`paused`: `Event::ChatPresence` measures no presence,
  so a consumer lighting an "online" dot off a typing event was lighting it off
  a literal. It is now `optional bool`, present only on real presence
  (`Event::Presence`). Migration: read `online` only when it is set; a consumer
  on the old generated code reads `false` where it used to read `true` on chat
  states, so gate the indicator on presence events. `last_seen` stays `0` on a
  chat state, as before.

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
