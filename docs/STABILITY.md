# Stability

wamux is `0.x`. This page says which parts of its surface you can build on,
which may still move, and how a change that breaks you is announced. Where a
line is a statement of intent rather than something the code or a check
enforces, it says so.

The surface has two halves:

- **The wire**: the gRPC API, package `wamux.v1`, defined by the `.proto` files
  in [`crates/wamux-proto/proto/`](../crates/wamux-proto/proto/). This is what
  an edge or a client builds against.
- **The operator surface**: `wamux.toml` keys and their `WAMUX_*` variables,
  their defaults, the CLI (`wamux store ...`), how the socket is bound, the
  store's schema and the release artifacts.

## The policy while the version is 0.x

- **A minor release may break either half.** `0.2.0` broke the wire on purpose
  (every jid became the `Jid` message, free strings became enums, JSON inside
  `bytes` became typed messages) and changed how the daemon binds the socket,
  stages media and bounds its defaults.
- **A patch release is meant to be compatible** (intent). It has not always
  been: `0.2.2` changed what `SendPoll` does for a channel, because the library
  started publishing channel polls the way WA Web does. When the library or the
  server forces such a change in a patch release, it ships with a guide like
  any other break.
- **Every breaking change ships an upgrade guide** under
  [`docs/upgrading/`](upgrading/README.md), and a `CHANGELOG.md` entry under
  **Changed** that says it breaks and links the guide. This is the rule from
  here on, kept by review, not by a check. So far it held for every change to
  the wire's shape and to what startup needs; smaller breaks were announced in
  the CHANGELOG only, such as `0.2.2` refusing a malformed `quote.quoted.chat`
  it used to ignore, or its image losing the shell.
- **The package stays `wamux.v1`.** It kept that name through the `0.2.0`
  break, so the package name is not a compatibility signal while the version
  is `0.x`. Build against the `.proto` files of the release you run, not
  against `main`.
- **Additions can arrive in any release**, patch included: a new RPC, a new
  field, a new enum value, a new event case in `EventEnvelope`. `0.2.4` added
  `quoted_message` to three sends, for example.

What counts as breaking: a removed or renumbered field or RPC, a field whose
type changes, an RPC that does something different with the same signature
(`MarkRead` started sending a read receipt in 2026-09), a request that used to
succeed and is now refused, a config default that changes behaviour, and a
startup that used to succeed and now fails.

### How the wire breaks, when it does

Verified in the protos, and the convention every past break followed:

- **A retyped field takes a new number**, and the old number (and name, when it
  changes) is `reserved`. A client built on the old contract then reads an
  unknown field and drops it, instead of decoding new bytes into the old type.
- **Enums have `*_UNSPECIFIED = 0`**, and most have `*_UNKNOWN = 1` for a value
  this build does not name, often with the raw value next to it (`type_raw`,
  `unknown_action`). New values appear in additive releases. A client has to
  expect an enum number it does not know and a `oneof` case it does not know,
  and must not treat either as an error.

## What is stable

The evidence: no RPC has ever been removed. `0.1.0` had 61, today there are 89,
and all 61 are still there. Since `0.2.0` every `.proto` change has been
additive at the schema level (no field removed, renumbered or retyped); the
one break, the channel poll above, changed behaviour, not shape.

**Stable** means: changes only in a minor release, with an upgrade guide, and
only for a reason the guide states (intent). It covers every RPC that `0.2.0`
shipped, except the channel surface below:

| Service | RPCs |
|---|---|
| `AccountService` | all: `CreateAccount`, `ListAccounts`, `GetAccountStatus`, `PairWithQr`, `PairWithCode`, `ConnectAccount`, `DisconnectAccount`, `Logout`, `DeleteAccount` |
| `EventService` | `SubscribeEvents`, and every typed case of `EventEnvelope` |
| `MessagingService` | all |
| `MediaService` | `DownloadMedia` |
| `GroupService` | the 21 RPCs of `0.2.0`, from `CreateGroup` to `RejectMembershipRequests` |
| `ContactService` | `CheckOnWhatsApp`, `GetProfilePicture`, `SetProfilePicture`, `RemoveProfilePicture`, `GetPushName`, `SetPushName`, `GetAbout`, `GetBusinessProfile`, `SubscribePresence`, `ResolveLidPn`, `ListLidMappings` |
| `AdminService` | `Check`, `GetMetrics` (the RPC; the metrics in it are operator surface) |

