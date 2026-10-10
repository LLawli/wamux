# Deployment

`wamux` exposes **one Unix domain socket** and nothing else. There is no TCP
port to publish, no TLS to terminate, and no HTTP endpoint. Everything below is
about one question: how does the consumer reach that file, and is it allowed to
open it.

Two things decide that, always:

1. **The path.** The consumer must see the socket at some path it can open.
2. **The permissions.** The socket is created `0660` (owner and group only), so
   the consumer's UID must own it or its GID must match. This is deliberate:
   the socket has no authentication of its own, so anyone who can open it
   controls every account. Filesystem permissions *are* the security boundary.

## Storage engine

The `database_url` scheme picks the engine, and nothing else needs to change:

```
postgres://user:pass@host:5432/wamux    # many accounts; a database server
sqlite:///var/lib/wamux/wamux.db        # single host; no server, file created if absent
turso:///var/lib/wamux/wamux.db         # EXPERIMENTAL, see below; needs a --features turso build
```

SQLite pins its pool to one connection so the process serializes its own
writes, which is correct for a handful of accounts and a bottleneck for many.
Postgres is the answer when accounts pile up.

### Turso (experimental)

`turso://<path>` runs the same schema on the native `turso` crate (a Rust
rewrite of SQLite) instead of sqlx. It is a third engine for a single host, not
a replacement for either of the others. What to know before choosing it:

- **It is a build option.** The `turso` cargo feature is off by default and the
  release binaries do not carry it, so the shipped daemon refuses a `turso://`
  DSN with an error naming the missing feature. Build with
  `cargo build --release --features turso`.
- **The DSN is a path and nothing else.** `turso:///abs/x.db` is `/abs/x.db`,
  `turso://x.db` is the relative `x.db`. A query string (`?mode=rwc`) is refused
  rather than ignored: turso has no options, and silently dropping one would
  hand you a different store than you asked for.
- **Same file as `sqlite://`, both ways.** It applies the SQLite migrations and
  records them in `_sqlx_migrations` exactly as sqlx does, so a store created
  under `sqlite://` opens under `turso://` and the reverse, nothing converted.
  The way back is the exit if turso ever misbehaves, and it is tested. Do it
  with the daemon stopped.
- **The lock trap: one process, one opener.** The file lock is a POSIX lock of
  the process, so opening and closing ANY descriptor of the `.db` inside the
  same process drops it, and a commit can be lost (reproduced on turso 0.8.1).
  The daemon never opens the file any other way, and neither may anything you
  embed next to it. Never run two daemons on one file either.
- **Durability.** The connection runs `synchronous = FULL` (turso only documents
  OFF and FULL), the journal is WAL, and `foreign_keys` is on. That is slower on
  every commit than the SQLite engine's `NORMAL`, in exchange for no lost commit
  on power loss.
- **One connection.** Every account shares it behind a mutex, like SQLite's
  one-connection pool. There is no `BEGIN CONCURRENT` / MVCC mode: it would leave
  the file unreadable by the `sqlite3` tool and by `sqlite://`.
- **Not stress-tested.** The store and service suites run on it; the stress
  harness does not. Prefer Postgres for many accounts.

## The store is a secret

The store holds, for every paired account, the private identity keys, the Signal
sessions, the prekeys, the sender keys and the message secrets, in plaintext
(`BYTEA` in Postgres, `BLOB` in SQLite and Turso). Whoever owns a dump, a
backup or the database file can take over every paired account. Treat it like
a private key.

- **Modes.** The database file is `0600`, its directory `0700`, and both are
  owned by the user the daemon runs as.
- **What the daemon does.** It creates a new SQLite or `turso://` file `0600`
  under any umask; the `-wal` and `-shm` files inherit that mode. At startup it
  logs a warning for any existing file (or its `-wal`/`-shm`) readable by group
  or others. It does not change the mode and does not refuse to start; fix it
  with `chmod 600 <path>`.
- **Postgres.** Give the daemon a role of its own, not a shared superuser, and
  do not expose the server to open network access. The compose file's
  `wamux-pgdata` volume holds the same secret.
- **Backups.** Encrypt them, restrict who can read them, and choose retention
  as you would for a private key.
- **Encryption at rest** is available, for new stores and for stores with accounts: see the next section.

## Encryption at rest

