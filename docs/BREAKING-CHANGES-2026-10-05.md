# Breaking changes for the edge: the 0.2.0 contract (Phase D, from 2026-10-05)

The `wamux.v1` package keeps its name and breaks in 0.2.0, as the CHANGELOG
allows while the version is `0.x`. This document collects every Phase D change
to the contract (#72 split into #120, #121 and #122, #73 into #126, #127 and
#128, #74 into #132, #133, #134 and #135, then #138, #141 split into #148, #149, #150 and #151, then #96, #99 and #101); each issue adds its section here as it merges. Until the last one
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

### 1c. `events.proto` (#122, closes #72)

| Message | Before | After |
|---|---|---|
| `InboundMessage` | `string chat = 2` | `Jid chat = 21` |
| `InboundMessage` | `string sender = 3` | `Jid sender = 22` |
| `InboundMessage` | `string sender_alt = 16` | `Jid sender_alt = 23` |
| `InboundMessage` | `string recipient_alt = 17` | `Jid recipient_alt = 24` |
| `ReceiptEvent` | `string chat = 1` | `Jid chat = 6` |
| `ReceiptEvent` | `string sender = 2` | `Jid sender = 7` |
| `UndecryptableEvent` | `string chat = 1` | `Jid chat = 4` |
| `UndecryptableEvent` | `string sender = 2` | `Jid sender = 5` |
| `PresenceUpdate` | `string jid = 1` | `Jid jid = 6` |
| `PresenceUpdate` | `string chat = 5` | `Jid chat = 7` |
| `GroupUpdate` | `string group_jid = 1` | `Jid group = 4` |
| `PushNameUpdate` | `string jid = 1` | `Jid jid = 3` |
| `ContactUpdate` | `string jid = 1` | `Jid jid = 4` |
| `AppStateUpdate` | `string chat = 1` | `Jid chat = 4` |
| `FavoritesChanged` | `repeated string chats = 1` | `repeated Jid chats = 5` |
| `NewsletterLiveUpdate` | `string newsletter_jid = 1` | `Jid newsletter = 3` |
| `CallEvent` | `string from = 1` | `Jid from = 5` |
| `ServerAckEvent` | `string from = 3` | `Jid from = 6` |

`InboundMessage` is also the row of channel history (`NewsletterMessage.message`,
GetNewsletterMessages), so its four fields change there too.

**`EventEnvelope.account_uuid` stays a `string`.** It is not a jid, and no other
uuid in the contract is a message (`AccountRef.uuid`, `Account.uuid`), so it
keeps the shape every other uuid has.

**What does not change:** which events are emitted and what they carry. Every
value is the one the string field held, verbatim: a `@lid` sender is still a
`@lid`, a chat state in a direct chat still names the sender's own jid, and a
favorites list keeps the phone's order and spelling. The JSON in `raw`
(`GroupUpdate`, `ContactUpdate`, `AppStateUpdate`, `CallEvent`) keeps its
shape; typing it is #74.

**What the edge has to do:**

- Read `.value` of each of these fields, and use `group` and `newsletter` where
  it read `group_jid` and `newsletter_jid`.
- Check presence where it checked for an empty string. These are unset when the
  core has no value: `sender_alt` and `recipient_alt` when the stanza carried
  none, `PresenceUpdate.chat` on real presence (a chat state always has one),
  `ServerAckEvent.from` when the server sent none or sent one that did not
  parse, `InboundMessage.sender` on the echo of a send made before the account
  had a jid of its own, and `sender` on a channel-history row.
- Expect a favorite with an empty id to be skipped, as one with no id already
  was. `raw` still carries every entry.
- An edge that is not updated still decodes every event, and reads each of
  these fields as empty: the old number is unknown to the new contract and the
  new one is unknown to the old edge.

With this section, every field of the contract that carries a jid is the `Jid`
message. `scripts/check-proto-jids.py` keeps it that way: there is no list of
exceptions, so a new `string` field named for a jid fails CI.

## 2. Enums with a raw fallback instead of free strings and `Debug` (#73)

Before, enum-like values travelled as free strings whose valid values lived in
comments, and three of them were the Rust `Debug` output of a library value, so
a rename in whatsapp-rust silently changed the contract. Each one becomes a
proto enum.

**The rules, the same for every field:**

- **`*_UNSPECIFIED = 0`** means the event has no value for the field (a real
  presence has no chat state, a sticker pack may carry no origin).
- **`*_UNKNOWN = 1`** means a value the library hands over that wamux does not
  name. Where the original survives, a raw field next to it carries it, set only
  on `UNKNOWN`: `string *_raw` for a token, `int32 *_code` for a numeric server
  code. Where the library's set is closed there is no raw field, and `UNKNOWN`
  is either the library's own "unknown" or never emitted (each enum says which).
- **A retyped field takes a new number,** and the old one is `reserved`. A
  string and an enum do not even share a wire type, so under the old number an
  old edge would fail to decode the event.
- Read the enum, never its number: values are named, and the numbers are not
  the server's codes.

### 2a. `events.proto` (#126)

| Message | Before | After |
|---|---|---|
| `ReceiptEvent` | `string type = 4` | `ReceiptType type = 8`, `string type_raw = 9` |
| `PresenceUpdate` | `string chat_state = 4` | `ChatState chat_state = 8` |
| `AppStateUpdate` | `string kind = 2` | `AppStateKind kind = 5` |
| `CallEvent` | `string action = 3` | `CallActionKind action = 6`, `string action_raw = 7` |
| `StickerPackInfo` | `string origin = 8` | `StickerPackOrigin origin = 12` |
| `UndecryptableEvent` | `string reason = 3` (`Debug`) | `UnavailableReason reason = 6` |
| `ConnectionStateChanged` | `string detail = 2` (`Debug`) | `LoggedOutInfo logged_out = 3`, `TemporaryBanInfo ban = 4` |

Old values to new ones:

- **`ReceiptEvent.type`:** `delivered` → `RECEIPT_TYPE_DELIVERED`, `read` →
  `READ`, `played` → `PLAYED`, `read-self` → `READ_SELF`, `played-self` →
  `PLAYED_SELF`, `sender` → `SENDER`, `retry` → `RETRY`, `enc_rekey_retry` →
  `ENC_REKEY_RETRY`, `server-error` → `SERVER_ERROR`, `inactive` → `INACTIVE`,
  `peer_msg` → `PEER_MSG`, `hist_sync` → `HISTORY_SYNC`, and `sent` (which no
  comment listed) → `SENT`. Any other token, which used to relay verbatim, is
  `RECEIPT_TYPE_UNKNOWN` with the token in `type_raw`.
- **`PresenceUpdate.chat_state`:** `composing`, `recording`, `paused` →
  `CHAT_STATE_COMPOSING`, `RECORDING`, `PAUSED`; `""` (real presence) →
  `CHAT_STATE_UNSPECIFIED`.
- **`AppStateUpdate.kind`:** `archive`, `pin`, `mute`, `star`, `mark_read`,
  `delete_chat` → `APP_STATE_KIND_ARCHIVE` ... `DELETE_CHAT`.
- **`CallEvent.action`:** `offer`, `offer_notice`, `pre_accept`, `accept`,
  `reject`, `terminate` → `CALL_ACTION_KIND_OFFER` ... `TERMINATE`. The eight
  actions that used to relay as the library's wire tag are named too:
  `transport`, `relaylatency` (`RELAY_LATENCY`), `video` (`VIDEO_STATE`),
  `group_update`, `enc_rekey`, `waiting_room_update`, `user_action`
  (`RAISE_HAND`), `screen_share`. One the library adds later is
  `CALL_ACTION_KIND_UNKNOWN` with its wire tag in `action_raw`.
- **`StickerPackInfo.origin`:** `first_party`, `third_party`, `user_created` →
  `STICKER_PACK_ORIGIN_FIRST_PARTY` ... `USER_CREATED`; `""` → `UNSPECIFIED`.
- **`UndecryptableEvent.reason`:** `ViewOnce`, `Hosted`, `Bot` →
  `UNAVAILABLE_REASON_VIEW_ONCE`, `HOSTED`, `BOT`; `Unknown` →
  `UNAVAILABLE_REASON_UNKNOWN`.
- **`ConnectionStateChanged.detail`** is gone. On `LOGGED_OUT`, `logged_out`
  carries `reason` (a `LogoutReason`, the server's `<failure>` codes: `401` is
  `LOGOUT_REASON_LOGGED_OUT`, `403` is `ACCOUNT_LOCKED`, and so on; each value's
  code is in `events.proto`). On `BANNED`, `ban` carries `reason` (a
  `BanReason`), `expire_seconds` (how long the ban lasts, not a deadline),
  `message` and `url`. A code the library does not name is `*_UNKNOWN` with the
  number in `reason_code`, where it used to read `Unknown(416)` inside `detail`.
  Both are unset in every other state, where `detail` was `""`.

**What does not change:** which events are emitted, and every value they relay.
`RawEvent.kind` stays a string, because it is the catch-all for a library event
wamux does not know; `ServerAckEvent.class` stays a string too, because it is
the server's open set.

**What the edge has to do:**

- Compare against the enum values where it compared strings, and read
  `type_raw` / `action_raw` / `reason_code` only when the value is `UNKNOWN`.
- Read the logout and ban details from `logged_out` / `ban` instead of parsing
  `detail`.
- Treat `UNSPECIFIED` as "no value" where it treated `""` that way.

### 2b. Media type and presence state (#127)

The first enums an edge *writes*: two of the four fields are request-only, and
`MediaDescriptor` goes both ways (an event carries it, `DownloadMedia` takes it
back as it came).

| Message | Before | After |
|---|---|---|
| `MediaDescriptor` (`common.proto`) | `string media_type = 7` | `MediaType media_type = 8` |
| `SendMediaHeader` | `string media_type = 9` | `MediaType media_type = 16` |
| `PostStatusMediaHeader` | `string media_type = 2` | `MediaType media_type = 9` |
| `SendPresenceRequest` | `string state = 3` | `PresenceState state = 4` |

Old values to new ones:

- **`media_type`:** `image`, `video`, `audio`, `document`, `sticker` →
  `MEDIA_TYPE_IMAGE`, `VIDEO`, `AUDIO`, `DOCUMENT`, `STICKER`; the two
  download-only kinds of a received sticker pack (#58), `sticker_pack` and
  `sticker_pack_thumbnail` → `MEDIA_TYPE_STICKER_PACK` and
  `MEDIA_TYPE_STICKER_PACK_THUMBNAIL`. The set is closed: the core builds every
  descriptor from these seven, so an event never carries `MEDIA_TYPE_UNKNOWN`,
  and there is no raw field.
- **`state`:** `available`, `unavailable`, `composing`, `recording`, `paused` →
  `PRESENCE_STATE_AVAILABLE`, `UNAVAILABLE`, `COMPOSING`, `RECORDING`, `PAUSED`.

Which values each RPC takes is unchanged: `SendMedia` takes the five sendable
kinds, `PostStatusMedia` takes `IMAGE` and `VIDEO`, `DownloadMedia` takes all
seven, `SendPresence` takes the five states.

**The refusal, one shape for every request enum:** `UNSPECIFIED`, `UNKNOWN`, a
number the enum does not name, and a value the RPC does not take are all
`InvalidArgument` with

```
<field> must be one of <A|B|...>, got <value>
```

where `<value>` is the proto name, or the bare number when the enum has no name
for it. For example:

```
media_type must be one of MEDIA_TYPE_IMAGE|MEDIA_TYPE_VIDEO|MEDIA_TYPE_AUDIO|MEDIA_TYPE_DOCUMENT|MEDIA_TYPE_STICKER, got MEDIA_TYPE_STICKER_PACK
status media_type must be one of MEDIA_TYPE_IMAGE|MEDIA_TYPE_VIDEO, got MEDIA_TYPE_AUDIO
state must be one of PRESENCE_STATE_AVAILABLE|PRESENCE_STATE_UNAVAILABLE|PRESENCE_STATE_COMPOSING|PRESENCE_STATE_RECORDING|PRESENCE_STATE_PAUSED, got 42
```

These replace `unknown media_type '<v>'`, `status media_type must be
image|video, got '<v>'` and `unknown presence state '<v>'`. The code
(`InvalidArgument`) and the point in the RPC where the check happens are the
same as before: `SendMedia` and `PostStatusMedia` still read the type after the
byte stream, and `SendPresence` still resolves the account and the chat first.

**What the edge has to do:**

- Set the enum where it set a string. Leaving the field unset now reads as
  `UNSPECIFIED` and is refused, where an empty string was refused as an unknown
  token.
- Pass the descriptor from the event to `DownloadMedia` untouched, as before.
- Match the new refusal text if it parsed the old one; better, match the code.

### 2c. Channel fields (#128, closes #73)

| Message | Before | After |
|---|---|---|
| `Newsletter` | `string verification = 6` | `NewsletterVerification verification = 11`, `string verification_raw = 12` |
| `Newsletter` | `string state = 7` | `NewsletterState state = 13`, `string state_raw = 14` |
| `Newsletter` | `string role = 8` | `NewsletterRole role = 15` |
| `NewsletterMessage` | `string type = 3` | `NewsletterMessageType type = 11`, `string type_raw = 12` |
| `NewsletterMessage` | `string poll_type = 7` | `NewsletterPollType poll_type = 13`, `string poll_type_raw = 14` |
| `NewsletterMessage` | `string edit = 8` | `EditAttribute edit = 15`, `string edit_raw = 16` |

`poll_type` and `edit` were not on #73's list; they are the same case and
joined it.

Old values to new ones:

- **`Newsletter.verification`:** `verified`, `unverified` →
  `NEWSLETTER_VERIFICATION_VERIFIED`, `UNVERIFIED`.
- **`Newsletter.state`:** `active`, `suspended`, `geosuspended` →
  `NEWSLETTER_STATE_ACTIVE`, `SUSPENDED`, `GEOSUSPENDED`.
- **`Newsletter.role`:** `owner`, `admin`, `subscriber`, `guest` →
  `NEWSLETTER_ROLE_OWNER`, `ADMIN`, `SUBSCRIBER`, `GUEST`; `""` →
  `NEWSLETTER_ROLE_UNSPECIFIED`.
- **`NewsletterMessage.type`:** `text`, `media`, `poll` →
  `NEWSLETTER_MESSAGE_TYPE_TEXT`, `MEDIA`, `POLL`; `""` (no attribute) →
  `UNSPECIFIED`. Any other token is `UNKNOWN` with the token in `type_raw`.
  That includes the five the library names but documents as never sent by the
  server (`reaction`, `revoke`, `poll_creation`, `poll_vote`, `edit`).
- **`NewsletterMessage.poll_type`:** `creation`, `quiz_creation`, `vote`,
  `result_snapshot`, `edit` → `NEWSLETTER_POLL_TYPE_CREATION` ...
  `RESULT_SNAPSHOT`, `EDIT`; `""` → `UNSPECIFIED`. It is still read on any row,
  as before, not only on a `POLL` row.
- **`NewsletterMessage.edit`:** `"1"` → `EDIT_ATTRIBUTE_MESSAGE_EDIT`, `"2"` →
  `PIN_IN_CHAT`, `"3"` → `ADMIN_EDIT`, `"7"` → `SENDER_REVOKE`, `"8"` →
  `ADMIN_REVOKE`, `""` → `UNSPECIFIED`. The enum numbers are not the server's
  tokens.

**Two things that are not a plain rename:**

- **The raw value is now verbatim.** A state or verification the library does
  not model used to relay lowercased (`deleted`, `pending_review`). It is now
  `UNKNOWN` with the server's spelling in `state_raw` / `verification_raw`
  (`DELETED`, `PENDING_REVIEW`). The core relays the token; it does not rewrite
  it.
- **Two defaults stay as they were (#56).** The library reads an absent state as
  `ACTIVE` and an absent verification as `UNVERIFIED`, and the core cannot tell
  those apart from a value the server sent. So `NEWSLETTER_STATE_UNSPECIFIED`
  and `NEWSLETTER_VERIFICATION_UNSPECIFIED` are never emitted. A role the
  library does not model reaches the core as no role at all, so it is
  `NEWSLETTER_ROLE_UNSPECIFIED`, and there is no `role_raw` because no token
  survives.

**What the edge has to do:**

- Compare against the enum values where it compared strings, and read the
  `*_raw` field only when the value is `UNKNOWN`.
- Stop lowercasing or case-folding an unknown state or verification. It arrives
  as the server spelled it.
- Read `edit` as the enum instead of matching `"3"` and `"8"`.

With this section, the Phase D enum work (#73) is complete: every enum-like
value in the contract is a proto enum. Four stay strings because they relay an
open set the server or the library can grow at any time (decided in #126):
`RawEvent.kind`, `ServerAckEvent.class`, `ParticipantChange.status` and
`PairedInfo.platform`.

## 3. Typed messages instead of JSON inside `bytes` (#74)

Some responses and events carried the library's data as JSON inside a
`bytes` field: a second schema inside the first, undocumented, that moved
whenever a library struct changed, and that became an empty payload when it
failed to serialize. Each one is now a protobuf message. `RawEvent.payload`
stays JSON, because its job is to carry a library event wamux does not know.
The protobuf blobs the server itself sends (`HistorySyncEvent.raw`,
`InboundMessage.raw_message`, `FavoritesChanged.raw`) are not JSON and do not
change.

The rules every new message follows:

- **Absent is unset.** A field the server left out is an unset `optional`, an
  unset message (`Jid`, or a sub-message), or an enum at `*_UNSPECIFIED`. It
  never crosses as `0` or `""`.
- **Times are milliseconds**, like every timestamp in this contract. A value
  whose unit is not verified crosses as the server sent it, as `uint64`, and the
  field's comment says so.
- **Enum-like values are enums** (section 2). A closed library set has no raw
  field; an open one has `*_raw`, set only on `UNKNOWN`.

### 3a. Group metadata, membership requests, business profile (#132)

| Message | Before | After |
|---|---|---|
| `GroupJidResponse` | `bytes metadata = 2` | `GroupMetadata metadata = 4` (unset after JoinWithInvite) |
| `GroupMetadataResponse` | `bytes metadata = 1` | `GroupMetadata metadata = 2` |
| `GroupSummary` | `bytes metadata = 4` | `GroupMetadata metadata = 6` |
| `MembershipRequestsResponse` | `bytes requests = 1` | `repeated MembershipRequest requests = 2` |
| `BusinessProfileResponse` | `bytes raw = 1` | `BusinessProfile profile = 2` |

`GroupMetadata` lives in the new `group_metadata.proto`. `GroupSummary` keeps
`jid`, `subject` and `participants` as they were.

**Group metadata, JSON key to field.** The JSON had five keys; the message has
every field the library parses (56), so the rest are new.

| JSON | `GroupMetadata` |
|---|---|
| `id` (string) | `id` (`Jid`) |
| `subject` (`""` when absent) | `subject` (`optional string`, unset when absent) |
| `description` (`null` when absent) | `description` (`optional string`) |
| `addressing_mode` (`"pn"`, `"lid"`) | `addressing_mode` (`GROUP_ADDRESSING_MODE_PN`, `_LID`) |
| `participants[]` | `participants` (`GroupParticipant`) |

| JSON participant | `GroupParticipant` |
|---|---|
| `jid` | `jid` (`Jid`) |
| `phone_number` (`null` when absent) | `phone_number` (`Jid`, unset when absent) |
| `lid` | `lid` (`Jid`) |
| `username` | `username` (`optional string`) |
| `type` (`"member"`, `"admin"`, `"superadmin"`) | `type` (`GROUP_PARTICIPANT_TYPE_MEMBER`, `_ADMIN`, `_SUPERADMIN`) |
| (none) | `details`: `join_time` (ms), `group_history_sent`, `participant_label`, `participant_label_mtime`, `display_name`, `is_addressable`; unset when the roster entry had none of them |

New on `GroupMetadata`, among others: the creator and the subject and
description owners, each with the phone jid and username beside the `@lid`;
`creation_time`, `subject_time` and `description_time` in milliseconds;
`is_locked`, `is_announcement`, `membership_approval`, `ephemeral`
(`expiration_seconds`, `trigger`), `member_add_mode`, `member_link_mode`,
`member_share_history_mode`, `size`; the community fields (`is_parent_group`,
`parent_group`, `is_default_sub_group`, `is_general_chat`, ...); and the
moderation fields (`is_suspended`, `appeal_status`, `growth_locked`, ...).
`appeal_update_time`, `participant_label_mtime` and `growth_locked.expiration`
are relayed as the server sent them: their unit is not verified.

**Membership requests.** `[{"jid":{"user":"..","server":"lid","agent":0,
"device":0,"integrator":0},"request_time":1790947901}]` becomes
`[MembershipRequest{jid: {value: "...@lid"}, request_time: 1790947901000}]`.
The jid is a `Jid` like every other one (this settles the third point of #96),
and the time is in milliseconds. The server also sends each requester's
`phone_number` and `request_method`; the library does not parse them, so they
are not relayed.

**Business profile, JSON key to field.** The names are the library's, so each
key is the field of the same name.

| JSON | `BusinessProfile` |
|---|---|
| `null` (no profile) | `profile` unset |
| `wid` (the jid struct) | `wid` (`Jid`) |
| `description` | `description` (`""` when the profile has none, the library's default) |
| `email`, `address` | `email`, `address` (`optional string`) |
| `website[]` | `website` (repeated) |
| `categories[]` `{id, name}` | `categories` (`BusinessCategory`) |
| `business_hours.timezone` | `business_hours.timezone` |
| `business_hours.business_config[]` | `business_hours.business_config` (`BusinessHoursConfig`) |
| `day_of_week` (`"sun"` ... `"sat"`) | `day_of_week` (`BUSINESS_DAY_OF_WEEK_SUNDAY` ... `_SATURDAY`), `day_of_week_raw` on `UNKNOWN` |
| `mode` (`"open_24h"`, `"specific_hours"`, `"appointment_only"`) | `mode` (`BUSINESS_HOUR_MODE_OPEN_24H`, `_SPECIFIC_HOURS`, `_APPOINTMENT_ONLY`), `mode_raw` on `UNKNOWN` |
| `open_time`, `close_time` | `open_time`, `close_time` (minutes past local midnight, unset when absent) |

**What the edge has to do:**

- Read the message instead of parsing JSON out of the bytes, and check presence
  where it checked for `null`.
- Treat an unset `subject` inside `GroupMetadata` as "no subject" (it was `""`).
  `GroupSummary.subject` is still `""` in that case.
- Read the times as milliseconds.
- Read a membership request's jid from `.jid.value`, not from a `user`/`server`
  object.

### 3b. Group and contact update events (#133)

| Message | Before | After |
|---|---|---|
| `GroupUpdate` | `string kind = 2` (always `"group_update"`), `bytes raw = 3` | both `reserved`; header fields 5 to 15 and `oneof action` (20 to 63) |
| `ContactUpdate` | `string kind = 2` (always `"contact_update"`), `bytes raw = 3` | both `reserved`; `timestamp = 5`, `action_timestamp = 6`, `from_full_sync = 7`, `ContactAction action = 8` |

The payload messages live in the new `group_update.proto`.

**Group update, JSON key to field.**

| JSON (`raw`) | `GroupUpdate` |
|---|---|
| `group_jid` (jid struct) | `group` (already a `Jid` since #122) |
| `notification_id`, `notify`, `offline` | same names, `optional string` |
| `action_index` | `action_index` |
| `participant`, `participant_pn` (jid structs) | `participant`, `participant_pn` (`Jid`, unset on a change the server made itself) |
| `participant_username`, `participant_country_code` | same names, `optional string` |
| `timestamp` (RFC 3339 string) | `timestamp` (ms) |
| `is_lid_addressing_mode`, `has_incomplete_participant_information` | same names |
| `action.type` (`"add"`, `"subject"`, ...) | the `oneof action` case of the same name |
| `action` fields | the fields of that case's message |

The action cases, by payload:

- **`GroupParticipantsChange { participants, reason }`:** `add`, `remove`
  (with `reason`), `promote`, `demote`, `modify`, `linked_group_promote`,
  `linked_group_demote`. Each participant is a `GroupNotificationParticipant`:
  `jid`, `phone_number`, `display_name`, `type` (`participant` is
  `GROUP_PARTICIPANT_TYPE_MEMBER`), `lid`, `username`, `join_time` (ms) and
  `group_history_sent_state`.
- **`Empty`:** `unlocked`, `announcement`, `not_announcement`,
  `no_frequently_forwarded`, `frequently_forwarded_ok`, `revoke`,
  `growth_unlocked`, `suspended`, `unsuspended`, `auto_add_disabled`,
  `is_capi_hosted_group`, `group_safety_check`, `allow_admin_reports`,
  `not_allow_admin_reports`, `reports`, `allow_non_admin_sub_group_creation`,
  `not_allow_non_admin_sub_group_creation`, `created_sub_group_suggestion`,
  `revoked_sub_group_suggestions`.
- **Their own message:** `subject` (`subject_time` in ms), `description`
  (unset `description` = deleted), `locked`, `ephemeral`
  (`expiration_seconds`; `not_ephemeral` is 0), `membership_approval_mode`,
  `membership_approval_request` and `created_membership_requests`
  (`MembershipRequestMethod`, `parent_group`), `revoked_membership_requests`,
  `member_add_mode` (`GroupMemberAddMode`, `mode_raw` on `UNKNOWN`), `invite`,
  `growth_locked` (`GrowthLockInfo`, expiration verbatim), `delete`, `link`,
  `unlink`, `limit_sharing_enabled`, `change_number`, `unknown` (`tag`).
- **`create`:** `GroupCreated { metadata }`, the `GroupMetadata` of section 3a.
  The JSON had no payload for it.

**Contact update, JSON key to field.**

| JSON (`raw`) | `ContactUpdate` |
|---|---|
| `jid` | `jid` (already a `Jid` since #122) |
| `timestamp` (RFC 3339) | `timestamp` (ms) |
| `action_timestamp` (RFC 3339 or `null`) | `action_timestamp` (ms, unset when the mutation had none) |
| `from_full_sync` | `from_full_sync` |
| `action.full_name`, `action.first_name`, `action.username` | the same names |
| `action.lid_jid`, `action.pn_jid` (strings) | `action.lid`, `action.pn` (`Jid`) |
| `action.save_on_primary_addressbook` | the same name |

**What the edge has to do:**

- Switch on the `oneof action` case where it read `action.type` from the JSON,
  and stop reading `kind`.
- Read jids as `.value` instead of rebuilding them from `user` and `server`.
- Read the times as milliseconds.

### 3c. App-state updates and the logout message (#134)

| Message | Before | After |
|---|---|---|
| `AppStateUpdate` | `AppStateKind kind = 5`, `bytes raw = 3` | both `reserved`; `timestamp = 6`, `action_timestamp = 7`, `from_full_sync = 8`, `oneof action` (10 to 15) |
| `LoggedOutInfo` | `reason`, `reason_code` | plus `LogoutMessage logout_message = 3` and `bool on_connect = 4` |

The `AppStateKind` enum is removed from the contract. Section 2a's
`kind` values map to the `oneof action` cases: `APP_STATE_KIND_ARCHIVE` is
`archive`, `PIN` is `pin`, `MUTE` is `mute`, `STAR` is `star`, `MARK_READ` is
`mark_read`, and `DELETE_CHAT` is `delete_chat`. The payload messages live in the new
`app_state.proto`.

**App-state update, JSON key to field.**

| JSON (`raw`) | `AppStateUpdate` |
|---|---|
| `jid` / `chat_jid` (star) | `chat` (already a `Jid` since #122) |
| `timestamp` (RFC 3339) | `timestamp` (ms) |
| `action_timestamp` (RFC 3339 or `null`) | `action_timestamp` (ms, unset when the mutation had none) |
| `from_full_sync` | `from_full_sync` |
| `action.archived` | `archive.archived` |
| `action.pinned` | `pin.pinned` |
| `action.muted`, `action.auto_muted` | `mute.muted`, `mute.auto_muted` |
| `action.mute_end_timestamp` (ms) | `mute.mute_end_timestamp` (ms, unchanged; unset on an unmute) |
| `action.mute_everyone_mention_end_timestamp` | `mute.mute_everyone_mention_end_timestamp` (verbatim, unit not verified) |
| `participant_jid`, `message_id`, `from_me`, `action.starred` (star) | `star.participant` (`Jid`), `star.message_id`, `star.from_me`, `star.starred` |
| `action.read` | `mark_read.read` (false is "mark as unread") |
| `delete_media` (delete) | `delete_chat.delete_media` |
| `action.message_range.last_message_timestamp`, `last_system_message_timestamp` (unix s) | `message_range.last_message_timestamp`, `last_system_message_timestamp` (ms) |
| `action.message_range.messages[].key` (`remote_jid`, `from_me`, `id`, `participant`) | `message_range.messages[].key` (`MessageKey`: `chat`, `from_me`, `id`, `participant`) |
| `action.message_range.messages[].timestamp` (unix s) | `message_range.messages[].timestamp` (ms) |

`message_range` sits on `archive`, `mark_read` and `delete_chat`.

**Logout.** `ConnectionStateChanged.logged_out` (section 2a) now also carries:

- `logout_message`: the server's own copy. It is set in practice on an
  account lock. Show it only when its `locale` matches the user's, as WA Web
  does.
- `on_connect`: true when the server refused the connection itself.

**What the edge has to do:**

- Switch on the `oneof action` case where it switched on `kind`.
- Read the message range's times as milliseconds. They were seconds.
- Read a range message's chat from `key.chat.value`.

### 3d. Call events (#135, closes #74)

| Message | Before | After |
|---|---|---|
| `CallEvent` | `CallActionKind action = 6`, `string action_raw = 7`, `bytes raw = 4` | all three `reserved`; `call_creator = 8`, `stanza_id = 9`, the stanza's fields (10 to 19), `oneof action` (20 to 34) |

The `CallActionKind` enum (section 2a) is removed from the contract. Its values
map to the `oneof action` cases: `OFFER` is `offer`, `OFFER_NOTICE` is
`offer_notice`, `PRE_ACCEPT` is `pre_accept`, `ACCEPT` is `accept`, `REJECT` is
`reject`, `TERMINATE` is `terminate`, `TRANSPORT` is `transport`,
`RELAY_LATENCY` is `relay_latency`, `VIDEO_STATE` is `video_state`,
`GROUP_UPDATE` is `group_update`, `ENC_REKEY` is `enc_rekey`,
`WAITING_ROOM_UPDATE` is `waiting_room_update`, `RAISE_HAND` is `raise_hand`
and `SCREEN_SHARE` is `screen_share`. `UNKNOWN` with `action_raw` is now
`unknown_action`, which carries the wire tag of an action the library added
after this core. The payload messages live in the new `call.proto`.

**Call event, JSON key to field.**

| JSON (`raw`) | `CallEvent` |
|---|---|
| `from` (the library's jid struct) | `from` (already a `Jid` since #122) |
| `stanza_id` | `stanza_id` |
| `action.call_id` | `call_id` (unchanged) |
| `action.call_creator` | `call_creator` (`Jid`) |
| `notify`, `platform`, `version`, `caller_username` | the same names, unset when absent |
| `participant`, `recipient` | `participant`, `recipient` (`Jid`, unset when absent) |
| `timestamp` (unix s) | `timestamp` (ms) |
| `offline` | `offline` |
| `video_orientation` | `video_orientation` |
| `group` | `group` (`GroupCallUpdate`) |
| `action.type` | the `oneof action` case |
| `action.caller_pn`, `group_jid` (offer) | `offer.caller_pn`, `offer.group_jid` (`Jid`) |
| `action.caller_country_code`, `device_class`, `joinable`, `is_video`, `audio[]` (offer) | `offer.*`, `audio[]` as `CallAudioCodec {enc, rate}` |
| `action.is_video`, `is_group` (offer_notice) | `offer_notice.*` |
| `action.audio[]` (preaccept, accept) | `pre_accept.audio[]`, `accept.audio[]` |
| `action.reason` (reject) | `reject.reason` |
| `action.reason`, `duration`, `audio_duration` (terminate) | `terminate.*`; the durations verbatim, unit not verified |
| `action.p2p_cand_round`, `transport_message_type` | `transport.*` |
| `action.state` (video, a number) | `video_state.state` (`CallVideoStateKind`), the number in `state_code` only on `UNKNOWN` |
| `action.orientation`, `dec` (video) | `video_state.orientation`, `video_state.dec` |
| `action.update` (group_update) | `group_update` (`GroupCallUpdate`) |
| `action.rekey` (enc_rekey) | `enc_rekey` (`GroupCallEncRekey`) |
| `action.room` (waiting_room_update) | `waiting_room_update` (`CallWaitingRoom`, `media` as `CallLinkMedia`) |
| `action.raised` (user_action) | `raise_hand.raised` |
| `action.screen_share` | `screen_share` (`state` as `ScreenShareState`, `version`, `screen_share_id`) |

Three values the library kept out of its JSON cross now, because the relay
does not pick: a group-call participant's phone number (`GroupCallParticipant.pn`),
a device's capability bitmask (`GroupCallDevice.capability`) and a rekey's
`ciphertext`.

`duration` and `audio_duration` cross as the server sent them: a linked device
is dismissed with `terminate reason="accepted_elsewhere"` as soon as another
device answers, so no capture carries one, and the library does not document
the unit.

**What the edge has to do:**

- Switch on the `oneof action` case where it switched on `action`.
- Read the stanza id from `stanza_id`, and the time as milliseconds. It was
  unix seconds inside the JSON.
- Read `call_creator` from the top level, for every action.

**The #74 rule from here on.** `scripts/check-wire-json.py` runs in `ci.sh`:
no JSON serializer in the daemon's sources except the `RawEvent` catch-all
(`raw_event_of` in `domain/event_mapping.rs`) and the store's own rows. A
library event wamux has not typed yet still arrives as `RawEvent` with a JSON
`payload` (#141 types the ones that matter).

### 3e. The raw logout and ban stanza (#138)

| Message | Before | After |
|---|---|---|
| `LoggedOutInfo` | `reason` to `on_connect` (1 to 4) | plus `StanzaNode stanza = 5` |
| `TemporaryBanInfo` | `reason` to `url` (1 to 5) | plus `StanzaNode stanza = 6` |

Additive: an edge that ignores the field keeps working. `StanzaNode` and
`StanzaNodeList` are new in `common.proto`.

- `stanza` is the whole stanza behind the logout or the ban, as the library
  received it: a `<failure>` when the server refused the connection
  (`on_connect` true), a `<stream:error>` for a later logout, where the reason
  is a child element such as `<conflict type="...">`. Unset for a local
  logout.
- An account lock puts a one-time `appeal_token`, `violation_reason` and `vt`
  on it, which no other field carries. The core does not parse them;
  `violation_reason` is not a closed set.
- `attrs` holds every attribute as text, a jid as its wire form. `content` is
  `bytes`, `text` or `children` (in order), unset when the element had none.
- Depth: a prost client decodes at most 100 nested messages, and each stanza
  level costs two. The core relays the stanza uncut; logout and ban stanzas
  are two or three levels deep.

### 3f. RawEvent kinds that became typed (#141)

Additive. Each library event below reached the socket as `RawEvent`, with
the library's JSON in `payload`. It now has a case of its own, so an edge
that matched `RawEvent.kind` on it stops seeing that kind.

| `RawEvent.kind` | Now | Issue |
|---|---|---|
| `LockChatUpdate` | `AppStateUpdate.lock` (`LockChange`) | #148 |
| `ClearChatUpdate` | `AppStateUpdate.clear_chat` (`ClearChatChange`) | #148 |
| `DeleteMessageForMeUpdate` | `AppStateUpdate.delete_message_for_me` (`DeleteMessageForMeChange`) | #148 |
| `UserStatusMuteUpdate` | `AppStateUpdate.user_status_mute` (`UserStatusMuteChange`) | #148 |
| `FavoriteStickerUpdate` | `StickerUpdate.favorite` (`FavoriteStickerChange`), `EventEnvelope.sticker = 28` | #148 |
| `RemoveRecentStickerUpdate` | `StickerUpdate.remove_recent` (`RemoveRecentStickerChange`) | #148 |
| `LabelEditUpdate` | `LabelUpdate.edit` (`LabelEdit`), `EventEnvelope.label = 29` | #149 |
| `LabelAssociationUpdate` | `LabelUpdate.chat` (`LabelChatAssociation`) | #149 |
| `MessageLabelAssociationUpdate` | `LabelUpdate.message` (`LabelMessageAssociation`) | #149 |
| `QuickReplyUpdate` | `QuickReplyUpdate`, `EventEnvelope.quick_reply = 30` | #149 |
| `DisableLinkPreviewsUpdate` | `AccountSettingUpdate.link_previews` (`LinkPreviewsSetting`), `EventEnvelope.account_setting = 31` | #149 |
| `StatusPrivacyUpdate` | `AccountSettingUpdate.status_privacy` (`StatusPrivacySetting`) | #149 |
| `SelfPushNameUpdated` | `SelfPushNameUpdate`, `EventEnvelope.self_push_name = 32` | #153 |
| `ContactRemoved` | `ContactUpdate` with `removed` true and no `action` | #153 |
| `CallLogSync` | `CallLogUpdate`, `EventEnvelope.call_log = 33` | #153 |

**JSON key to field (#148).**

| JSON (`payload`) | Field |
|---|---|
| `jid` / `chat_jid` | `AppStateUpdate.chat` |
| `timestamp`, `action_timestamp` (RFC 3339) | `timestamp`, `action_timestamp` (ms) |
| `action.locked` (lock) | `lock.locked` |
| `delete_starred`, `delete_media`, `action.message_range` (clear) | `clear_chat.*`, the range in ms (it was seconds) |
| `participant_jid`, `message_id`, `from_me` (delete for me) | `delete_message_for_me.participant` (`Jid`), `message_id`, `from_me` |
| `action.delete_media`, `action.message_timestamp` (unix s) | `delete_message_for_me.delete_media`, `message_timestamp` (ms) |
| `action.muted` (status mute; the top-level `muted` read an absent flag as false) | `user_status_mute.muted`, unset when absent |
| `filehash` (stickers) | `StickerUpdate.filehash` |
| `action.direct_path`, `media_key`, `file_enc_sha256`, `file_length`, `mimetype` | `favorite.media` (`MediaDescriptor`, `media_type` STICKER, `file_sha256` = `filehash` decoded), unset without a path |
| `action.is_favorite`, `url`, `width`, `height`, `is_lottie`, `is_avatar_sticker`, `image_hash`, `device_id_hint` | `favorite.*` |
| `action.last_sticker_sent_ts` (ms) | `remove_recent.last_sticker_sent_ts` (ms, unchanged) |

`favorite.media` goes to DownloadMedia as is. Its `file_sha256` is the
`filehash` decoded, because the library checks the decrypted bytes against
it: with it empty the download is refused (measured live). It is empty only
when the filehash is not a base64 SHA-256.

On the user's phone, hiding a contact from the status list is what sends
`user_status_mute` (`muted` true); showing it again sent nothing in two tries.

**JSON key to field (#149).**

| JSON (`payload`) | Field |
|---|---|
| `label_id`, `timestamp` (RFC 3339), `from_full_sync` (labels) | `LabelUpdate.label_id`, `timestamp` (ms), `from_full_sync` |
| `action.name`, `color`, `predefined_id`, `deleted`, `order_index`, `is_active`, `is_immutable`, `mute_end_time_ms` | `edit.*`, as sent |
| `action.type` (a name, e.g. `"CUSTOM"`) | `edit.list_type` (`LabelListType`); absent or unknown to the library is UNSPECIFIED |
| `chat_jid`, `action.labeled`, `action.model_meta_data` (chat label) | `chat.chat` (`Jid`), `chat.labeled`, `chat.model_meta_data` |
| `chat_jid`, `message_id`, `action.*` (message label) | `message.chat`, `message.message_id`, `message.labeled`, `message.model_meta_data` |
| `id`, `action.shortcut`, `message`, `keywords`, `count`, `deleted`, `associated_label_ids` (quick reply) | the same names on `QuickReplyUpdate` |
| `action.is_previews_disabled` (the top-level `previews_disabled` is derived) | `link_previews.previews_disabled` |
| `action_timestamp` (status privacy) | `AccountSettingUpdate.action_timestamp` (ms) |
| `action.mode`, `action.modes` | `status_privacy.mode`, `modes` (`StatusAudience`: the case, and `mode_code` on UNKNOWN) |
| `action.user_jid`, `custom_lists[].user_jid` (strings) | `status_privacy.users`, `custom_lists[].users` (`Jid`) |
| `action.share_to_fb`, `share_to_ig`, `custom_lists[]` | the same names on `StatusPrivacySetting` |

Measured on 2026-10-08: the consumer app's chat lists are labels of type
CUSTOM, and deleting one sends `deleted` true with an empty name and
`is_active` false. "My contacts except..." sends the status audience as
`mode` DENY_LIST with the excluded users and `modes` [DENY_LIST]; going back
sends CONTACTS with no users. The app no longer labels messages, so the
message label is covered by tests only.

**JSON key to field (#153).**

| JSON (`payload`) | Field |
|---|---|
| `old_name`, `new_name` (own push name) | `SelfPushNameUpdate.old_name`, `new_name` |
| `from_server` | dropped: always true |
| `jid`, `timestamp`, `action_timestamp`, `from_full_sync` (contact removed) | the same names on `ContactUpdate` (times in ms), with `removed` true |
| `call_creator_jid`, `call_id`, `from_me`, `timestamp`, `from_full_sync` (call log) | `CallLogUpdate.call_creator` (`Jid`), `call_id`, `from_me`, `timestamp` (ms), `from_full_sync` |
| `record.call_result`, `silence_reason`, `call_type` (names) | `result` (`CallLogResult`), `silence_reason` (`CallLogSilenceReason`), `call_type` (`CallLogType`); absent is UNSPECIFIED |
| `record.duration` | `duration_seconds`, as sent |
| `record.start_time` (seconds) | `start_time` (ms) |
| `record.is_dnd_mode`, `is_video`, `is_call_link`, `call_link_token`, `scheduled_call_id` | the same names on `CallLogUpdate` |
| `record.group_jid` (string) | `group` (`Jid`) |
| `record.participants[].user_jid`, `call_result` | `participants[].user` (`Jid`; an entry with none is skipped), `result` |
| `record.is_incoming`, `record.call_id`, `record.call_creator_jid` | dropped: read `from_me`, `call_id` and `call_creator` |

Measured on 2026-10-08 with a call placed from one paired account to the
other: an answered call of 30 to 31 s on the phones carried `duration` 31 and
a `start_time` in seconds, equal to the second the offer arrived; a
rejected one carried REJECTED and `duration` 0. Only the companion of the
caller received the call log. A removed contact carried its
`action_timestamp` (equal to `timestamp`).
