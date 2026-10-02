# Changelog

All notable changes to this project are documented here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

While the version is `0.x`, the gRPC contract in `crates/wamux-proto/proto/`
may change in a minor release. Breaking wire changes are called out under
**Changed** with the migration note, since the edge that consumes this socket
has to follow them.

## [Unreleased]

### Added

- **A received sticker pack can be opened** (issue #58). A
  `stickerPackMessage` reached the socket with no `MediaDescriptor`, and
  `DownloadMedia` had no type that could fetch it: an edge saw that a pack
  arrived and could not open it. Now:
  - `InboundMessage.media` carries the pack's ZIP under the new `media_type`
    `sticker_pack`, with `mime_type` empty (the message has no mimetype field),
    and the pack's caption rides `caption`.
  - `InboundMessage.sticker_pack` (field 18, `StickerPackInfo`) carries the
    pack id, name, publisher, description, tray icon file name,
    `stickerPackSize`, the origin as `first_party|third_party|user_created`,
    the thumbnail's size, and each sticker's `file_name`, `is_animated`,
    `emojis`, `accessibility_label`, `is_lottie` and `mime_type`.
  - `StickerPackInfo.thumbnail` is the thumbnail's own descriptor, under
    `media_type` `sticker_pack_thumbnail`: its own path and hashes, the pack's
    media key, and `file_length` 0 because the message declares none.
  - `DownloadMedia` accepts both new types and returns the decrypted ZIP or
    thumbnail like any other file; unzipping is the edge's. `SendMedia` still
    refuses them: sending a pack is not relayed.

  Checked against the pack the issue was measured on (30 stickers, origin
  `third_party`): both descriptors come out complete. Its entries carry no
  `emojis` at all, so the list is empty there, not dropped.

- **A channel poll can be voted on** (issue #26, upstream #1552 and #1555).
  `SendPollVote` could never vote in a `@newsletter` poll: a channel is not
  E2E, so its poll carries no `message_secret`, and the vote is a plaintext
  stanza naming each option by `sha256(option_name)`, the hash the history
  tallies already use. Three `NewsletterService` RPCs:
  - `SendNewsletterPollVote{account, jid, server_id, option_hashes}` sends the
    whole selection; it replaces the previous one, and an empty list removes
    the vote. The answer is the stanza id, the `id` of the `ServerAckEvent`
    that carries the server's verdict. No echo: a vote puts no message in the
    channel. A hash that is not 32 bytes, a repeated one, more than 1000, a
    `server_id` of 0 or a jid off `@newsletter` answer InvalidArgument before
    anything is sent.
  - `GetMyNewsletterAddOns{account, jid, limit}` reads this account's own
    reaction and poll vote per message, timestamps in ms. The tallies count
    every follower, so this is the only read-back of a vote. An absent
    `poll_vote` means never voted; one with no hashes is a removed vote, which
    the server keeps dated.
  - `SubscribeNewsletterLiveUpdates(JidRequest)` asks the server to push the
    channel's tallies (next entry) and answers how long for, in seconds (90
    measured). Renewing is the caller's timer.

  Measured live on 2026-09-25 through WA Web, on the WhatsApp channel's poll:
  one and two options, a change of selection and a removal, all acked, the
  server holding exactly the last list each time.
  `NewsletterReactionCount` and `NewsletterPollVote` moved to
  `newsletter_tallies.proto` so the event below can use them; same package,
  same generated names, no wire change.

- **`NewsletterLiveUpdate`: a subscribed channel's tallies are typed** (issue
  #26, upstream #1554). They reached the socket as an untyped `RawEvent`
  carrying reactions only. `EventEnvelope.newsletter_live_update` (field 27)
  now carries, per message, `reactions`, `votes` (by option hash, polls only)
  and `forwards_count` (absent when the server sent none). A push names only
  the messages whose counts changed, after a first push of the latest ones. A
  consumer matching the `Raw` kind `NewsletterLiveUpdate` should switch to the
  typed event.

- **`FavoritesChanged`: the favorite chats list is typed** (issue #48, upstream
  #1544). When the favorites change on a linked device, the library emits the
  whole list. It reached the socket as an untyped `RawEvent` (kind
  `FavoritesUpdate`); it is now `EventEnvelope.favorites_changed` (field 26),
  with `chats` (the JIDs in the phone's order, spelled as the phone sent them),
  `timestamp` (ms), `from_full_sync`, and `raw` (the serialized
  `FavoritesAction`). Each event replaces the previous list, and an empty
  `chats` means no favorites. It is not an `AppStateUpdate` kind because it is
  one list for the account, not one chat. An entry without an id is skipped
  in `chats` and kept in `raw`. A consumer matching the `Raw` kind
  `FavoritesUpdate` should switch to the typed event.

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

- **Every store method is tested on both engines, and CI keeps it that way**
  (issue #60). The Postgres and SQLite stores implement 59 methods of the
  wacore traits, the Signal, app-state and protocol state of every account;
  41 of them were never called by a test. Before the storage rewrite (#65)
  touches that SQL, `crates/wamux/tests/store_parity/` pins what it does
  today:
  - Each behavior runs as a `postgres_` / `sqlite_` pair asserting the same
    values: round-trip, overwrite, delete, and that one account never sees or
    removes another's rows. Edge semantics are pinned too: the expiry sweeps
    and their cut-off comparisons, `take_sent_message` consuming what it
    returns, the three sender-key clears and their scopes,
    `mark_prekeys_uploaded` never resurrecting a consumed prekey, the most
    recent LID winning for a phone number, the latest sync key id in byte
    order, and a `messageSecret` redelivery never shortening its retention.
  - Three `both_engines_*` tests write the same state through both engines
    and compare the stored columns byte for byte, for every table that holds
    a blob, plus the device registry's JSON. The device blob already had one.
  - `scripts/check-store-coverage.py`, run by `scripts/ci.sh` in both modes,
    fails when a method of either engine's `*_store.rs` is never called
    from `crates/wamux/tests/`, when the two engines declare different
    methods, or when it finds fewer than 59, so a parser that finds nothing
    cannot pass. `scripts/ci.sh --no-postgres` runs the `sqlite_` half.

  No code changed: every new test passed against the stores as they were. The
  methods the stores leave on the trait's default body are tracked in #93.

- **The integration suites run on both engines, wait on conditions, and clean
  up after themselves** (issue #67). The groundwork for the service suites
  (#68 to #71):
  - One `logged_in_client` and one Unix-socket connector in `tests/common`.
    They replace three and three copies.
  - The stress suites build their registry from `WAMUX_TEST_ENGINE`, so
    `scripts/ci.sh` runs every stress stage on SQLite too, including the load
    test and the 199-client scale test in `--full`. The SQLite pass skips the
    tests that name their engine (`postgres_`, `sqlite_`, `both_engines_`),
    because they already ran in the first pass.
  - No test synchronizes on a fixed sleep. Retries go through one bounded
    `poll_until`. Specific replacements:
    - The poll-vote "nothing was sent" check now waits for a valid vote sent
      after the malformed ones (a positive signal).
    - The scale test samples its two-second hold instead of checking once at
      the end.
    - The shutdown tests run on a paused clock.

    `scripts/check-test-sleeps.py`, run in both modes, fails on any `sleep(`
    in a test that lacks a `not a sync point:` comment giving the reason.
  - Every test that creates an account sweeps its own `<suite>/<test>/`
    prefix first and deletes the account at the end. `scripts/ci.sh`
    snapshots the shared Postgres before the first test and fails at the end
    if any account or throwaway database created by the run survived.

- **GroupService is tested through the socket** (issue #68). All 21 RPCs now
  run over a real Unix socket and tonic client, on both engines, with the
  account logged in against the mock. Before this they were only ever run by
  hand.
  - Each RPC has a success test that checks the relayed payload by value. For
    the writes, it also checks the stanza the call put on the wire.
  - Three table tests go through all 21 RPCs: an unknown account gets
    `NotFound`, an account that is not connected gets `FailedPrecondition`,
    and a server refusal (403) comes back as `PermissionDenied` with the
    `wa-code` and `wa-text` trailers.
  - Each `InvalidArgument` branch the core has today has its own test: an
    empty or malformed group, a malformed participant, a subject over 100
    characters, and a description over 2048.
  - The five reads (GetGroupMetadata, GetInviteLink, GetMembershipRequests,
    PreviewInvite, ListGroups) are answered with what WhatsApp's server sent
    on 2026-10-02. The capture came from a throwaway group between the
    owner's own two accounts, was anonymized, and is checked into
    `crates/wamux/tests/group_service/`. A test proves the transcription
    renders back to the captured stanzas. The writes are answered with the
    shapes the whatsapp-rust parser accepts.
  - The mock answers any IQ by namespace, type and first child, can answer
    with a server error, and records every IQ a client sends. #69 to #71
    reuse it.
  - `scripts/check-service-coverage.py`, run in both modes, fails if an RPC
    of the service is never called by its suite, or if the service's RPC
    count changes from the 21 it pins.

  Some behavior is pinned as it is today and is questioned in #96:
  - a group RPC accepts any JID as the group;
  - an empty invite code comes back as `Unavailable`;
  - a 409 on the description loses its code;
  - membership requests carry their `jid` as an object, not a string.

- **DownloadMedia is tested through the socket** (issue #69). It is the one
  MediaService RPC, and it now runs over a real Unix socket and tonic client,
  on both engines. The media comes from `MockCdn`, a real HTTP server on
  loopback, and is fetched by the production HTTP client.
  - The stream is checked by value. A 150 000-byte file arrives as the meta
    frame (mime, length) followed by chunks of 65 536, 65 536 and 18 928
    bytes, equal to the plaintext.
  - Every media type decrypts: image, video, audio, document, sticker, and
    the two sticker-pack types from #58. Channel media without a key is
    verified against `file_sha256` (#6).
  - A tampered ciphertext, a keyless file whose hash does not match, and a
    CDN 404 each end the call without a single frame.
  - An unknown account gets `NotFound`, an account that is not connected
    gets `FailedPrecondition`, and a missing descriptor or an unknown
    `media_type` gets `InvalidArgument`. In all four cases nothing reaches
    the CDN.
  - The library always builds `https://` media URLs. Builds with the
    `stress` feature therefore use `LoopbackHttpClient`, the same ureq
    client with `https://` sent as `http://` only when the host is
    `127.0.0.1`, `localhost` or `[::1]`. A lookalike host such as
    `127.0.0.1.evil.example` is left alone. The production daemon, built
    without `stress`, is unchanged.
  - `scripts/check-service-coverage.py` pins MediaService at 1 RPC.

- **NewsletterService is tested through the socket** (issue #70). All 6 RPCs
  now run over a real Unix socket and tonic client, on both engines, with the
  account logged in against the mock.
  - The five reads (ListSubscribedNewsletters, GetNewsletterMetadata,
    GetNewsletterMessages, GetMyNewsletterAddOns,
    SubscribeNewsletterLiveUpdates) are answered with what WhatsApp's server
    sent on 2026-10-02, together with its answers for a channel that does not
    exist. The capture came from one public channel, was anonymized, and is
    checked into `crates/wamux/tests/newsletter_service/`. A test proves the
    transcription renders back to the captured stanzas.
  - Each read is checked by value: every `Newsletter` field (a list entry
    has no subscriber count and falls back to the preview picture), each
    history row with its body in the event bus's shape and the server's
    tallies, a cleared poll vote kept as a dated vote with no options, and the
    live-update duration.
  - The requests are checked on the wire. History goes to the server and
    names the channel inside, and `before` 0 is left out. The add-ons query
    carries its `limit`, and the subscription goes to the channel with an
    empty `<live_updates/>`. The poll vote is checked against the stanza WA Web
    sent on 2026-09-25 (#26). It was not captured again because a vote on a
    real channel is a public write.
  - Three table tests go through all 6 RPCs: an unknown account gets
    `NotFound`, an account that is not connected gets `FailedPrecondition`,
    and a missing account ref gets `InvalidArgument`. A server refusal (403)
    comes back as `PermissionDenied` with the `wa-code` and `wa-text` trailers
    on the five RPCs that use an IQ. The vote still returns its stanza id,
    since the server's verdict on it arrives later as a `ServerAckEvent`.
  - Each `InvalidArgument` branch has a test, and each confirms that nothing
    reached the wire: an empty or malformed jid, a jid that is not a channel
    (vote, add-ons, subscription), a vote with no poll, a hash that is not 32
    bytes, a repeated option or 1001 options, and a page size of 0.
  - The mock wire helpers and the capture comparison that #68 introduced now
    live in `tests/common/mock_wire.rs`, shared by both suites.
  - `scripts/check-service-coverage.py` pins NewsletterService at 6 RPCs.

  GetNewsletterMetadata and GetNewsletterMessages still send a jid that is not
  a channel to the server. This is pinned as it is today and is questioned in
  #99.

### Fixed

- **`THIRD-PARTY-LICENSES.md` matches what ships, and CI holds it there**
  (issue #83). The file in the image and the tarball still listed the
  whatsapp-rust family at 0.6.0, and since the workspace split (#62) its
  generator wrote a file with 0 crates and exited 0, because it read a graph
  root a virtual manifest does not have. Now:
  - The notices come from the shipped daemon's graph (`wamux`): the crates
    only the tools use (`qrcode`, `image`, `ureq` 2) and the workspace's own
    crates are no longer listed. 341 crates.
  - The whatsapp-rust family, built from git, carries its MIT text. The
    license lives at the root of the upstream repository rather than in each
    crate's directory, so nine of those crates used to be listed without one.
  - `scripts/check-third-party.sh`, run by `scripts/ci.sh` in both modes,
    regenerates the file and fails on any difference, naming the command
    that fixes it (`python3 scripts/gen-third-party.py`). It also demands an
    absolute floor of crates and the whatsapp-rust version in `Cargo.lock`,
    so an empty graph cannot pass. The output does not depend on the
    machine: generated from a freshly downloaded `CARGO_HOME`, it is
    identical byte for byte.

- **`GetNewsletterMetadata` answers `NotFound` for a channel that does not
  exist** (issue #56). The server answers such a JID with a node whose `id`
  is null (`"state": {"type": "NON_EXISTING"}`), not with `null`, so the
  core's own `is_null()` check never fired and it relayed an empty
  `Newsletter` (`jid: ""`, `state: "non_existing"`) with `OK`. The stress test
  that should have caught it answered `null`, which the server never sends; it
  now plays the answer captured live. `ListSubscribedNewsletters` drops an
  entry with no `id` instead of relaying it with an empty jid, as WA Web does.

- **A MEX query the server refuses answers with the server's code** (issue
  #56). A GraphQL error carrying a code (e.g. `405 Not Allowed`, measured on
  `GetNewsletterMetadata` for a channel the account does not follow) came
  back `Unavailable`, which reads as the core being down. It now lifts into
  the same `wa-code` / `wa-text` trailers an IQ rejection carries, and `405`
  (IQ `not-allowed` or MEX `Not Allowed`) maps to `PermissionDenied`.

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

- **The development binaries share one client and fail for real** (issue
  #64). Nothing here is shipped and nothing changes on the wire or in the
  daemon's config; this is for anyone running the tools in
  `crates/wamux-tools`, whose runbook is now
  `crates/wamux-tools/README.md` (what each one needs, reads, proves and
  writes to WhatsApp).
  - One library, `wamux_tools`, holds the socket connection (17 copies
    before), the wait for CONNECTED, the env contract, the check report, the
    delivery judgement, QR rendering, the media helpers and the in-process
    Postgres bootstrap. A binary that cannot connect its account stops
    instead of carrying on.
  - Configuration is environment only, validated before anything is opened:
    `WAMUX_SOCKET_PATH` (default the production socket,
    `$HOME/.local/state/wamux/wamux.sock`, instead of `/tmp/wamux.sock`),
    `WAMUX_REF` (required, no default account), and `WAMUX_LIVE_DEST`
    (required by every binary that writes to someone else's chat, refusing
    the legacy `@c.us` spelling and the account's own number). No phone number
    is hard-coded as a default any more; `e2e_all`, `e2e` and `send_types`
    used to send to one when given no destination.
  - Every binary exits non-zero on a failed check. A check is `PASS` only when
    a returned value was asserted, `ACCEPTED` for an RPC that answers nothing,
    `FAIL` otherwise, and a run is green only with no failure and at least one
    pass. A send to someone else passes on its fan-out reaching the phone and
    a `delivered` receipt, never on the ack.
  - `e2e_all` is non-destructive and runs unattended; the human reception
    window is opt-in (`WAMUX_E2E_INBOUND=1`). Its old destructive phase is the
    new `e2e_destructive`, which needs `WAMUX_E2E_DESTRUCTIVE=yes` and no
    longer logs out or deletes the paired account (`logout_e2e` does that).
  - Removed: `whois` and `e2e` (superseded, and `whois` existed to probe the
    `@c.us` forms #4 retired), `validate1` (covered by `pair_socket`, `set_pfp`
    and `recv_media`), and `send_group` (it opened a second connection on a
    device the daemon already held).
  - `scripts/ci.sh` runs the tools' suites (`cargo test -p wamux-tools`),
    including the built binaries refusing a bad configuration.

- **whatsapp-rust moves to git main `6f07e3ab`** (issue #86), same nightly,
  every crate of the family together. Brings upstream #1568, the fix for
  #1567: since #30 every connection's keepalive loop exited at its first tick,
  so a logged-in account had no idle ping and no dead-socket watchdog. It has
  both again, and `scripts/ci.sh --full` is green through the keepalive stage
  (M2b) for the first time since #30. Also carried:
  - **`AppStateUpdate.raw` and `ContactUpdate.raw` gain `action_timestamp`**
    (upstream #1562). The raw JSON is the library's struct, so the new key
    reaches the socket with no mapping change: the instant the mutation itself
    carried, or `null` when it carried none. `timestamp` keeps its old value,
    which in that case is a fallback (the epoch or the dispatch time), so only
    `action_timestamp` tells a real epoch from a missing one. Additive: a
    consumer that ignores unknown keys sees no change. Applies to `archive`,
    `pin`, `mute`, `star`, `mark_read`, `delete_chat` and the contact update.
  - **The store keeps the status audience** app-state sync delivers (upstream
    #1565, `Device.status_privacy`), as a new optional field in the device
    blob, in the library's own bytes, so unknown modes and custom lists survive
    a restart. A store written before it reads the audience as unknown until
    the phone syncs it again; one that does not decode is also unknown, with a
    warning, and never stops the account from loading. Nothing to run: the
    blob is field-tagged. The new `StatusPrivacyUpdate` event is not relayed
    yet.
  - **The bincode conversion (#31) keeps working.** It read a legacy device
    blob as the current `Device`, and bincode is positional: #1565 inserting a
    field would have made every store still in bincode unconvertible, with
    the tests green, because they built their "legacy" blobs from the current
    type. It now reads a frozen mirror of the layout that wrote it, proved
    against real blobs written by the previous build
    (`crates/wamux/tests/fixtures/bincode-23846f7e/`).
  - Also in the range, internal to the library: pairwise retries keep the
    message's content metadata (#1566), dependency bumps (#1563).

- **The repository is a Cargo workspace** (issue #62). Three crates under
  `crates/`: `wamux-proto` (the `.proto` files, the build script that
  generates them, and nothing of wamux), `wamux` (the daemon, its tests and
  migrations) and `wamux-tools` (the development binaries that lived in
  `src/bin/`, not shipped). For anyone building from source:
  - `cargo build`, `cargo run` and `cargo test` at the root still mean the
    daemon (`default-members`). The release build is
    `cargo build --release -p wamux --bin wamux`; the tools build with
    `-p wamux-tools`.
  - The daemon no longer compiles `ureq` 2, `qrcode` or `image`, which only
    the tools use. `scripts/check-crate-deps.sh`, run by `scripts/ci.sh`,
    fails if any of them comes back, or if `wamux-proto` gains a dependency
    on a wamux crate.
  - `unsafe_code` is forbidden workspace-wide. The build script no longer
    sets `PROTOC` in its environment: it hands the vendored protoc to
    prost's config.

  No wire or config change: the release tarball and the Docker image carry
  the same single `wamux` binary.

- **tonic and prost move to 0.14** (issue #61). `tonic`, `tonic-reflection`,
  `prost` and `prost-types` go up one line, and the codegen moves from
  `tonic-build` to `tonic-prost-build`, since 0.14 split the prost codec into
  its own crates (`tonic-prost` at runtime). No wire change: the `.proto`
  files are untouched, reflection lists the same eight services, and the
  on-disk blob format (#31) encodes byte for byte as before, which the golden
  and cross-engine parity tests pin. The lock no longer carries `tower` 0.4
  next to 0.5, and `scripts/ci.sh` now fails if any crate of the gRPC/HTTP
  stack (tonic, prost, http, hyper, h2, tower) resolves to two versions.

- **whatsapp-rust moves to git main `23846f7e`** (issue #56), same nightly.
  Brings the three newsletter fixes the core had been waiting on, all filed
  from here: #1557 (channel state and verification read in the server's
  spelling, unknown values kept), #1558 (history rows keep the server's
  `type` and `polltype` tokens) and #1561 (a missing channel is `NotFound`,
  one id-less list entry no longer fails the list). Also #1559, which makes a
  channel's `send_reaction` return the stanza id: a breaking signature for
  callers that bind the `()`, and the core never calls it. No other
  dependency moved.

- **Channel metadata and history go through the library** (issue #56, which
  closes out #38 and #40). `ListSubscribedNewsletters`,
  `GetNewsletterMetadata` and `GetNewsletterMessages` stopped issuing their
  own MEX queries and history IQ: every reason they were hand-rolled is fixed
  upstream (see the bump above). What the library still answers differently,
  accepted on purpose:
  - a `role` it has no variant for relays as `""` instead of its token;
  - an absent `state` / `verification` relays as `"active"` / `"unverified"`
    instead of `""`;
  - a history answer without `<messages>` is an error (`Unavailable`), where
    it used to be an empty page. WA Web's own parser requires the node and
    throws, so this is the server's contract, not a new strictness.

  Measured on production before the swap, read-only: 9 subscribed channels
  (list and one get each), every role `SUBSCRIBER`, every state `ACTIVE`,
  verification always present, so no row changes. The stress suites
  (`stress_newsletter_parse`, `stress_newsletter_history`) now pin what the
  core relays by value, with canaries on the two accepted losses.

- **whatsapp-rust moves to git main `f9811768`** (issue #26), same nightly.
  Brings the three channel-poll PRs the new newsletter RPCs sit on (#1552
  `send_poll_vote`, #1554 live tallies, #1555 `get_my_addons`) and #1550,
  opt-in history sharing on a group member add. #1550 touches the send and
  retry paths; read before the bump: its sender-key repair now skips only
  message ids registered as pairwise history bundles, which exist only when
  `add_participants_with_history` is called, and the core never calls it. The
  message-unwrapping list in `classify.rs` is the same 24 wrappers,
  reorganised. No other dependency moved.

- **whatsapp-rust moves to git main `f7468ae2`** (issue #36), same nightly.
  Brings upstream #1545 (see **Fixed**), two keepalive fixes (#1543: pending
  IQs are probed before the watchdog reconnects; #1547: an IQ's write is
  bounded by its deadline), #1542 (a call offer teaches the caller's LID-PN
  pair) and #1544, a new `FavoritesUpdate` event for the favorite-chats sync.
  That event reached subscribers as a `RawEvent` until #48 typed it (see
  **Added**, `FavoritesChanged`). The bump itself left the gRPC contract
  unchanged.

- **`DeleteMessage` on a status answers `InvalidArgument`** (issue #41).
  With `for_everyone` on a `status@broadcast` key it used to reach the chat
  revoke, which the library refuses there, and came back `Unavailable`, which
  reads as the core being down. It now names `RevokeStatus`, and is checked
  before the account is, so it answers the same whether or not the account is
  connected. Delete-for-me on a status is unchanged.

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
