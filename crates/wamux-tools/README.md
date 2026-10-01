# wamux-tools

Development and validation binaries for wamux: pairing, end-to-end checks and live probes. They are
not shipped and not part of the daemon. Every one reaches the daemon through the shared library
(`wamux_tools`), reads its configuration from the environment, prints one line per check and exits
with a real exit code.

## Environment contract

Every variable is validated before a socket or a database is opened, so a missing one fails at once
and names itself on stderr.

| Variable | Meaning |
|---|---|
| `WAMUX_SOCKET_PATH` | The daemon's Unix socket. Default `$HOME/.local/state/wamux/wamux.sock`. |
| `WAMUX_REF` | The `external_ref` of the account the binary acts as. Required, no default (not for `bench_client`, which connects every account). |
| `WAMUX_PEER_REF` | A second account. Required by `chat_state_live`, optional for `send_echo_live`. |
| `WAMUX_LIVE_DEST` | The one chat a binary may write to. Required by every binary that sends to someone else. Spelled `<digits>@s.whatsapp.net` (or `@lid`, `@g.us`); the legacy server spelling is refused, and so is the connected account's own number. |
| `WAMUX_DELIVERY_SECS` | How long a send waits for its `delivered` receipt. Default 30. |
| `DATABASE_URL` | Postgres for the in-process binaries (`pair_cli`, `pair_backfill`, `stress_live`). Default the dev database on port 5433. |

Positional arguments left on a binary are only bin-specific ones (seconds, counts, a chat, a kind).

## What a result means

Each check is printed as it is recorded, then a `summary: N pass, M accepted, K fail` line.

- `[PASS]`: a returned value was asserted. For a send to someone else that means the fan-out reached
  the recipient's phone AND a `delivered` (or later `read` / `played`) receipt for the message id
  arrived within `WAMUX_DELIVERY_SECS`. The server ack alone never counts. A reaction, an edit and a
  revoke pass on the fan-out reaching the phone (their receipts are not awaited).
- `[ACCEPTED]`: an RPC that answers with nothing (Empty), or a note to self, answered Ok. A
  self-chat has no receipt and no fan-out, so it can only ever be accepted.
- `[FAIL]`: anything else.

The exit code is zero only with no `fail` and at least one `pass`: a run of only accepted calls, or
of nothing, proved nothing and exits non-zero.

## Binaries

### backfill

- **Needs:** the daemon, with `WAMUX_REF` paired.
- **Env:** `WAMUX_REF`, `WAMUX_SOCKET_PATH`. Arguments: `[chat_jid] [count] [watch_secs]`.
- **Proves:** that FetchMessageHistory returns a session id and that a HistorySyncEvent answering that session arrives.
- **Writes to WhatsApp:** nothing visible; it asks the phone for older messages of a chat.

### bench_client

- **Needs:** the daemon, with accounts to connect.
- **Env:** `WAMUX_SOCKET_PATH`. No `WAMUX_REF`: it connects every account.
- **Proves:** nothing; it is a benchmark that holds an all-accounts subscription for the wamux-vs-wacli memory comparison, runs until Ctrl+C and keeps its own tally. It exits non-zero if the stream errors or the daemon closes it.
- **Writes to WhatsApp:** nothing (it connects the accounts).

### chat_state_live

- **Needs:** the daemon, with two paired accounts on different numbers.
- **Env:** `WAMUX_REF` (types), `WAMUX_PEER_REF` (watches), `WAMUX_LIVE_GROUP` (optional existing group), `WAMUX_SOCKET_PATH`. Argument: `[seconds]`.
- **Proves:** issue #24: a chat state names the conversation it happened in (group vs direct differ in `chat`) and carries no `online` value.
- **Writes to WhatsApp:** a group between your own two accounts (unless `WAMUX_LIVE_GROUP` is set) and three typing indicators. No message.

### e2e_all

