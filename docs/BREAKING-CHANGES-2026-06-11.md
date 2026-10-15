# Breaking changes for the edge (wamux-http-edge): 2026-06-11

Consolidation of ALL the core's contract changes from this code-review round
(critical fixes + a contract batch). Everything lands together in today's
working tree; the edge should treat this document as the complete delta
against the contract at the end of Sprint 5.

Changed .proto files: `proto/events.proto` and `proto/account.proto`
(full text in the final section). The others (`messaging.proto`, `common.proto`,
`media.proto`, `groups.proto`, `contacts.proto`, `admin.proto`) did not change
in this round.

Summary by impact:

| # | Change | Impact on the edge |
|---|--------|--------------------|
| 1 | `ReceiptEvent.type` is now lowercase tokens | **BREAKING** (string values) |
| 2 | `PresenceUpdate.chat_state` in lowercase + new value `recording` | **BREAKING** (string values) |
| 3 | `PairedInfo`: `push_name` removed; `business_name`, `lid`, `platform` added | **BREAKING** (codegen + semantics) |
| 4 | The creation channel's gap became `kind="subscription_gap"` | **BREAKING** (gap dispatch) |
| 5 | Replay without duplicates + full-ring replay for accounts created after subscribe | Behavioral (in the edge's favor) |
| 6 | `SendMediaHeader.quote`/`mentions` are now honored | Behavioral (fields that used to be ignored now take effect) |
| 7 | IQ errors: structured `wa-code`/`wa-text` trailers | Additive (migrate off prose parsing) |
| 8 | WhatsApp wire: no empty ContextInfo; quote participant in a DM is absent instead of `""` | Invisible in the socket API |

---

## 1. `ReceiptEvent.type`: lowercase tokens (BREAKING)

**Before:** the core emitted Rust's Debug output: `"Read"`, `"Delivered"`,
`"Played"`, and even `"Other(\"...\")"` for unknown types. The proto comment
promised `delivered|read|played` and it was never true.

**Now:** stable lowercase tokens, exactly the ones documented in the proto:

```
delivered | read | played | read-self | played-self | sender | retry |
enc_rekey_retry | server-error | inactive | peer_msg | hist_sync
```

An unknown type relays the raw stanza attribute verbatim (no `Other(...)`
wrapper).

**Migration:** change any match/comparison of `"Read"`/`"Delivered"` to the
lowercase tokens. If the edge already normalized with lowercase, check the
hyphenated ones (`read-self`, `server-error`), which used to arrive as
`ReadSelf`/
`ServerError`.

## 2. `PresenceUpdate.chat_state`: lowercase + `recording` (BREAKING)

**Before:** `"Composing"` or `"Paused"` (Debug). "Recording audio" was
indistinguishable from "typing" (the lib models recording as Composing with
media=Audio, and Debug ignored the media).

**Now:** `composing` | `recording` | `paused` | `""` (the empty string still
means "presence without chat state", coming from an online/offline
PresenceUpdate).

Symmetry bonus: these are the same tokens that `SendPresenceRequest.state`
accepts, so the edge can mirror the observed value back without translating.

**Migration:** change `"Composing"` → `"composing"`, `"Paused"` → `"paused"`,
and handle the new `"recording"` (it used to arrive as `"Composing"`).

## 3. `PairedInfo` reworked (BREAKING)

**Before:**
```proto
message PairedInfo {
  Jid jid = 1;
  string push_name = 2;   // in practice it carried the business_name!
}
```

**Now:**
```proto
message PairedInfo {
  Jid jid = 1;
  reserved 2; reserved "push_name";
  string business_name = 3;   // empty for personal accounts
  Jid lid = 4;                // the account's LID (hidden-PN) identity
  string platform = 5;        // e.g.: "android", "smba", "iphone"
}
```

**Why:** the lib's PairSuccess event carries no push name at all; the old
field delivered the business name under a misleading name (and an empty string
for every personal account). Real push names arrive later, via `PushNameUpdate`
events (and `Account.push_name`/`AccountStatus.push_name`, which did not
change).

**Migration:** regenerate the client from the protos; any read of
`paired.push_name` breaks at compile/codegen time (intentional). Use
`business_name` if what you wanted was the business name; for the push name,
consume `PushNameUpdate`. `lid` and `platform` are new information that used
to be discarded.

## 4. Creation channel gap: `kind="subscription_gap"` (BREAKING)

**Before:** both kinds of gap used `RawEvent.kind="gap"`, distinguishable only
by the empty `account_uuid` and by the note text. An edge that followed the
documented contract ("gap → resync via GetAccountStatus") would call
`GetAccountStatus("")` and get InvalidArgument.

**Now:** two reserved kinds, documented in the proto (`RawEvent`):

- `kind="gap"`: ONE account's subscriber lagged the buffer; `account_uuid`
  is set; recovery = `GetAccountStatus` for that account.
- `kind="subscription_gap"`: the created-accounts channel of the all-accounts
  stream lagged (burst of >64 creates); `account_uuid` is empty; recovery =
  re-subscribe or reconcile via `ListAccounts`.

Both keep `monotonic_seq = -1` and an empty payload. The note still exists,
but it is prose for humans, not contract.

**Migration:** dispatch gaps by `kind`, never by note or by empty uuid.
`subscription_gap` has only existed since Sprint 5; if the edge does not yet
handle the all-accounts gap, just implement it directly on the new kind.

## 5. Replay semantics (behavioral, in the edge's favor)

Two new guarantees, documented in `SubscribeRequest.replay_from_ring`:

- **No duplicates within a stream.** Before, an event published in the window
  between the internal subscribe and the ring snapshot arrived twice
  (replay + live) with the same `monotonic_seq`. Now the core filters the
  overlap. An edge that already deduplicated by `(account_uuid, monotonic_seq)`
  stays correct and simply never sees a duplicate again.
- **An account created AFTER the all-accounts subscribe always replays the
  whole ring, even with `replay_from_ring=0`.** "Live only" does not apply to
  events emitted before the forwarder attached; without this, the pairing QR
  of a freshly created account could be lost. Visible effect: a live-only
  stream now receives the first events of new accounts, events that used to
  vanish.
  (Config exception: `ring_capacity=0` turns the ring off and the window
  exists again.)

**Migration:** none required. If the edge had a workaround of
"re-subscribe with replay>=1 after CreateAccount", it can remove it.

## 6. `SendMediaHeader.quote` and `mentions` now take effect (behavioral)

**Before:** fields 7/8 of `SendMediaHeader` existed in the proto but were
silently dropped: media went out with no reply bubble and no mention,
with SendResult OK.

**Now:** quote + mentions + ephemeral compose into a single ContextInfo, with
the same semantics as the text path (including the precedence
`QuoteContext.participant` > `quoted.participant` and the mapping of an empty
string to an absent field).

**Migration:** no call changes. But note: if the edge was already sending
these fields thinking they worked, they now really work; if it filled them
with junk "because it made no difference", review that.

## 7. IQ errors: structured `wa-code`/`wa-text` metadata (additive)

Every IQ rejection from the WhatsApp server (`WaServer`) now carries gRPC
trailers in addition to the Status:

- `wa-code`: the raw IQ code (e.g.: `403`, `409`), always present.
- `wa-text`: the stanza text (e.g.: `forbidden`), present when it is valid
  ASCII.

The code→Status mapping did not change (400→InvalidArgument,
401→Unauthenticated, 403→PermissionDenied, 404→NotFound,
429→ResourceExhausted, everything else→Unavailable), and the prose message
(`whatsapp server rejected the request: code=..., text='...'`) stays the same
for compatibility.

**Recommended migration:** replace any regex over the message
(`code=(\d+)`) with reading the trailers. The prose is now considered free to
change; the trailers are the contract. In particular, unmapped codes
(405/406/409/500) that collapse into Unavailable can now be told apart from a
dropped connection by `wa-code` (a generic connection error carries no
trailer at all).

## 8. WhatsApp wire (no impact on the socket API)

Changes in what the core puts on the wire to WhatsApp, not in gRPC:

- Text with only a link_preview no longer emits a present-but-empty
  ContextInfo (a non-canonical shape that no official client produces).
- A quote in a DM (empty participants) emits the participant field ABSENT
  instead of `Some("")` (an invalid empty JID).
- Receipts/presence/pairing: nothing changed on the wire, only in the gRPC
  representation (items 1 to 3).

## Internal changes without a contract (FYI)

- Event forwarders now terminate when the client disconnects
  (`tx.closed()`), eliminating the leak of tasks/handles of idle or deleted
  accounts in long streams.
- A single internal `MediaKind` derives the upload and the sub-message
  (mislabeling media through drifting matches is impossible).
- `scripts/ci.sh`: accepts DATABASE_URL without an explicit port and fails if a
  filtered gate runs 0 tests.
- Deferred on purpose: capacity 64 of the created_tx channel (see
  `docs/DEFERRED.md`).

---

## proto/events.proto (new, full text)

```proto
syntax = "proto3";

package wamux.v1;

import "common.proto";
import "account.proto";

message SubscribeRequest {
  oneof selector {
    AccountRef account = 1;
    // "all" is DYNAMIC: it includes accounts created after subscribing, so the
    // stream stays open indefinitely. Under an extreme burst of creates the
    // core may emit a `subscription_gap` RawEvent meaning some newly created
    // accounts are missing from the stream: re-subscribe or reconcile via
    // ListAccounts.
    Empty all_accounts = 2;
  }
  // 0 = live only; N = replay up to N buffered events for quick reconnect.
  // No duplicates within one stream: live events already delivered by the
  // replay are skipped (filtered by monotonic_seq). Accounts created AFTER an
  // all-accounts subscribe always replay their full ring regardless of N —
  // "live only" cannot apply to events emitted before their forwarder
  // attached, so e.g. a just-created account's pairing QR is never lost.
  uint32 replay_from_ring = 3;
}

message InboundMessage {
  MessageKey key = 1;
  string chat = 2;
  string sender = 3;
  int64 timestamp = 4;
  string push_name = 5;
  string text = 6;
  QuoteContext quote = 7;
  repeated Mention mentions = 8;
  MediaDescriptor media = 9;        // set for media messages
  string caption = 10;
  string reaction = 11;             // set for reaction messages
  MessageKey reaction_target = 12;
  bool is_edit = 13;
  bool is_delete = 14;
  bytes raw_message = 20;           // serialized wa.Message for advanced consumers
}

message ReceiptEvent {
  string chat = 1;
  string sender = 2;
  repeated string message_ids = 3;
  // Lowercase tokens: delivered|read|played plus the rarer read-self|
  // played-self|sender|retry|enc_rekey_retry|server-error|inactive|peer_msg|
  // hist_sync; an unknown type relays the raw stanza attribute verbatim.
  string type = 4;
  int64 timestamp = 5;
}

message UndecryptableEvent {
  string chat = 1;
  string sender = 2;
  string reason = 3;
}

message ConnectionStateChanged {
  ConnectionState state = 1;
  string detail = 2;
}

message PresenceUpdate {
  string jid = 1;
  bool online = 2;
  int64 last_seen = 3;
  string chat_state = 4;            // composing|recording|paused|""
}

message GroupUpdate {
  string group_jid = 1;
  string kind = 2;
  bytes raw = 3;
}

message PushNameUpdate {
  string jid = 1;
  string push_name = 2;
}

message ContactUpdate {
  string jid = 1;
  string kind = 2;
  bytes raw = 3;
}

// History-sync chunk, only emitted when the account connected with backfill
// enabled (ConnectAccountRequest.backfill_history) or in answer to
// FetchMessageHistory. The core relays the blob verbatim: the edge decodes
// `raw` (a `wa.HistorySync` protobuf) itself. Delivered in chunks.
message HistorySyncEvent {
  int32 sync_type = 1;            // wa.HistorySync.HistorySyncType (InitialBootstrap|Recent|PushName|OnDemand|...)
  optional uint32 chunk_order = 2;
  optional uint32 progress = 3;   // 0-100
  // Set for on-demand syncs: correlate with FetchMessageHistoryResponse.session_id.
  optional string session_id = 4;
  bytes raw = 20;                 // decompressed `wa.HistorySync` protobuf
}

// Catch-all so the edge never silently loses an event type or a broadcast gap.
// Reserved kinds (both carry monotonic_seq -1 and an empty payload):
//   "gap"              one account's subscriber lagged its broadcast buffer;
//                      account_uuid is set: resync it via GetAccountStatus.
//   "subscription_gap" the all-accounts created-channel lagged; account_uuid
//                      is empty: re-subscribe or reconcile via ListAccounts.
// Any other kind is a forward-compat relay of a lib event (payload = JSON).
message RawEvent {
  string kind = 1;
  bytes payload = 2;
  string note = 3;
}

message EventEnvelope {
  string account_uuid = 1;
  int64 monotonic_seq = 2;
  int64 ts_unix_ms = 3;
  oneof event {
    InboundMessage message = 10;
    ReceiptEvent receipt = 11;
    UndecryptableEvent undecryptable = 12;
    ConnectionStateChanged connection = 13;
    PairingUpdate pairing = 14;
    PresenceUpdate presence = 15;
    GroupUpdate group = 16;
    PushNameUpdate push_name = 17;
    ContactUpdate contact = 18;
    HistorySyncEvent history_sync = 19;
    RawEvent raw = 99;
  }
}

service EventService {
  rpc SubscribeEvents(SubscribeRequest) returns (stream EventEnvelope);
}
```

## proto/account.proto (new, full text)

```proto
syntax = "proto3";

package wamux.v1;

import "common.proto";

message CreateAccountRequest {
  optional string external_ref = 1;
  reserved 2; // was ConnectionPolicy policy (removed: connect is edge-driven)
  reserved "policy";
}

message Account {
  string uuid = 1;
  string external_ref = 2;
  reserved 3; // was ConnectionPolicy policy (removed: connect is edge-driven)
  reserved "policy";
  ConnectionState state = 4;
  Jid jid = 5;
  string push_name = 6;
}

message ListAccountsRequest {}

message ListAccountsResponse {
  repeated Account accounts = 1;
}

message AccountStatus {
  string uuid = 1;
  ConnectionState state = 2;
  Jid jid = 3;
  string push_name = 4;
  int64 paired_at = 5;
}

message PairWithQrRequest {
  AccountRef account = 1;
  // true = connect with history backfill ON so the InitialBootstrap dump the
  // phone pushes right after linking is emitted as HistorySyncEvent(s).
  bool backfill_history = 2;
}

message PairWithCodeRequest {
  AccountRef account = 1;
  string phone_number = 2;
  optional string custom_code = 3;
  bool backfill_history = 4; // see PairWithQrRequest.backfill_history
}

message ConnectAccountRequest {
  AccountRef account = 1;
  // false (default) = skip the phone's history dump (relay-pure default).
  // true = process and emit it as HistorySyncEvent(s) for backfill (e.g. a CRM).
  bool backfill_history = 2;
}

message PairedInfo {
  Jid jid = 1;
  // Removed: the lib's PairSuccess carries no push name at pair time; the old
  // field relayed the business name under a misleading name. Use
  // business_name, and take push names from PushNameUpdate events.
  reserved 2;
  reserved "push_name";
  string business_name = 3;         // empty for personal (non-business) accounts
  Jid lid = 4;                      // the account's LID (hidden-PN) identity
  string platform = 5;              // e.g. "android", "smba", "iphone"
}

message PairingError {
  string message = 1;
}

// Streamed during pairing: QR refreshes, the pair code, then success or error.
message PairingUpdate {
  oneof event {
    string qr_code = 1;
    string pair_code = 2;
    PairedInfo paired = 3;
    PairingError error = 4;
  }
}

service AccountService {
  rpc CreateAccount(CreateAccountRequest) returns (Account);
  rpc ListAccounts(ListAccountsRequest) returns (ListAccountsResponse);
  rpc GetAccountStatus(AccountRef) returns (AccountStatus);
  rpc PairWithQr(PairWithQrRequest) returns (stream PairingUpdate);
  rpc PairWithCode(PairWithCodeRequest) returns (stream PairingUpdate);
  rpc ConnectAccount(ConnectAccountRequest) returns (AccountStatus);
  rpc DisconnectAccount(AccountRef) returns (AccountStatus);
  rpc Logout(AccountRef) returns (Empty);
  rpc DeleteAccount(AccountRef) returns (Empty);
}
```

## Edge migration checklist

1. Copy the new `events.proto` and `account.proto` (or point at the core's
   `proto/`) and regenerate the gRPC client.
2. Fix whatever codegen breaks: `PairedInfo.push_name` no longer exists.
3. Search for and update string comparisons: `"Read"`, `"Delivered"`,
   `"Played"`, `"Composing"`, `"Paused"` → lowercase tokens; add
   `"recording"`.
4. Dispatch gaps by `kind` (`gap` vs `subscription_gap`).
5. Replace the `code=(\d+)` parse of the error message with the
   `wa-code`/`wa-text` trailers.
6. Remove replay workarounds (dedupe by seq becomes a no-op; re-subscribing
   after CreateAccount becomes unnecessary).
7. Review media sends that fill `quote`/`mentions`: they now take effect.
