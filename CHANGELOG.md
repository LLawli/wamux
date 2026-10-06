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

- **An experimental Turso storage engine, behind the `turso` cargo feature**
  (issue #106). `database_url = "turso://<path>"` opens the store through the
  native async `turso` crate (pinned `=0.8.1`, no default features), a second
  engine family behind `StorageEngine` next to the sqlx one. Nothing changes
  on the wire, and nothing changes for a build without the feature: it refuses
  `turso://` with an error that names the feature, and the shipped daemon does
  not carry the crate. See `docs/DEPLOYMENT.md`, "Turso (experimental)".
  - A wamux SQLite file opens with `turso://` as it is, and a file Turso wrote
    opens again with `sqlite://`: Turso applies `migrations_sqlite/` and
    records them in `_sqlx_migrations` exactly as sqlx does, checksums
    included, then runs the same bincode upgrade. Both directions are tested,
    and were run live on a production store.
  - Every store statement is now written once, in `storage::statements`, and
    both families run that text. turso binds `$N` by order of first
    appearance, not by number, which put values in the wrong columns with no
    error; the Turso family rewrites `$N` to `?N`, and a test pins the upstream
    behavior. `scripts/check-store-sql-shared.py` keeps SQL out of the
    families.
  - One connection behind a mutex, `foreign_keys` on (checked at open),
    `synchronous = FULL`, a 30 s busy timeout. `maintenance` is the WAL
    checkpoint alone: turso accepts `optimize` and does nothing with it.
  - CI runs the engine-parity suite, the existing-store fixture, the bincode
    upgrade and the service suites on Turso as well (`WAMUX_TEST_ENGINE=turso`).
  - Adding the crate to `Cargo.lock` moved some crates the shipped daemon
    already uses to newer compatible versions (`cc` 1.6, `icu_*` 2.3,
    `zerovec` 0.11.8), because turso 0.8.1 requires them.

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

- **MessagingService is tested through the socket** (issue #71). All 23 RPCs
  now run over a real Unix socket and tonic client, on both engines, with a
  companion account logged in against the mock. Before this, the only test was
  a status-revoke shape check.
  - **What a send put on the wire is checked by value.** The mock gained
    `MockPeer`, a parked whatsapp-rust client with a real Signal identity, and
    serves its devices and prekey bundle. The test then opens what the
    client sent with the peer's own session and compares the message.
  - **Covered this way:**
    - text (plain and with mentions, quote, link preview and timer);
    - reaction, edit, revoke, contact, interactive reply, poll and poll vote;
    - the copy every DM sends to the account's own phone;
    - a group send, opened by both members through the sender key;
    - the status text, media and revoke, opened by each recipient.
  - **Media** goes through a real upload: `MockCdn` now accepts POST. The
    uploaded body is proved to be the plaintext encrypted with the media key
    the peer opened.
  - **The `media_max_bytes` cut-off** and the stream framing errors (no
    header, a chunk first, a second header) are refused before anything is
    uploaded.
  - **Chat actions** (archive, pin, mute, star, mark read and unread, delete
    chat, delete for me) are read back from the app-state patch with the
    seeded sync key, MACs checked, and the mutation is asserted.
  - **Other wire checks:**
    - `MarkRead` sends one read receipt listing every id;
    - `SendPresence` sends the right `<presence>` or `<chatstate>`;
    - `FetchMessageHistory` sends the on-demand request to the account's
      own phone as a peer message, and the session id it returns is that
      stanza's id;
    - `AggregatePollVotes` tallies the vote this account sent.
  - **Statuses:** an unknown account gets `NotFound`, an account that is not
    connected gets `FailedPrecondition`, and a missing account ref gets
    `InvalidArgument`, across all 23 RPCs. Each `InvalidArgument` branch has a
    test that also proves nothing reached the wire.
  - **The live capture.** Four DM sends between the owner's own accounts
    (text, reaction, edit, revoke) were captured live on 2026-10-02 and
    anonymized. A test holds the stanzas the client sends the mock to their
    skeleton.
  - **Mock changes:** `start_as` ends offline delivery, so a send does not
    wait the library's 60 s timeout. Before `<success>`, it sends a contacts
    notification for each served peer, which is how the server teaches a
    client a LID. This matters because a status resolves its recipients from
    local state only. The mock now answers usync and prekey queries per jid,
    answers group metadata, and records receipts, presences and chat states.
    The app-state fixture seeds and opens patches.
  - `scripts/check-service-coverage.py` pins MessagingService at 23 RPCs.

  Pinned as it is today and questioned in #101:
  - a malformed poll (including the proto3 default `selectable_count` 0), a
    mute deadline in the past, and a status with no recipients come back as
    `Unavailable`;
  - a delete-for-me checks the account before the jid;
  - a status recipient given by phone number whose LID the client does not
    know is dropped without a word.

- **The two engines' migrations keep one numbering from now on** (issue #66).
  Postgres 0002 has no SQLite counterpart, so the same change carried
  different numbers (Postgres 0003/0004 are SQLite 0002/0003), and anyone
  comparing the engines had to work out the mapping by hand.
  - No applied migration is renumbered: sqlx stores each one's version and
    checksum.
  - From 0005 on, a change has the same number and name in both
    directories. A change for one engine gets a no-op file of the same name
    on the other, so SQLite's next migration is 0005.
  - `tests/migration_alignment`, run by every `cargo test` with no database,
    fails when a migration exists in only one directory, carries two numbers,
    reuses a historical number, or when the applied history is edited. The
    historical offset is its one named exception.
  - The rule is written in CLAUDE.md.

### Fixed

- **The Postgres engine no longer runs out of pool connections** (issue
  #107). After a `CheckOnWhatsApp`, `GetAbout` or `ListParticipating`, the
  daemon could spend minutes failing every store call with `pool timed out
  while waiting for an open connection`. Signal state stopped flushing, and a
  send could take minutes and answer `Unavailable`.
  - The cause: those three RPCs run a library future that is not `Send`, and
    they ran it on a throwaway runtime built per call. A Postgres connection
    the pool opened during the call stayed bound to that runtime after it was
    gone, and every later acquire that picked it waited out the 30-second
    acquire timeout. Tasks the library started during the call died with the
    runtime too.
  - The call now runs on the daemon's own runtime, from a blocking thread, so
    connections, timers and tasks outlive it.
  - The SQLite engine was not affected: its connections do not use tokio's
    I/O driver. The per-call runtime dates from before 0.1.0, so 0.1.0 on
    Postgres has the same bug.

- **The stores override the wacore trait defaults that were wrong for them**
  (issue #93). Both engines left every trait method with a default body on
  that default, and four of those defaults do something other than what the
  trait asks of a real backend. Now, on Postgres and SQLite alike:
  - `get_sent_message` reads the sent-message row without consuming it. The
    default errored, so a group repair or resend that missed the in-memory
    cache logged a warning and found no payload.
  - `delete_expired_base_keys` prunes `base_keys` older than the keepalive's
    one-hour cutoff. The default deleted nothing, so the table only grew.
  - `touch_tc_token_sender_timestamp` and `store_received_tc_token` are each
    one upsert, atomic against each other as the trait requires. The default
    read-modify-write let a concurrent history sync and send path drop one
    writer's field: in the parity test, 63 (Postgres) and 64 (SQLite) of 64
    contacts lost one.
  - `docs/store-trait-defaults.md` classifies all 36 trait defaults: 5
    overridden, 31 kept, each with the reason. `scripts/check-store-defaults.py`,
    run by `scripts/ci.sh` in both modes, reads the pinned wacore and fails on
    a default the table does not classify, which is how a whatsapp-rust bump
    that adds one is caught. The throughput defaults (batches, `commit_patch`,
    SQLite maintenance) are #104.

  No migration and no wire change.

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

- **BREAKING: the channel fields are proto enums** (issue #128, part 3 of #73,
  which it closes). `Newsletter.verification`, `.state` and `.role`, and
  `NewsletterMessage.type`, `.poll_type` and `.edit` are enums, each under a new
  number with the old one `reserved`. Old string to new value for each:
  `docs/BREAKING-CHANGES-2026-10-05.md`, section 2c.
  - A value the library does not name is `UNKNOWN`, with the server's token
    next to it in `verification_raw`, `state_raw`, `type_raw`, `poll_type_raw`
    or `edit_raw`. The token is now verbatim: an unmodelled state used to relay
    lowercased (`deleted`), and is now `DELETED`.
  - `role` has no raw field: the library drops a role it does not model, so
    that and "no role" are both `UNSPECIFIED`.
  - An absent state still reads `ACTIVE` and an absent verification
    `UNVERIFIED`, the library's defaults (#56).
  - `type` names text, media and poll, the three the server sends. A history
    row with no `type` is `UNSPECIFIED`, not `TEXT`.
  - `poll_type` and `edit` were not on #73's list and joined it: `edit` is the
    `EditAttribute` enum instead of `"3"` / `"8"`.

- **BREAKING: the media type and the presence state are proto enums** (issue
  #127, part 2 of #73). `MediaDescriptor.media_type`, `SendMediaHeader.media_type`
  and `PostStatusMediaHeader.media_type` are `MediaType`, and
  `SendPresenceRequest.state` is `PresenceState`, each under a new number with
  the old one `reserved`. Old string to new value for each:
  `docs/BREAKING-CHANGES-2026-10-05.md`, section 2b.
  - `MediaType` names the seven kinds the core relays, the two download-only
    sticker pack kinds included. The core builds every descriptor from those
    seven, so `MEDIA_TYPE_UNKNOWN` is never emitted and there is no raw field.
    `DownloadMedia` takes the descriptor back as it came.
  - A request refuses every wrong value the same way: `UNSPECIFIED`, `UNKNOWN`,
    a number outside the enum, or a value the operation does not take is
    `InvalidArgument` with `<field> must be one of <A|B|...>, got <value>`,
    where the value is its proto name or the bare number. It used to be
    `unknown media_type '<v>'` and `unknown presence state '<v>'`.
  - Request enums carry no raw field: the edge writes them, and there is no
    value without a name that the core would act on.

- **BREAKING: event fields that were free strings or Rust `Debug` output are
  proto enums** (issue #126, part 1 of #73). `ReceiptEvent.type`,
  `PresenceUpdate.chat_state`, `AppStateUpdate.kind`, `CallEvent.action`,
  `StickerPackInfo.origin` and `UndecryptableEvent.reason` are enums, each under
  a new number with the old one `reserved`. Old string to new value for each:
  `docs/BREAKING-CHANGES-2026-10-05.md`, section 2a.
  - Each enum has `*_UNSPECIFIED = 0` for "no value" and `*_UNKNOWN = 1` for a
    value the library hands over that wamux does not name. Where the original
    survives, it travels next to `UNKNOWN`: an unnamed receipt type in
    `type_raw`, a call action added upstream in `action_raw`.
  - `ConnectionStateChanged.detail`, the `Debug` of the logout reason or of the
    whole ban, is replaced by `logged_out { reason, reason_code }` and
    `ban { reason, reason_code, expire_seconds, message, url }`, each set only
    in its own state. A server code the library does not name is `UNKNOWN`
    with the number in `reason_code`.
  - Every value the library names is an enum value. That includes the `sent`
    receipt and eight call actions that used to relay as the library's wire tag.
  - `scripts/check-wire-debug.py` fails CI on a `Debug` format spec in the
    event mapping outside `variant_name`, the `RawEvent` catch-all.

- **BREAKING: every jid an event carries is the `Jid` message** (issue #122,
  part 3 of #72, which it closes). Eighteen fields of `events.proto`, with the
  rules of parts 1 and 2: a new number with the old one `reserved`, and only
  the `_jid` suffix dropped (`GroupUpdate.group_jid` is now `group`,
  `NewsletterLiveUpdate.newsletter_jid` is now `newsletter`). Before and after
  for each field: `docs/BREAKING-CHANGES-2026-10-05.md`, section 1c.
  - A jid the event does not have is an unset field where it was an empty
    string: `InboundMessage.sender_alt` and `.recipient_alt` when the stanza
    carried none, `PresenceUpdate.chat` on real presence, `ServerAckEvent.from`
    when the server sent none, the echo's `sender` while the account has no jid
    of its own, and the `sender` of a channel-history row.
  - A favorite whose id is empty is skipped like one with no id; `raw` still
    carries both.
  - `EventEnvelope.account_uuid` stays a `string`: it is not a jid, and no
    other uuid in the contract is a message.
  - An edge built against the old contract reads each retyped event field as
    empty (the field is unknown to it), never as the bytes of a `Jid`.
  - `scripts/check-proto-jids.py` has no pending list any more: any `string`
    field named for a jid fails CI.

- **BREAKING: every jid in `groups.proto`, `contacts.proto` and
  `newsletters.proto` is the `Jid` message** (issue #121, part 2 of #72).
  Twenty-five fields, with the rules of part 1: a new number with the old one
  `reserved`, only the `_jid` suffix dropped (`group_jid` is now `group`), an
  unset or empty required jid answers `"missing jid"`, and a jid the core
  answers with is relayed verbatim or left unset. Before and after for each
  field: `docs/BREAKING-CHANGES-2026-10-05.md`, section 1b.
  - Every entry of a repeated jid field (`participants`, the `jids` of
    CheckOnWhatsApp and ResolveLidPn) is still required; an empty one now
    answers `"missing jid"` (it answered `"empty jid"`).
  - `LidPnResult.query` and `CheckResult.query` are `Jid`s carrying the value
    they carried before; ResolveLidPn still echoes each query as it was sent.
  - Which jids an RPC accepts does not change (#96 and #99 decide that).
  - `scripts/check-proto-jids.py` now lists only the event fields of #122.

- **BREAKING: every jid in `common.proto` and `messaging.proto` is the `Jid`
  message** (issue #120, part 1 of #72). Eleven fields that carried a jid as a
  `string` are `Jid` now: `MessageKey.chat` (was `remote_jid`) and
  `.participant`, `Mention.jid`, `QuoteContext.participant`, `poll_creator`
  (was `poll_creator_jid`) on SendPollVote and AggregatePollVotes,
  `PollVote.voter` (was `voter_jid`), `PollOptionResult.voters`, and the
  `recipients` of the three status RPCs. The migration guide, with every
  field's before and after, is `docs/BREAKING-CHANGES-2026-10-05.md`.
  - Each retyped field took a new number and `reserved` the old one, so a
    client built against the old contract is not misread: its field is
    unknown, and the request answers `InvalidArgument("missing jid")`.
  - A required jid that is unset or empty answers `"missing jid"`, the message
    `to` and `chat` already gave; these fields answered `"empty jid"` as
    strings. An unset or empty `participant` is still the DM.
  - A jid the core answers with is relayed verbatim, and an absent one is an
    unset field, never `Jid { value: "" }`.
  - `scripts/check-proto-jids.py` fails CI on a `string` field named for a
    jid. The fields #121 and #122 still have to convert are on its pending
    list, which only shrinks.

- **Groups, contacts, channels and LID lookups take named types too** (issue
  #116, part 3 of #63, which it closes). No function in `domain/` or `state/`
  takes a generated `pb::*` input struct or an identifier as a string any more,
  except event construction (#74). `scripts/check-domain-inputs.py` now fails
  on either.
  - The new types: `NewsletterHistoryQuery`, `NewsletterPollVote`,
    `NewsletterAddOnsQuery` and `LidPnQuery`. A group, a participant, a
    contact and a channel are a `Jid`. `domain/jid_parse.rs` is gone; its
    helpers were the `Jid` constructors.
  - Nothing changes on the wire except one message. A channel that does not
    exist still answers NotFound, but the message is now `newsletter <jid> not
    found`; it read `account no newsletter metadata for <jid> not found`. It
    has a `WamuxError::NotFound` variant of its own.
  - A group RPC and GetNewsletterMetadata/GetNewsletterMessages still accept a
    jid on any server, as before: whether to refuse one is #96 and #99.
    ResolveLidPn still echoes each query as it was sent.
  - `require_field<T>` and `groups::invalid<E>` are concrete now.
    `run_isolated` stays generic, recorded as the one exception: each call
    passes a closure of its own type, and the concrete form would be a boxed
    trait object.

- **The messaging domain takes its own types, not the wire's** (issue #115,
  part 2 of #63). The services convert a request once, at the boundary, into a
  `wamux-types` type, and `domain/` and `state/` never see a generated
  `pb::*` input struct (`scripts/check-domain-inputs.py`, which lists the
  newsletter files #116 still has to convert). The `.proto` does not change,
  and well-formed input goes out exactly as before.
  - The new types: `MessageTarget`, `QuotedRef`, `OutgoingContext`,
    `OutgoingText`, `LinkPreview`, `OutgoingMedia`, `DownloadableMedia`,
    `StatusText`, `StatusMedia`, `StatusRevoke`, `PollVoteCast`,
    `PollVotesToTally`, `NewPoll`, `ContactCard`, `InteractiveReply` and
    `ReplyChoice`. Every function of the messaging domain takes the
    `wamux_types::Jid`.
  - Every error that existed keeps its code, its message and the order the
    checks run in.
  - What changes, for input that was malformed or spelled the legacy way:
    - a mention, a quote's participant, or a target's participant used to
      relay unparsed; a malformed one is now `InvalidArgument`, and a `@c.us`
      one goes out as `@s.whatsapp.net`, the library's parse, as a `@c.us`
      recipient already did;
    - an empty id on the target of SendReaction, EditMessage, DeleteMessage
      or StarMessage is now `InvalidArgument("empty message id")`, where
      before it reached the library;
    - a quote whose key has an empty id is now
      `InvalidArgument("empty quote.quoted.id; expected the id of the quoted
      message")`;
    - a quote with no key quotes nothing, as before, and a SendText whose only
      extra was such a quote now goes as a plain `conversation` instead of an
      extended text with no context.

- **`wamux-types`: the domain's named types and its error, in a crate of
  their own** (issue #114, part 1 of #63). For anyone using the `wamux` crate
  as a library (today `crates/wamux-tools`). Nothing changes on the wire
  except the two edge cases at the end of this entry.
  - `Jid`, `GroupJid`, `NewsletterJid`, `AccountId`, `ExternalRef`,
    `AccountRef` and `MessageId` are validated when they are built. `Jid`
    wraps the library's parsed `Jid`, so the library's parse stays the one
    normalization (`@c.us` comes out `@s.whatsapp.net`, as sends already did).
  - One `MediaKind` replaces three: what SendMedia accepts, what a status can
    be, and the two download-only sticker-pack types. Every `media_type`
    token on the wire now comes from it.
  - `WamuxError` and its `tonic::Status` mapping moved to the new crate, and
    that mapping is now the only place a `Status` is built
    (`scripts/check-status-sites.py`). The services no longer build any by
    hand: "account is not connected" always goes through
    `WamuxError::NotConnected`. The `Database(sqlx::Error)` variant is gone,
    since nothing built it.
  - `AccountRegistry::resolve` takes an `AccountRef`. `client_err` stays in
    the daemon, which is the only crate that knows whatsapp-rust.
  - The two edge cases that changed:
    - a `wa-text` trailer whose text is not ASCII is now left out, where
      before it carried the raw UTF-8 bytes in an ASCII trailer (the code
      still rides `wa-code`, and the text stays in the status message);
    - `account <uuid> not found` names the uuid in lowercase, whatever case
      the request used.

- **The stores write batches in one transaction and read them in one query
  per hundred** (issue #104). Seventeen wacore store methods that were left on
  their default, a loop over the one-row method, are now overridden on both
  engines. Nothing changes on the wire, in the config or in the stored bytes.
  - Batch writes (identities, sessions, prekeys, sender keys, LID mappings,
    device lists) run the same one-row statement in one transaction: one
    connection and one commit instead of one per row, and all or nothing. The
    signal cache flush already treats a failed batch as wholly unwritten.
  - Batch reads (sessions, prekeys, mutation MACs, device lists, tc-tokens)
    send one query per 100 values with a fixed-size `IN` list.
  - `commit_patch` writes the app-state version, the removed MACs and the
    added MACs in one transaction. A crash between them could leave a new
    version next to old MACs, and the next patch's ltHash was then computed
    from the wrong set.
  - `maintenance`, which the keepalive calls about hourly, now refreshes
    SQLite's planner statistics (`PRAGMA optimize`) and truncates the `-wal`
    file, which otherwise only grows between checkpoints. A busy checkpoint is
    skipped, not failed. On Postgres it does nothing (autovacuum).
  - Measured on a dev machine (release build): on Postgres a 200-session
    flush, a 256-user device-list write and 100 `commit_patch` calls each took
    300 to 650 ms, one commit per row. On SQLite, which runs in process on one
    connection, the same work took 10 to 30 ms. See the PR for the numbers
    after the change.

- **The storage SQL is written once for Postgres and SQLite** (issue #65).
  Nothing changes on the wire, in the config or in the stored bytes: an
  existing database or file opens as it is, with no migration and no
  re-pairing. This is for anyone using the `wamux` crate as a library (today
  `crates/wamux-tools`).
  - `storage/postgres/` and `storage/sqlite/` were the same code written twice
    (about 1,400 lines each). They are now one family, `storage::sql`:
    `SqlStore` (the `StorageEngine`), `SqlBackend` (the per-account wacore
    backend) and `SqlPool { Pg, Sqlite }`. `PgStorage`, `SqliteStorage`,
    `PgBackend` and `SqliteBackend` are gone, with no aliases. The engine is
    still picked by the `database_url` scheme.
  - Each statement is one `$N` string that runs on both drivers: sqlx-sqlite
    binds `$N` by number, which a test pins. The dialect differences became
    portable SQL (`CASE` instead of `GREATEST`/`MAX`, `length` instead of
    `octet_length`, the empty blob passed as a bind). Three statements are
    still written per driver, side by side: the prekey array update, the
    `blob_format` row lock and the `accounts` uuid (UUID on Postgres, TEXT on
    SQLite).
  - `StorageEngine` stays the only plug point. A non-sqlx engine is a family
    of its own behind it, and #106 adds Turso that way. The CI checks of store
    coverage and trait defaults now count families, and fail on a
    `*_store.rs` outside every listed family.
  - `tests/existing_store` opens a store written by `0f40e34`, before this
    change, on each engine, and reads every table back. The fixture is in
    `crates/wamux/tests/fixtures/store-0f40e34/`.

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
