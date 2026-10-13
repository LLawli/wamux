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

### How the daemon binds the socket (#77)

- **It never takes over a live socket.** If something is at the path, the
  daemon probes it first. A socket that accepts a connection means another
  wamux is serving it, and startup fails with `another wamux is serving <path>`.
  A regular file (or anything that is not a socket) fails startup and is left
  untouched. Only a socket that refuses connections is stale and replaced. Any
  other probe error (for example `EACCES`) also fails startup, because the
  socket cannot be proven dead. The probe runs before the store opens, so a
  second daemon never runs migrations on the live one's database.
- **There is no permission window.** The socket is bound in a private `0700`
  directory beside the final path (`.wamux-bind-<pid>`), given its group and
  mode there, and renamed onto `socket_path`. The path only ever shows the
  final mode and group, whatever the process umask.
- **The parent directory.** Missing levels are created `0750` (before umask); a
  parent that already exists is not touched, so a systemd `RuntimeDirectory=`
  keeps the mode you gave it.
- **`socket_group`** is a group name (resolved through NSS: `/etc/group`,
  systemd-userdb, LDAP) or a number, taken as a gid. The daemon `chown`s the
  socket to it, so its user must be a member of that group (root excepted):
  otherwise startup fails with `not a member of group`, and a group that does
  not exist fails with `does not exist`. Unset, the socket keeps the daemon's
  own group.

### Allowlist by peer credentials (#77)

`allowed_uids` and `allowed_gids` (both empty by default) add a second gate on
top of the file permissions. With either set, a connection is served only if
its peer uid is in `allowed_uids` or its gid is in `allowed_gids`; any other
gets `PermissionDenied("peer not allowed")` before any RPC runs, reflection
included, and the daemon logs a warning with the uid and gid. There is no
implicit uid: list the daemon's own user if it should be admitted. The gid is
the effective gid that `SO_PEERCRED` reports at connect time, so a peer that is
in a listed group only as a supplementary one is not admitted by
`allowed_gids`: list its uid instead.

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

### What survives a crash

The engines do not commit alike. The difference matters for the Signal state
the store keeps: a ratchet that was already used to send or receive a message
and is then lost makes the following messages fail to decrypt, or resets the
session.

| Engine | Setting | A process crash | A power loss or an OS crash |
|---|---|---|---|
| SQLite | WAL, `synchronous = FULL`, group commit | loses nothing | loses nothing that was acknowledged |
| Turso | WAL, `synchronous = FULL` | loses nothing | loses nothing that was acknowledged |
| Postgres | its default (`synchronous_commit = on`) | loses nothing | loses nothing that was acknowledged |

SQLite used to run `NORMAL`, inherited from the whatsapp-rust SQLite reference,
which can lose the last commits on power loss. It now runs `FULL` (#169, #173),
and there is no setting to go back: nothing the daemon acknowledged is lost. What
makes `FULL` affordable is the group commit. Every write goes through one writer
connection, and the accounts that queue at the same moment share one transaction
and one COMMIT (one fsync) instead of paying one each; nobody is answered before
the COMMIT that carries their write, and a lone writer still commits on its own,
with no timer. The reads run on a separate pool of read-only connections: they
never wait for the batch, and they never see a write that has no COMMIT yet. A
SQLite DSN that is in memory (`:memory:`, `mode=memory`) is refused, since the
readers are other connections and each would see a database of its own. The
Turso engine groups its commits the same way (#172); Postgres is unchanged.

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
  OFF and FULL), the journal is WAL, and `foreign_keys` is on. No commit is lost
  on power loss, the same as the SQLite engine now; the difference is that its
  reads go through the same connection and can see the open batch.
- **One connection.** Every account shares it behind a mutex (SQLite has one
  writer too, plus read-only readers). There is no `BEGIN CONCURRENT` / MVCC
  mode: it would leave the file unreadable by the `sqlite3` tool and by
  `sqlite://`.
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

**Changing the key: `wamux store rotate-key --new-key-file <path>` (#166).** It
re-seals every secret of an encrypted store under a new key, and after it only
the new key opens the store. Use it on a schedule, or at once if the key may
have leaked.

1. **Stop the daemon**, and take a database backup (under the old key).
2. **Generate the new key** the same way as the first one, next to it:

   ```sh
   umask 077
   openssl rand -hex 32 > /etc/wamux/store-key.new
   chmod 600 /etc/wamux/store-key.new
   ```

3. **Rotate.** With the daemon's config (`store_key_file` still pointing at the
   OLD key, and the same `database_url`):

   ```sh
   wamux store rotate-key --new-key-file /etc/wamux/store-key.new
   ```

   It checks the old key against the store first ("does not match" if it is the
   wrong one), refuses a new key equal to the old one ("already"), opens every
   blob of the 13 columns with the old key and seals it with the new one, and
   replaces the store's key id and verifier, all in **one transaction**. If it
   dies half way nothing changed: the store is still whole under the old key,
   and running the command again starts over. It prints
   `rotated N account(s) to key id <hex>`. The new key file goes through the
   same checks as `store_key_file` (64 hex characters, mode `0600`).
4. **Swap the files**, so `store_key_file` names the new key
   (`mv /etc/wamux/store-key.new /etc/wamux/store-key`, or point the setting at
   the new path), then start the daemon. It logs the new key id.

The old blobs are scrubbed like after a conversion (`VACUUM` and a truncating
WAL checkpoint on SQLite and Turso, `VACUUM FULL` on the 13 tables on
Postgres), so the store file does not keep blobs the old key can open. Copies
made before the rotation are out of its reach: **a backup taken before it still
opens with the old key**, so if the reason to rotate is a leaked key, discard
those backups (and Postgres WAL archives and replicas) too, or keep the old key
as secret as the new one for as long as they exist. The store must be encrypted
and whole: a plaintext store, or one whose conversion or decrypt was
interrupted, is refused with the step that finishes it. As with decrypt,
Postgres does not lock the store against a second process: the daemon must be
stopped.

**Limits of this version.**

- An encrypted store does not open without its key (the error names
  `store_key_file`), and a different key is refused at startup with "does not
  match", before any account is read.
- **Losing the key means re-pairing every account.** There is no recovery: keep
  a copy of the key where you keep your other secrets, apart from the database
  backups.
- Postgres is covered by the same mechanism; the key protects the columns, so
  restrict who can read the server anyway (see above).