A key makes the store useless to whoever copies only the database: the secret
columns are sealed with XChaCha20-Poly1305 (#164, the first part of #76), so a
dump, a backup or the file without the key holds ciphertext. The key lives
outside the database, in a file.

- **Generate it.** 32 random bytes as 64 hex characters, readable by its owner
  only. The daemon refuses a key file with any group or other permission bit.

  ```sh
  umask 077
  openssl rand -hex 32 > /etc/wamux/store-key
  chmod 600 /etc/wamux/store-key
  ```

- **Point the daemon at it.** `store_key_file` in `wamux.toml`, or
  `WAMUX_STORE_KEY_FILE`, is the PATH of the file. The key itself is never a
  config value or an environment variable: an environment is readable through
  `/proc/<pid>/environ` and `docker inspect`. At startup the daemon logs
  `store encryption on, key id <hex>`; the id is the first 4 bytes of the key's
  SHA-256, enough to tell keys apart and nothing more.
- **systemd.** Let systemd deliver the file with `LoadCredential=`, so the key
  stays `0600` under `/etc` and the service sees a private copy:

  ```ini
  LoadCredential=store-key:/etc/wamux/store-key
  Environment=WAMUX_STORE_KEY_FILE=%d/store-key
  ```

  `contrib/wamux.service` carries this as a commented block.
- **Docker / Compose.** Use a secret and point the variable at it:

  ```yaml
  services:
    wamux:
      environment:
        WAMUX_STORE_KEY_FILE: /run/secrets/wamux_store_key
      secrets:
        - wamux_store_key
  secrets:
    wamux_store_key:
      file: ./store-key
  ```

  A file secret is a bind mount of the host file, so it keeps the host file's
  mode and owner: keep it `0600` and owned by the uid the daemon runs as in the
  container, or the daemon refuses it.

**What is sealed.** The 13 columns that hold key material or message content:
`device.data`, `identities.key`, `sessions.record`, `prekeys.key`,
`signed_prekeys.record`, `sender_keys.record`, `app_state_keys.key_data`,
`app_state_versions.state_data`, `app_state_mutation_macs.value_mac`,
`base_keys.base_key`, `tc_tokens.token`, `msg_secrets.secret` and
`sent_messages.payload`. The last one is the text of recently sent messages
(kept for retries), so it is sealed too. Each blob is bound to its device,
table, column and row, so a blob copied to another row or account does not open.

**What is not.** `app_state_keys.key_id` and `app_state_mutation_macs.index_mac`
stay in the clear because the daemon looks rows up by them. Account metadata,
`device_registry` and `lid_pn_mapping` (phone numbers, device lists) stay in the
clear too: they are not key material. Sealing protects the secrets, not the
fact that an account exists.

**Turning a key on over a store with accounts (#165).** Set `store_key_file` and
start the daemon: it converts the store before it serves anything.

- **Back up first.** Take a database backup before the first start with the key.
  A conversion that is interrupted is resumed, but a backup is the way back from
  anything else.
- **What it does.** It records the key's id and verifier, then seals the 13
  columns one account at a time, one transaction per account, in `device_id`
  order, logging `sealing account i of n`. The store is marked encrypted by the
  last account's transaction. If the process dies half way, start it again with
  the SAME key: accounts already sealed are skipped (a progress table says
  which), nothing is sealed twice, and a different key is refused with "does not
  match". Started without the key, a half-converted store refuses with
  "interrupted" and names `store_key_file`.
- **Old plaintext is scrubbed.** An `UPDATE` does not erase the old bytes: SQLite
  keeps them in free pages and in the WAL, Postgres in dead tuples. At the end of
  a conversion the daemon runs `VACUUM` and a truncating WAL checkpoint (SQLite
  and Turso) or `VACUUM FULL` on the 13 tables (Postgres). It cannot reach copies
  it does not own: **discard the backups, the Postgres WAL archives and the
  replicas made before the conversion**, they still hold the keys in the clear.
- **Stop the daemon first** if another process converts or decrypts the same
  store. SQLite and Turso lock the file; Postgres does not, so there it is on
  you not to run two at once.

**Turning it off: `wamux store decrypt --yes`.** It writes the keys of every
account back to the store in the clear, which is why it needs `--yes`. It needs
the same `store_key_file` the daemon uses (and the same `database_url`), opens
the store without serving it, decrypts account by account like the conversion
does, and prints `decrypted N account(s)`. After it the store is plaintext and
opens without a key. An interrupted run is finished by running the command
again; until then the daemon refuses to start, with "interrupted" and the
command to run. Stop the daemon before running it. The old encrypted pages are
not scrubbed on this path, and the data is now in the clear anyway.

**Limits of this version.**

- An encrypted store does not open without its key (the error names
  `store_key_file`), and a different key is refused at startup with "does not
  match", before any account is read.
- **Losing the key means re-pairing every account.** There is no recovery: keep
  a copy of the key where you keep your other secrets, apart from the database
  backups.
- Postgres is covered by the same mechanism; the key protects the columns, so
  restrict who can read the server anyway (see above).
- Changing the key of an encrypted store is not supported yet (#166).
