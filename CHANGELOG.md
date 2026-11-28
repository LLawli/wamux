# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

While the version is `0.x`, the gRPC contract in `crates/wamux-proto/proto/`
may change in a minor release. Breaking changes are marked **Breaking** under
**Changed**, and what to do about each is in the upgrade guides,
[docs/upgrading/](docs/upgrading/README.md), one per version step.

## [Unreleased]

### Added

- **Android (Termux) and Debian packages** (#277). Each release ships
  `wamux-<version>-aarch64-linux-android.tar.gz` and `wamux_<version>_aarch64.deb`
  (for Termux, with a `termux-services` service, disabled until
  `sv-enable wamux`), and `wamux_<version>_amd64.deb` (static binary, systemd
  user unit and docs, for Debian and Ubuntu). See `docs/TERMUX.md` and
  `docs/DEPLOYMENT.md`, "Debian package".
- **A `termux` cargo feature** (#277): the socket, the media spool and the SQLite
  store default to paths under Termux's prefix. A plain Android build has no
  defaults and refuses to start until `socket_path`, `media_spool_dir` and
  `database_url` are set.
- **Sound stickers** (#278). `SendMedia` with `media_type` STICKER and the new
  `sticker_audio` header field takes the clip as `sticker_audio` parts (Ogg/Opus,
  not transcoded); `media_max_bytes` applies to each file. On receive,
  `InboundMessage.sticker_audio` describes the clip, downloadable like any audio.
  Misuse is `INVALID_ARGUMENT` before anything is sent. Additive.
- **`ContactService.UnsubscribePresence`** (#271) releases a `SubscribePresence`.
  On error the subscription stays in place.
- **`DeleteMessage` revokes another member's group message as an admin** (#276):
  `for_everyone` with `from_me = false` and a `participant` in a group. A non-admin
  gets a `ServerAckEvent` with `error` `"403"`. No proto change.

### Changed

- **SQLite creates the store's directory** (#277), mode `0700`, when it does not
  exist. An existing directory is not touched.
- **Breaking: `DeleteMessage` for everyone refuses a key no revoke can be built
  from** (#276): `from_me = false` outside a group, or in a group without
  `participant`, is `INVALID_ARGUMENT` and sends nothing. It used to send a
  revoke of a different message. See
  [0.2.4 to next](docs/upgrading/0.2.4-to-next.md).
- **whatsapp-rust `c03cb783` and stable Rust `1.100.0`** (#281): a source build
  needs no nightly. The store format is unchanged, and the previous binary opens
  a store the new one wrote.
- **Text and media inside a wrapper are projected** (#281): a message wrapped in
  a spoiler, mention, view-once, ephemeral, audio sticker, edit or `device_sent`
  envelope now carries `text`, `media` and `caption`, and a revoke inside an
  ephemeral wrapper is seen as a delete. No proto change.
- **The offline drain dispatches up to 8 stanzas at a time** (#281): after a
  reconnect, `monotonic_seq` order holds within each chat, not across chats.
- **`RecipientFanout.addressed` counts hosted devices** (#281), so it can be
  higher for a WhatsApp Business account on Coexistence.

### Fixed

- **`ListAccounts` carries each account's `jid`** (#274), as `GetAccountStatus`
  does; unset only for an account that never paired.
- **The streaming sidecar of a sent video or audio matches the WhatsApp apps'**
  (#281, upstream #1922).
- **`CreateAccount` with a taken `external_ref` answers `ALREADY_EXISTS`** (#269)
  on every engine; it answered `INTERNAL`.

### Security

- **`url` 2.5.9** (RUSTSEC-2026-0592): `Url::origin()` could overflow the stack
  on nested `blob:` prefixes. Lockfile only.

## [0.2.4] - 2026-11-10

### Added

- **`SendText`, `SendMedia` and `SendContact` take `quoted_message`** (#230), the
  quoted message's serialized `wa.Message`, so receivers without the original
  still show the quote. Bytes that are not a `wa.Message` are `InvalidArgument`.
- **Config key `blocking_shutdown_timeout_ms`** (#239, env
  `WAMUX_BLOCKING_SHUTDOWN_TIMEOUT_MS`, default 5000): how long shutdown waits for
  blocking tasks before exiting without them.

### Changed

- **whatsapp-rust `66eb7331`.** No proto change. The library now verifies device
  lists against their ADV signature: a companion it cannot verify is left out of
  a group send, and its messages arrive undecryptable. A group action this build
  does not know arrives as `GroupUnknownAction`. WhatsApp error text in the logs
  is redacted (`other(len=N)`). The store stays readable by 0.2.3. See
  [0.2.3 to 0.2.4](docs/upgrading/0.2.3-to-0.2.4.md).

### Fixed

- **A contact whose identity changes no longer breaks its device-list
  resolution**, in a group refresh too, on all three engines.
- **A bot stuck in its handshake no longer outlives `stop`** (#252), and its
  connection-budget slot is always returned.
- **`GetAccountStatus` reads `LOGGED_OUT` as soon as `Logout` returns** (#257),
  even with a burst of events queued.
- **Breaking: `PairWithQr` and `PairWithCode` refuse an account that holds a
  paired identity** (#253) with `FailedPrecondition`, including a logged-out
  one, instead of hanging. To pair again, `DeleteAccount` and `CreateAccount`.
- **`PairWithQr` and `PairWithCode` fail when the connect fails** (#250), with
  `ResourceExhausted` or `Unavailable`, instead of staying open with no frame.
- **An open pairing stream no longer holds the shutdown** (#247): it ends cleanly
  with no terminal update, like `SubscribeEvents`.
- **The daemon exits after SIGTERM even when a blocking task is stuck** (#239);
  before, a stalled connect kept it alive until SIGKILL.
- **Pairing on the published image no longer fails with
  `SQLITE_IOERR_GETTEMPPATH`** (#229): SQLite keeps temporaries in memory and the
  image ships a `/tmp`.
- **The group commit no longer wedges every later write** after SQLite rolls a
  batch back on its own (I/O error, full disk) (#229).
- **pulldown-cmark 0.13.6** (RUSTSEC-2026-0433). Build time only.

### Security

- **RUSTSEC-2026-0459 is ignored, with a trigger** (#260): the bundled SQLite can
  return uninitialized memory when it opens a database whose `-wal` file has a
  different page size. The fix is out of reach until sqlx lifts its cap; the
  daemon opens only its own store (`0600` in a `0700` directory).

## [0.2.3] - 2026-10-29

### Added

- **A native macOS build** (#231): `wamux-<version>-macos-universal.pkg` and its
  `.sha256`, with a build provenance attestation. Not signed or notarized by
  Apple (the README documents the Gatekeeper path). It installs
  `/usr/local/bin/wamux` and an example LaunchAgent, and starts nothing.

### Changed

- **The socket default on macOS is `/var/run/wamux.sock`** (`/run` cannot be
  created there). Linux keeps `/run/wamux.sock`.

### Fixed

- **`THIRD-PARTY-LICENSES.md` is the same on Linux and macOS** (#231), and now
  includes simdutf8's Apache text.
- **pulldown-cmark 0.13.5** (RUSTSEC-2026-0414). Build time only.

## [0.2.2] - 2026-10-26

### Added

- **A private reply, and a reply to a status, can be sent** (#217): a
  `quote.quoted.chat` other than the destination goes out as the context's
  `remoteJid`, on SendText, SendMedia, SendContact and SendInteractiveReply.
- **Four library events are typed** (#215): `AccountSettingUpdate.unarchive_chats`,
  `EventEnvelope.label_sublist = 38`, `call_log_history = 39` and
  `reachout_timelock = 40`. Optional library fields cross as `optional`.

### Changed

- **Breaking: the release is one static musl binary** (#218). The tarball is
  `wamux-<version>-x86_64-unknown-linux-musl.tar.gz`, and the GHCR image carries
  the same file. See [0.2.1 to 0.2.2](docs/upgrading/0.2.1-to-0.2.2.md).
- **Breaking: the image is built on `scratch`**, with no shell, so
  `docker exec ... sh` no longer works. `Dockerfile.glibc` keeps a Debian
  runtime. The image creates `/var/cache/wamux`, the default media spool.
- **The daemon sets its own umask to `077`**, for every command.
- **mimalloc is the musl build's allocator**, which keeps its CPU cost per
  message within 2% (Postgres) and 10% (SQLite) of glibc.
- **Breaking: a malformed `quote.quoted.chat` on a send is `InvalidArgument`**
  (#217); the field used to be ignored.
- **Breaking: the `RawEvent` kinds `UnarchiveChatsSettingUpdate`,
  `LabelSublistUpdate`, `CallLogHistory` and `ReachoutTimelockUpdate` no longer
  arrive** (#215): read the typed events above.
- **Breaking: a poll sent to a channel follows WA Web** (#220): `selectable_count`
  is 0 or 1, `message_secret` is empty, a blank question or option is refused.
  Polls to chats and groups are unchanged.
- **whatsapp-rust `3870ce5c`** (#220): a lock mutation without a `locked` value is
  no longer reported as an unlock, and media transfers fall back to the next
  address within a 10-second connect budget.

### Fixed

- **An inbound quote names the quoted message's chat, and a reply with media
  keeps its quote** (#217). `QuoteContext.quoted.chat` was always the reply's
  chat, and media replies arrived without quote or mentions.
- **Events of one account arrive in `monotonic_seq` order, and a replay during a
  burst no longer loses one** (#213). `monotonic_seq` is strictly increasing per
  account and starts again at 0 when the daemon restarts.

### Security

- **ctutils 0.4.5** (RUSTSEC-2026-0385): non-constant-time comparisons, reached
  through the library's hashing. Lockfile only.

## [0.2.1] - 2026-10-21

### Added

- **`CallService.RejectCall`** (#200) declines an incoming call, with `call_id`,
  `peer` and `call_creator` from the `CallEvent`. Placing and answering calls
  stay out of scope.
- **`NewsletterService`: `FollowNewsletter`, `UnfollowNewsletter`,
  `MuteNewsletter` and `CreateNewsletter`** (#199). A mutation the server does not
  apply answers `Unavailable`.
- **`GroupService`: `SetGroupMembershipApproval`, `SetGroupMemberAddMode`,
  `SetGroupMemberLinkMode` and `SetGroupMemberShareHistoryMode`** (#198).
- **`ContactService`: `BlockContact`, `UnblockContact`, `GetBlocklist` and
  `SetAbout`** (#197). A block needs the contact's LID-phone mapping; without it
  the answer is `FailedPrecondition` (learn it with `ResolveLidPn` or
  `CheckOnWhatsApp` first).

### Changed

- **whatsapp-rust `eab45737`** (#205). No proto change, but the library imposes
  new behavior: delete-for-me relays (and for someone else's message requires)
  `participant`; star and delete-for-me refuse a channel message or an empty id;
  `GetProfilePicture` reports a rate limit as `ResourceExhausted`; a malformed
  media descriptor is `InvalidArgument` and a CDN 403/404/410 is no longer
  retried; `SendPoll` to a broadcast list and `SendPollVote` in a channel are
  refused; `GetAccountStatus` reads `LOGGED_OUT` after `Logout`; a heavy inbound
  burst recycles the connection. See [0.2.0 to 0.2.1](docs/upgrading/0.2.0-to-0.2.1.md).
- **sqlx 0.9** (#209): a Postgres `.pgpass` file now interprets backslash
  escapes.

### Fixed

- **`GetGroupMetadata`, `ListGroups` and `GroupCreated` report the three member
  modes** (#205), which were always UNSPECIFIED.

### Security

- **rustls-webpki 0.103.16 and webpki-roots 1.0.10** (RUSTSEC-2026-0382,
  RUSTSEC-2026-0381): certificate verification could accept the distrusted emSign
  intermediates. They verify the WhatsApp transport, the media CDN and sqlx's TLS.
- **No more webpki-roots 0.26 shim or `rsa` in the tree** (#209), so both
  advisory ignores are gone. SQLite is built without `load-extension` and
  `deserialize`.

## [0.2.0] - 2026-10-19

Most of this release is a new contract and a new store format. Read
[0.1 to 0.2](docs/upgrading/0.1-to-0.2.md) before upgrading: a 0.1.0 store must
be migrated first.

### Changed

- **Breaking: a 0.1.0 store must be migrated before 0.2 opens it** (#30, #31).
  Run `migrate_0_7` and then `migrate_0_7_main` from commit `ac4c21b`; 0.2 then
  converts the store's blobs from bincode to protobuf on its first open, in one
  checked transaction. Back up first. See
  [the store](docs/upgrading/0.1-to-0.2.md#the-store).
- **Breaking: socket bind** (#77). A live socket at `socket_path` stops startup
  instead of being taken over, a non-socket is refused, `socket_group` now
  `chown`s the socket and fails startup when it cannot, and the socket is never
  visible wider than `socket_mode`. New optional peer allowlist: `allowed_uids`,
  `allowed_gids`. See [the socket](docs/upgrading/0.1-to-0.2.md#the-socket-77).
- **Breaking: media is spooled to disk** (#79). `media_spool_dir` (default
  `/var/cache/wamux`) must be writable or startup fails; DownloadMedia refuses a
  file above `media_max_bytes`; `media_spool_max_bytes` (default 2 GiB) bounds all
  transfers. Memory per transfer no longer grows with the file. See
  [the media spool](docs/upgrading/0.1-to-0.2.md#the-media-spool-79).
- **Breaking: bounded defaults** (#80). `replay_max_event_bytes` defaults to 256
  KiB (was no cap), and the `method` metric label is `unknown` for a path the
  daemon does not serve. See
  [bounded defaults](docs/upgrading/0.1-to-0.2.md#bounded-defaults-80).
- **SQLite runs at `synchronous = FULL` with group commit** (#173): a power loss
  no longer drops acknowledged commits, reads never wait for writes, and an
  in-memory DSN is refused. Turso gets the same group commit (#172).
- **Breaking: `MarkRead` sends the read receipt** (#20), reading `message_ids`
  and `sender`. It used to mark the chat read on the account's own devices,
  which is now `MarkChatRead`.
- **Breaking: every jid in the contract is the `Jid` message** (#120, #121,
  #122). Each retyped field takes a new number and drops only its `_jid` suffix;
  a missing required jid answers `"missing jid"`. See
  [every jid](docs/upgrading/0.1-to-0.2.md#every-jid-is-the-jid-message-72).
- **Breaking: free strings and Rust `Debug` output are proto enums** (#126, #127,
  #128): receipt type, chat state, media type, presence state, sticker pack
  origin, undecryptable reason, the channel fields, and the logout and ban
  details that replace `ConnectionStateChanged.detail`. A request enum outside
  its set is `InvalidArgument`. See
  [enums](docs/upgrading/0.1-to-0.2.md#enums-instead-of-strings-73).
- **Breaking: JSON inside `bytes` fields is typed** (#132, #133, #134, #135):
  group metadata, membership requests, the business profile, and the group,
  contact, app-state and call events. Times are milliseconds and absent values
  are unset. Only `RawEvent.payload` stays JSON. See
  [typed messages](docs/upgrading/0.1-to-0.2.md#typed-messages-instead-of-json-74).
- **Breaking: `PresenceUpdate.online` is optional** (#24), set only on real
  presence; it was a hard-coded `true` on chat states.
- **Breaking: `ContactService.GetPushName` takes an `AccountRef`** (#1): it
  always answered the account's own push name.
- **Breaking: a malformed mention or participant on a send is `InvalidArgument`**
  (#115), and so is an empty target message id or quote id; they used to be
  relayed unparsed.
- **A `@c.us` jid is parsed as `@s.whatsapp.net`** (#30), so it echoes that way.
  Such sends used to be encrypted for nobody (#4).
- **Media downloads verify the hashes the message declares** (#30).
- **`DeleteMessage` for everyone on a status answers `InvalidArgument`** (#41),
  naming `RevokeStatus`.
- **Channel reads go through the library** (#56): a history answer without
  `<messages>` is `Unavailable` (it was an empty page), and a missing channel's
  error reads `newsletter <jid> not found` (#116).
- **A `wa-text` trailer that is not ASCII is left out** (#114), and
  `account <uuid> not found` names the uuid in lowercase.
- **Store batches are atomic** (#104): batch writes run in one transaction, and
  `commit_patch` writes the app-state version and its MACs together, so a crash
  can no longer leave them out of step. SQLite maintenance truncates the WAL.
- **whatsapp-rust `6f07e3ab`** (#86): every connection has its keepalive and
  dead-socket watchdog again, and the store keeps the status audience.
- **The repository is a Cargo workspace** (#62). The release build is
  `cargo build --release -p wamux --bin wamux`; the tools are `-p wamux-tools`.

### Added

- **Encryption at rest** (#164, #165, #166). `store_key_file` (a `0600` file with
  64 hex characters) seals the store's secrets with XChaCha20-Poly1305. An
  existing store is converted on start (resumable), `wamux store decrypt --yes`
  reverses it, and `wamux store rotate-key --new-key-file <path>` changes the
  key. Losing the key means re-pairing every account. See `docs/DEPLOYMENT.md`.
- **The store file is created `0600`** (#75), and an existing file others can
  read is reported with a `WARN` naming the `chmod 600` that fixes it.
- **`AdminService.Check` reports the store's blob format** (#84):
  `BLOB_FORMAT_PROTOBUF` means it is ready for 0.3.
- **`MessagingService.GetStatusAudience`** (#89) returns the status audience the
  library holds for the account.
- **An experimental Turso storage engine** behind the `turso` cargo feature
  (#106): `database_url = "turso://<path>"`, file-compatible with `sqlite://`
  both ways. The shipped daemon does not include it.
- **Events that were `RawEvent` are typed** (#148, #149, #150, #151, #153):
  `EventEnvelope.sticker = 28`, `label = 29`, `quick_reply = 30`,
  `account_setting = 31`, `self_push_name = 32`, `call_log = 33`,
  `connection_notice = 34`, `contact_notice = 35`, `device_notice = 36`,
  `mex = 37`, new `AppStateUpdate` cases, `ContactUpdate.removed`, a missed call
  as `CallEvent.missed`, and three pairing updates. Each old `RawEvent.kind` stops
  arriving. See
  [RawEvent kinds](docs/upgrading/0.1-to-0.2.md#rawevent-kinds-that-became-typed-141).
- **PairWithQr and PairWithCode end when nothing more can come** (#150): after a
  refused or expired code, or used-up QR refs, besides `paired` and `error`.
- **A forced logout and a ban carry the server's whole stanza** (#138) as a
  `StanzaNode`, with an account lock's appeal token.
- **The raw element of group create, link, unlink and sub-group suggestions**
  (#146), as a `StanzaNode`.
- **GroupService and channel RPCs refuse a jid of the wrong kind** (#96, #99), and
  `GroupJidResponse.join_outcome` tells a join from a pending request.
  MessagingService answers `InvalidArgument` for a poll, mute or status that can
  never work (#101). See
  [new refusals](docs/upgrading/0.1-to-0.2.md#new-refusals).
- **Channel polls can be voted on** (#26): `SendNewsletterPollVote`,
  `GetMyNewsletterAddOns` and `SubscribeNewsletterLiveUpdates`, with the tallies
  typed as `EventEnvelope.newsletter_live_update`.
- **`FavoritesChanged`** (#48): the favorite chats list, typed.
- **`NewsletterMessage.original_timestamp` and `last_edit_timestamp`** (#51), in
  ms.
- **A received sticker pack can be opened** (#58): its ZIP and thumbnail are
  descriptors `DownloadMedia` accepts, and `InboundMessage.sticker_pack` describes
  the pack.
- **`RevokeStatus`** (#41) takes a posted status down; send the same recipients.
- **The six library-built sends echo** (#38): poll, vote, edit, revoke and status
  posts publish a from-me `InboundMessage` like the other sends.
- **`OfflineSyncInterrupted`** (#38), typed: the drain was cut and the remainder
  comes back.
- **`PresenceUpdate.chat`** (#24): the conversation a chat state happened in.
- **The LID-phone mapping** (#1): `InboundMessage.sender_alt` and
  `recipient_alt`, `ContactService.ResolveLidPn` and `ListLidMappings`.

### Deprecated

- **Converting a bincode store** (#84, #179). 0.2.x is the last line that
  converts a store written before #31; 0.3.0 refuses one. Upgrade through 0.2.x
  before 0.3; see `docs/DEPLOYMENT.md`, "Upgrading a store written before #31".

### Fixed

- **SIGTERM stops the daemon** (#35) with event subscriptions open: they end
  cleanly, anything else gets `shutdown_grace_ms`, and the socket is unlinked.
  Before, systemd had to SIGKILL it.
- **The Postgres engine no longer runs out of pool connections** (#107) after
  `CheckOnWhatsApp`, `GetAbout` or `ListParticipating`. 0.1.0 on Postgres has
  this bug.
- **The stores override wacore defaults that were wrong for them** (#93):
  `get_sent_message` reads without consuming, `base_keys` older than an hour are
  pruned, and two tc-token writers no longer drop each other's field.
- **`GetNewsletterMetadata` answers `NotFound` for a channel that does not exist**
  (#56); a MEX error carries `wa-code`/`wa-text`, and `405` is `PermissionDenied`.
- **`THIRD-PARTY-LICENSES.md` matches what ships** (#83).

### Security

- **`contrib/wamux.service` creates the store's directory `0700`** (#188). Copy
  the new unit, `daemon-reload` and restart.
- **rustls 0.23.46** (RUSTSEC-2026-0346 to -0349).
- **The release publishes only what passed its gates, with provenance** (#78):
  pinned actions and base images, `--locked` builds, and attestations you can
  check with `gh attestation verify` (`docs/DEPLOYMENT.md`, "Verifying a
  release").

## [0.1.0] - 2026-08-28

First tagged release. The daemon multiplexes many WhatsApp accounts in one
process and serves a gRPC API over a Unix domain socket.

### Added

- **Multi-account core.** One process, N accounts, each with its own Signal
  session state, connection supervisor and event stream. `AccountService`
  covers the lifecycle: create, list, pair (QR and phone code), connect,
  disconnect, logout, delete.
- **Event fan-out.** `EventService.SubscribeEvents` streams typed events per
  account or across all accounts (accounts paired later join an open stream). A
  per-account replay ring lets a reconnecting subscriber pick up recent events;
  one that falls too far behind gets an explicit `subscription_gap` marker.
- **Messaging.** Text, media (image, video, audio, document, sticker, PTT, PTV),
  reactions, edits, deletions, link previews, ephemeral messages, contacts,
  polls and status posting. Chat actions: read/unread, star, archive, pin, mute,
  delete, presence.
- **Groups.** Creation and membership, permissions and settings, ephemeral
  timers, invite previews, group photo, and membership approval.
- **Contacts.** WhatsApp presence check, profile picture get/set/remove, push
  name get/set.
- **Storage behind a `StorageEngine` trait**: `postgres://` for many accounts,
  `sqlite://` for a single host. Both store byte-identical blobs, so a store is
  portable between them.
- **Relay-pure persistence.** Only `whatsapp-rust`'s Signal/session/device
  state is stored. No message history, ever.
- **Unix socket transport** with configurable mode (default `0660`) and owning
  group, graceful shutdown on SIGTERM/SIGINT, and socket unlink on exit.
- **Observability.** Structured `tracing` logs (text or JSON), one span per
  request, plus `AdminService` with a Prometheus render and a health check.
- **Install surface.** Docker image, `docker-compose.yml`, and a hardened
  systemd unit in `contrib/`. See [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md).
- **Third-party attribution** (`THIRD-PARTY-LICENSES.md`), shipped inside the
  Docker image.

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

[Unreleased]: https://github.com/LLawli/wamux/compare/v0.2.4...HEAD
[0.2.4]: https://github.com/LLawli/wamux/compare/v0.2.3...v0.2.4
[0.2.3]: https://github.com/LLawli/wamux/compare/v0.2.2...v0.2.3
[0.2.2]: https://github.com/LLawli/wamux/compare/v0.2.1...v0.2.2
[0.2.1]: https://github.com/LLawli/wamux/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/LLawli/wamux/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/LLawli/wamux/releases/tag/v0.1.0