The `RawEvent` kinds `gap` and `subscription_gap` are stable too: they are the
core's own markers, documented in `events.proto`.

## What may still move

**Provisional** means: shipped and tested, but its shape may change in the
next minor release, with a guide (intent). It covers RPCs added after `0.2.0`,
which no minor release has carried yet:

| Service | RPCs | Added in |
|---|---|---|
| `CallService` | `RejectCall` | `0.2.1` |
| `ContactService` | `BlockContact`, `UnblockContact`, `GetBlocklist`, `SetAbout` | `0.2.1` |
| `ContactService` | `UnsubscribePresence` | unreleased |
| `GroupService` | `SetGroupMembershipApproval`, `SetGroupMemberAddMode`, `SetGroupMemberLinkMode`, `SetGroupMemberShareHistoryMode` | `0.2.1` |
| `NewsletterService` | `FollowNewsletter`, `UnfollowNewsletter`, `MuteNewsletter`, `CreateNewsletter` | `0.2.1` |

It also covers **the channel surface as a whole**: all of `NewsletterService`,
and `SendPoll` to a `@newsletter` jid. Channels are not
end-to-end encrypted, their tallies are server-side nodes, and what the core
sends follows what WA Web sends as the library tracks it. That already changed
once in a patch release.

## What is not a contract

These can change in any release, without a guide:

- **`RawEvent` with any other kind.** Its payload is the library's own JSON,
  relayed so an event the core has not typed yet is not lost. That JSON can
  change on any library bump, and a kind stops arriving as `RawEvent` the day
  the core types it (`0.2.2` typed four). Build on a kind only until its typed
  event lands.
- **Bytes that are WhatsApp's, not the core's.** `raw_message` (a serialized
  `wa.Message`), the `payload_json` of a `MexNotification`, and the raw fallbacks next
  to an `UNKNOWN` enum. The fields are stable; what is inside them is whatever
  the server or the library sends.
- **The Turso engine** (`turso://`, behind the `turso` cargo feature). It is
  experimental, the release binaries do not carry it, and it is not
  stress-tested: see [Turso (experimental)](DEPLOYMENT.md#turso-experimental).
- **The `wamux.store` package** in `proto/store/`. It is the store's on-disk
  blob format, compiled apart from the API and kept out of reflection.
- **The Rust crates.** `wamux-proto`, `wamux-types` and `wamux-tools` are not
  published (`publish = false`), and the `wamux` library API is the daemon's
  own. The `.proto` files are the contract, not the Rust that implements them.
- **The development binaries** in `crates/wamux-tools`.

## The operator surface

Config keys, defaults, the CLI, the socket bind and the metrics follow the same
policy as the wire: they may break in a minor release, with a guide. `0.2.0`
shipped three such guides (the socket bind, the media spool and the bounded
defaults), and the `method` label of the metrics changed there too.

The store is the one part that only moves forward:

- **Migrations run forward, on startup.** There is no down migration. Back up
  the store before upgrading; going back to an older release after a newer one
  migrated the store is not supported.
- **A known break is coming in `0.3.0`**: it refuses a store whose blobs are
  still in the old bincode format. `0.2.x` is the last line that converts one,
  so upgrade through a `0.2.x` release first. `AdminService/Check` reports
  `blob_format`; `BLOB_FORMAT_PROTOBUF` is ready. See
  [Upgrading a store written before #31](DEPLOYMENT.md#upgrading-a-store-written-before-31).

## Before 1.0

What `1.0` will promise is not decided. When it is, this page will say so; until
then, the tables above are the best guide to what is safe to build on.
