# Breaking change for the edge: 2026-09-02 (issue #20)

A single change, and it swaps what an existing RPC **does**, not its shape.
The edge compiles the same and the behavior changes: it is the worst kind of
change to go unnoticed, so it is here on its own.

Changed file: `proto/messaging.proto`.

| # | Change | Impact on the edge |
|---|--------|--------------------|
| 1 | `MarkRead` now sends a read receipt instead of syncing state | **BREAKING** (same signature, different effect) |
| 2 | `MarkChatRead` is new and does what `MarkRead` used to do | Additive |

---

## 1. `MarkRead` now sends the receipt (BREAKING)

**Before:** `MarkRead` called `chat_actions().mark_chat_as_read()`, which is an
app-state mutation, a sibling of archive and pin. It marks the conversation as
read **on the account's own devices** and puts no receipt on the network. The
request's `message_ids` and `sender` fields were read and discarded.

The result: the other person's ticks never turned blue, and nothing said so.
The RPC returns `Empty` in both cases.

**Now:** `MarkRead` calls `client.mark_as_read(chat, sender, message_ids)`, which
emits the receipt stanza. All three fields are now used.

```proto
message MarkReadRequest {
  AccountRef account = 1;
  Jid chat = 2;
  repeated string message_ids = 3;  // now read
  Jid sender = 4;                   // now read
}
```

**What the edge needs to do:**

- If you called `MarkRead` **to send a receipt**: nothing. It now works.
  Check that you are filling in `message_ids` and, in a group, `sender`.
- If you called `MarkRead` **for the effect on your own devices**: switch to
  `MarkChatRead`, which preserves the old behavior exactly.
- If you want both, as the official app does: call both. The core does not
  compose the two operations for you, on purpose: joining them would remove
  the possibility of asking for one without the other.

**An empty `message_ids` is not an error.** The library returns before building
the stanza, so a call with nothing to acknowledge costs nothing and does not
fail.

**An empty `sender` means absent, not an error.** In a DM the author is the
conversation itself and the field does not apply. Sending `Jid { value: "" }`
or omitting the field amount to the same thing.

## 2. `MarkChatRead` (additive)

```proto
rpc MarkChatRead(MarkReadRequest) returns (Empty);
```

Marks the conversation read on the account's own devices. It reads only
`account` and `chat`: the read state of a conversation is per conversation,
not per message. It is the `MarkRead` from before this change, with a name
that describes what it does, next to the `MarkUnread` that already existed
for the opposite direction.

## Two things the core can NOT tell you

Both are for upstream to expose, and they matter because both look like the
bug this change fixes:

1. **Which receipt went out.** When the account has "read receipts" turned off
   in its privacy settings, WhatsApp sends `read-self` instead of `read`,
   and **only in a private DM**; reading in a group goes out as `read` either
   way. The library reads that flag from a device snapshot that it does not
   expose (`persistence_manager` is `pub(crate)` and there is no getter), so
   the core has no way to report which of the two it sent.

2. **Whether the receipt arrived.** There is no confirmation coming back.
   Silence here proves nothing, in either direction.

What works as verification is a second device: open the conversation, call
`MarkRead` with the ids of the messages the other person sent, and ask them
whether the ticks turned blue. **A watermark on the caller's side is not
proof**: that is exactly what hid the bug. The `remote_read_ms` column of
`wamux-omarchy` advanced on every call, because it is written when the relay
accepts, which says the call happened and not that a receipt went out.
