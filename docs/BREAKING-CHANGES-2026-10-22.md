# Breaking changes for the edge: polls published to a channel (#220, 2026-10-22)

No field changes in the `.proto`. A `SendPoll` whose `to` is a channel
(`@newsletter`) is now published by the library's `Newsletter::send_poll`, which
follows what WA Web sends to a channel. A poll to a chat or a group is
unchanged.

| # | Change | Impact |
|---|--------|--------|
| 1 | `selectable_count` for a channel is 0 or 1; above 1 is refused | **BREAKING** (2..N used to be published) |
| 2 | `SendPollResult.message_secret` is empty for a channel | **BREAKING** (it used to be 32 bytes) |
| 3 | A blank question or option is refused for a channel | New `InvalidArgument` |

## 1. `selectable_count` for a channel

**Before:** a channel took the same count as a chat, `1..=options.len()`. 1 was
published as a single-answer poll (v3), and anything above as a several-answer
poll (v1) carrying the number. 0 was refused.

**Now:** a channel takes 1 for a single answer (v3) or 0 for several answers
(v1, `selectableOptionsCount` 0), as WA Web publishes them. Anything above 1 is
`InvalidArgument("invalid poll: channel polls require selectable_count 0
(multiple answers) or 1 (single answer)")`, and nothing is sent. The core relays
the number as given and does not rewrite it.

**To migrate:** for a channel poll with several answers, send
`selectable_count = 0`. A chat still takes `1..=options.len()` and still refuses
0.

## 2. No message secret for a channel poll

**Before:** a channel poll carried a random 32-byte `messageSecret`, and the RPC
handed it back.

**Now:** a channel poll is plaintext and its votes are not encrypted, so nothing
generates or sends a secret, and `message_secret` is empty. Voting in a channel
poll (`SendNewsletterPollVote`) never used it, and does not change.

**To migrate:** do not store or require a secret for a channel poll. For a chat
poll the field is still the key to open its votes.

## 3. Blank text is refused for a channel

A question or an option that is empty or only whitespace is now
`InvalidArgument("invalid poll: channel poll questions and options cannot be
blank")` for a channel, with nothing sent. The other refusals keep their
messages: fewer than 2 or more than 12 options, and duplicate options.