- **Needs:** the daemon, `WAMUX_REF` paired and connected.
- **Env:** `WAMUX_REF`, `WAMUX_LIVE_DEST`, `WAMUX_DELIVERY_SECS`, `WAMUX_E2E_INBOUND=1` (opt in to the 60 second window where the person at `WAMUX_LIVE_DEST` sends the account a text and an image, then DownloadMedia; only direct messages from `WAMUX_LIVE_DEST` count, other chats are ignored), `WAMUX_SOCKET_PATH`.
- **Proves:** the non-destructive API surface: metrics, the create/list/status/delete lifecycle of a throwaway account, contact and group reads, and a text and an image that reach the destination and are delivered. A failed connect ends the run before anything is sent.
- **Writes to WhatsApp:** to `WAMUX_LIVE_DEST` a text, a reaction, an edit, an image, a presence and a revoke of the text. The `e2e-tmp-*` account lives only in the daemon's store.

### e2e_destructive

- **Needs:** the daemon, `WAMUX_REF` paired. Refuses to run unless `WAMUX_E2E_DESTRUCTIVE` is exactly `yes`.
- **Env:** `WAMUX_REF`, `WAMUX_LIVE_DEST` (the group member), `WAMUX_E2E_DESTRUCTIVE=yes`, `WAMUX_SOCKET_PATH`.
- **Proves:** push name and profile picture can be set and restored (read back), and a new group can be created with the destination, renamed, described, given and revoked an invite link, have its member promoted, demoted and removed, and be left. Logout and DeleteAccount are not here: see `logout_e2e`.
- **Writes to WhatsApp:** it changes the account's push name and profile picture (then restores them) and creates a group that includes `WAMUX_LIVE_DEST`.

### interactive_reply_live

- **Needs:** the daemon, `WAMUX_REF` paired, and a business bot that sends buttons, lists or templates.
- **Env:** `WAMUX_REF`, `WAMUX_REPLY` (the choice to pick; read-only when unset), `WAMUX_LIVE_DEST` (required only when `WAMUX_REPLY` is set), `WAMUX_DELIVERY_SECS`, `WAMUX_SOCKET_PATH`. Argument: `[seconds]`.
- **Proves:** issue #28: an offer is decoded and, when a reply is requested, that the reply reached the bot's phone and was delivered. The bot's next message is the human proof.
- **Writes to WhatsApp:** with `WAMUX_REPLY` set, one interactive reply to `WAMUX_LIVE_DEST`; otherwise nothing.

### logout_e2e

- **Needs:** the daemon, `WAMUX_REF` paired.
- **Env:** `WAMUX_REF`, `WAMUX_SOCKET_PATH`.
- **Proves:** Logout unlinks the device and the account then reports DISCONNECTED (the row and keys are kept, so it can be paired again).
- **Writes to WhatsApp:** it unlinks the device from the phone's linked-devices list. Destructive: you must pair again.

### newsletter_history_live

- **Needs:** the daemon, `WAMUX_REF` paired and following at least one channel.
- **Env:** `WAMUX_REF`, `WAMUX_CHANNEL` (one `@newsletter` jid; default every subscribed channel), `WAMUX_SOCKET_PATH`. Argument: `[count]`.
- **Proves:** issue #26: pages arrive and project, `before` paginates strictly older, `count = 0` is refused, and a poll row's vote hashes match its options.
- **Writes to WhatsApp:** nothing; every call is a read.

### pair_backfill

- **Needs:** Postgres (in process, no daemon) and a phone to scan the QR.
- **Env:** `WAMUX_REF`, `DATABASE_URL`. Argument: `[watch_secs]`.
- **Proves:** a fresh QR pairing with backfill on emits the InitialBootstrap history dump (at least one HistorySyncEvent).
- **Writes to WhatsApp:** it links a new device to the phone; no message.

### pair_cli

- **Needs:** Postgres (in process, no daemon) and a phone.
- **Env:** `WAMUX_REF`, `DATABASE_URL`. Argument: `qr` or the international digits for a pair code (requested once, never retried).
- **Proves:** pairing completes with a jid. The self-message that follows is only accepted (a self-chat has no receipt).
- **Writes to WhatsApp:** it links a device and sends one note to your own number.

### pair_socket

