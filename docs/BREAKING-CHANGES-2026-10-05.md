# Breaking changes for the edge: the 0.2.0 contract (Phase D, from 2026-10-05)

The `wamux.v1` package keeps its name and breaks in 0.2.0, as the CHANGELOG
allows while the version is `0.x`. This document collects every Phase D change
to the contract (#72 split into #120, #121 and #122, then #73, #74, #96, #99
and #101); each issue adds its section here as it merges. Until the last one
lands, the contract is mid-migration: build the edge against a released
`0.2.0`, not against `main`.

**Consumers to update before deploying 0.2.0:** the edge, and `wamux-omarchy`
(the local mirror daemon, its own repository), which carries its own copy of
the `.proto` and builds `MessageKey`, `Mention` and the poll requests from it.

## 1. One jid representation: the `Jid` message (#72)

Before, the contract carried a jid two ways: the `Jid` message in some fields
and a bare `string` in about 50 others, so the same value changed type between
two RPCs and the schema could not say which string was a jid. Every field that
carries a jid becomes `Jid`.

**The rules, the same for every field:**

- **A retyped field takes a new number.** The old number is `reserved` (and its
  name, when the name changes). A `string` and a message share protobuf wire
  type 2, so keeping the number would let an old client's string bytes decode
  as a `Jid`, or fail to decode. With a new number, an old client's field is
  unknown and skipped: a request from an edge that was not updated answers
  `InvalidArgument("missing jid")` instead of being misread.
- **Names lose the `_jid` suffix, nothing else.** `remote_jid` becomes `chat`,
  `poll_creator_jid` becomes `poll_creator`. A field already named `jid` in a
  message about that entity, or already named for its role (`participant`,
  `recipients`, `voters`), keeps its name.
- **A request's `Jid` is parsed at the boundary.** A required one that is unset,
  or set with an empty `value`, answers `InvalidArgument("missing jid")`. Several
  of these fields answered `"empty jid"` while they were strings; `to` and
  `chat` already answered `"missing jid"`, and now every field does. A
  malformed one answers `InvalidArgument("invalid jid '<value>': <reason>")`.
- **An optional one is absent when unset or empty.** A DM's `participant` can
  be left unset or sent as `Jid { value: "" }`, as the empty string was.
- **A `Jid` the core answers with is relayed verbatim.** It is not parsed or
  normalized on the way out, and one the core does not have is an unset field,
  never `Jid { value: "" }`. In proto3 terms: check presence
  (`key.participant.is_some()` / `has_participant()`), not an empty string.

### 1a. `common.proto` and `messaging.proto` (#120)

| Message | Before | After |
|---|---|---|
| `MessageKey` | `string remote_jid = 1` | `Jid chat = 5` |
| `MessageKey` | `string participant = 4` | `Jid participant = 6` |
| `Mention` | `string jid = 1` | `Jid jid = 2` |
| `QuoteContext` | `string participant = 2` | `Jid participant = 3` |
| `SendPollVoteRequest` | `string poll_creator_jid = 4` | `Jid poll_creator = 7` |
| `PollVote` | `string voter_jid = 1` | `Jid voter = 4` |
| `AggregatePollVotesRequest` | `string poll_creator_jid = 3` | `Jid poll_creator = 7` |
| `PollOptionResult` | `repeated string voters = 2` | `repeated Jid voters = 3` |
| `PostStatusTextRequest` | `repeated string recipients = 5` | `repeated Jid recipients = 6` |
| `PostStatusMediaHeader` | `repeated string recipients = 7` | `repeated Jid recipients = 8` |
| `RevokeStatusRequest` | `repeated string recipients = 3` | `repeated Jid recipients = 4` |

`MessageKey`, `Mention` and `QuoteContext` travel in requests (every RPC that
acts on a message by its key, every send with a quote or mentions), in
`SendResult.key`, in events (`InboundMessage.key`, `.reaction_target`,
`.protocol_target`, `.quote`, `.mentions`) and in channel history
(`NewsletterMessage.message.key`). All of them change together.

**What does not change:** what the core sends to WhatsApp. A request that was
well formed before reaches the wire exactly as it did; the spelling rules of
#115 still hold (the library's parse is the one normalization, so a `@c.us`
mention still goes out as `@s.whatsapp.net`). `EditMessage` still answers with
`key.chat` exactly as the caller wrote it.