- **Needs:** the daemon and a phone to scan the QR.
- **Env:** `WAMUX_REF`, `WAMUX_SOCKET_PATH`.
- **Proves:** pairing through the socket completes with a jid. The self-message that follows is only accepted.
- **Writes to WhatsApp:** it links a device and sends one note to your own number.

### poll_live

- **Needs:** the daemon, `WAMUX_REF` paired, and a person on the destination phone who votes.
- **Env:** `WAMUX_REF`, `WAMUX_LIVE_DEST`, `WAMUX_POLL_EXPECT` (the option the person votes for), `WAMUX_DELIVERY_SECS`, `WAMUX_SOCKET_PATH`. Argument: `[seconds]`.
- **Proves:** issue #13: the poll and this account's two votes are delivered, and the tally of the phone's vote has `undecryptable=0` and names the expected option.
- **Writes to WhatsApp:** to `WAMUX_LIVE_DEST` one poll and two poll votes.

### read_receipt_live

- **Needs:** the daemon, `WAMUX_REF` paired, and a person on the destination phone who replies.
- **Env:** `WAMUX_REF`, `WAMUX_LIVE_DEST`, `WAMUX_DELIVERY_SECS`, `WAMUX_SOCKET_PATH`. Argument: `[seconds]`.
- **Proves:** issue #20: the prompt is delivered and the reply arrives. MarkRead and MarkChatRead answer Empty, so they are accepted; look at the other phone for blue ticks and at this account's own app for the badge.
- **Writes to WhatsApp:** to `WAMUX_LIVE_DEST` one text, then a read receipt for the reply and an app-state read mark.

### recv_media

- **Needs:** the daemon, `WAMUX_REF` paired, and a person who sends an image or other media.
- **Env:** `WAMUX_REF`, `WAMUX_SOCKET_PATH`. Argument: `[seconds]`.
- **Proves:** DownloadMedia streams more than zero bytes for an inbound media message.
- **Writes to WhatsApp:** nothing.

### send_echo_live

- **Needs:** the daemon, `WAMUX_REF` paired; optionally a second paired account.
- **Env:** `WAMUX_REF`, `WAMUX_LIVE_DEST`, `WAMUX_PEER_REF` (optional), `WAMUX_DELIVERY_SECS`, `WAMUX_SOCKET_PATH`. Argument: `[seconds]`.
- **Proves:** issue #22: the text is delivered and a subscription that made no call sees the send (and, with a peer, the peer sees it under the same id).
- **Writes to WhatsApp:** to `WAMUX_LIVE_DEST` one text.

### send_types

- **Needs:** the daemon, `WAMUX_REF` paired, internet for the sample files (an OGG/Opus file for `ptt`).
- **Env:** `WAMUX_REF`, `WAMUX_LIVE_DEST`, `WAMUX_DELIVERY_SECS`, `WAMUX_PTT_FILE`, `WAMUX_PTT_SECS`, `WAMUX_SOCKET_PATH`. Argument: `[all|video|audio|sticker|ptt]` (`all` excludes `ptt`).
- **Proves:** video, audio, sticker and voice note each reach the destination's phone and are delivered.
- **Writes to WhatsApp:** to `WAMUX_LIVE_DEST` one message per selected kind.

### set_pfp

- **Needs:** the daemon, `WAMUX_REF` paired.
- **Env:** `WAMUX_REF`, `WAMUX_SOCKET_PATH`.
- **Proves:** SetProfilePicture changes the picture as read back, and the restore is itself a recorded check.
- **Writes to WhatsApp:** it replaces the account's profile picture with a test image, then restores (or removes) it.

### stress_live

- **Needs:** Postgres, a paired real account, and the `stress` feature (`cargo run --features stress --bin stress_live`). Not built by default.
- **Env:** `WAMUX_REF`, `WAMUX_LIVE_DEST`, `DATABASE_URL`. Arguments: `[n_fakes] [probes_per_phase]`.
- **Proves:** the real account's delivery-receipt round trip, with and without N fake connections held against the local mock; a probe with no receipt in 30 seconds fails.
- **Writes to WhatsApp:** to `WAMUX_LIVE_DEST` one probe text per probe (2 phases). The fakes never leave the machine.