**What the edge has to do:**

- Wrap each of these values in `Jid { value }` and use the new field names.
- Read `key.chat.value` where it read `key.remote_jid`, and treat an unset
  `participant` as "none" (a DM, or a key the core built for its own send).
- Expect `"missing jid"` where an empty creator, voter, mention, recipient or
  key chat used to answer `"empty jid"`.

### 1b. `groups.proto`, `contacts.proto` and `newsletters.proto` (#121)

| Message | Before | After |
|---|---|---|
| `CreateGroupRequest` | `repeated string participants = 3` | `repeated Jid participants = 4` |
| `GroupJidResponse` | `string group_jid = 1` | `Jid group = 3` |
| `ParticipantsRequest` | `string group_jid = 2` | `Jid group = 4` |
| `ParticipantsRequest` | `repeated string participants = 3` | `repeated Jid participants = 5` |
| `GroupTextRequest` | `string group_jid = 2` | `Jid group = 4` |
| `GroupRef` | `string group_jid = 2` | `Jid group = 3` |
| `GroupToggleRequest` | `string group_jid = 2` | `Jid group = 4` |
| `GroupEphemeralRequest` | `string group_jid = 2` | `Jid group = 4` |
| `SetGroupPhotoRequest` | `string group_jid = 2` | `Jid group = 4` |
| `GroupSummary` | `string jid = 1` | `Jid jid = 5` |
| `ParticipantChange` | `string jid = 1` | `Jid jid = 7` |
| `ParticipantChange` | `string phone_number = 4` | `Jid phone_number = 8` |
| `CheckOnWhatsAppRequest` | `repeated string jids = 2` | `repeated Jid jids = 3` |
| `CheckResult` | `string query = 1` | `Jid query = 4` |
| `CheckResult` | `string jid = 3` | `Jid jid = 5` |
| `JidRequest` | `string jid = 2` | `Jid jid = 3` |
| `SubscribePresenceRequest` | `string jid = 2` | `Jid jid = 3` |
| `LidPnMapping` | `string lid = 1` | `Jid lid = 5` |
| `LidPnMapping` | `string pn = 2` | `Jid pn = 6` |
| `ResolveLidPnRequest` | `repeated string jids = 2` | `repeated Jid jids = 3` |
| `LidPnResult` | `string query = 1` | `Jid query = 4` |
| `Newsletter` | `string jid = 1` | `Jid jid = 10` |
| `GetNewsletterMessagesRequest` | `string jid = 2` | `Jid jid = 5` |
| `SendNewsletterPollVoteRequest` | `string jid = 2` | `Jid jid = 5` |
| `GetMyNewsletterAddOnsRequest` | `string jid = 2` | `Jid jid = 4` |

`JidRequest` is shared: it is the request of GetProfilePicture, GetAbout,
GetBusinessProfile, GetNewsletterMetadata and SubscribeNewsletterLiveUpdates.

**What does not change:** which jids an RPC accepts. A group RPC,
GetNewsletterMetadata and GetNewsletterMessages still take a jid on any
server (refusing one is #96 and #99, later in this document). The three
channel RPCs that refused another server still do, with the same message:
`'<value>' is not a channel: expected a jid ending in @newsletter`. The JSON
payloads (`GroupSummary.metadata`, `GroupMetadataResponse`, membership
requests) keep their shape; their jid spelling is #96.

**What the edge has to do:**

- Wrap each value in `Jid { value }` and use `group` where it sent `group_jid`.
- Each entry of `participants`, `jids` (CheckOnWhatsApp, ResolveLidPn) is
  required: an empty one fails the whole request with `"missing jid"`, where
  it answered `"empty jid"`.
- `LidPnResult.query` still echoes the requested jid exactly as it was sent (a
  `@c.us` query comes back `@c.us`), now inside a `Jid`.
- Treat an unset `LidPnMapping.lid` / `.pn` as "no user part known" (it was an
  empty string), and an unset `ParticipantChange.phone_number` as "the server
  sent none".
