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

## The media spool (#79)

SendMedia, PostStatusMedia and DownloadMedia stage each file on disk, so the
memory a transfer takes does not depend on its size. An upload is encrypted
into the spool a chunk at a time (the plaintext never touches disk) and then
sent to the CDN from there. A download is decrypted into the spool, checked,
and only then streamed to the client.

```toml
media_spool_dir = "/var/cache/wamux"   # WAMUX_MEDIA_SPOOL_DIR
media_spool_max_bytes = 2147483648     # WAMUX_MEDIA_SPOOL_MAX_BYTES, 0 = unlimited
media_max_bytes = 104857600            # WAMUX_MEDIA_MAX_BYTES, caps one upload and one download
```

- **The directory is required and must be writable.** The daemon creates the
  missing levels as `0700` at startup, proves it can create a file there, and
  stops with the path if it cannot. The shipped unit sets `CacheDirectory=wamux`
  and points the key at it; `docker-compose.yml` uses a volume.
- **Files have no name.** They are opened unlinked (`O_TMPFILE`), so the
  directory looks empty during and after every transfer, and a crash leaves
  nothing behind to clean up.
- **Never on a tmpfs.** `/tmp` and `/run` are RAM on most systems; a spool there
  puts the files back in memory and defeats the point. Check `df -h
  $(dirname <dir>)`: the filesystem must be a real disk.
- **Sizing.** `media_spool_max_bytes` is the most all transfers in flight may
  hold together. A download reserves its declared `file_length` before fetching
  (`media_max_bytes` when the descriptor declares none); an upload reserves as
  its chunks arrive, plaintext length plus up to 26 bytes of padding and MAC.
  Past the budget a transfer gets `ResourceExhausted` ("media spool budget
  exhausted"); an upload refused mid-stream sends nothing. Pick the budget
  below the free space of the spool's filesystem, and at least a few times
  `media_max_bytes` if you expect concurrent transfers.
- **A download above `media_max_bytes` is refused** (`ResourceExhausted`, "media
  exceeds size limit") before anything is fetched.

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

## Upgrading a store written before #31

Until #31 the store kept three structured blobs (the device, the app-state
versions and the app-state sync keys) as bincode. The one-row `blob_format`
table records which format a store holds, and a 0.2.x daemon converts a store
still on `bincode` to `protobuf` the first time it opens it: every row is
converted and checked before anything is written, and the rows and the marker
change in one transaction. It logs a warning when it converted any row. (A new
store is created marked `bincode` and converts zero rows on its first start;
that is logged at info, since there is nothing in it 0.3.0 could not read.)

**Upgrade through 0.2.x before 0.3.** 0.2.x is the last release line that
converts a bincode store. 0.3.0 deletes the conversion (#179) and refuses to open
a store whose marker still reads `bincode`, naming the 0.2.x release to upgrade
through. It never guesses the format from the bytes.

Before upgrading past 0.2.x, ask the running daemon:

```
grpcurl -unix -plaintext /run/wamux.sock wamux.v1.AdminService/Check
```

`"blobFormat": "BLOB_FORMAT_PROTOBUF"` means the store is ready for 0.3. A 0.2.x
daemon that is up always reports it, since it converts a store or refuses to
start; the field is what lets you check without reading the database.

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

## Verifying a release

Each release ships a tarball on the GitHub release page and an image at
`ghcr.io/llawli/wamux:<tag>`. Both carry a build provenance attestation (#78):
a signed statement, made by the release workflow itself, of which commit and
which workflow run produced the artifact. The `.sha256` next to the tarball only
proves the download matches the release page; the attestation proves where it
came from. Check it with the GitHub CLI before you deploy:

```
gh attestation verify wamux-<version>-x86_64-unknown-linux-gnu.tar.gz -R LLawli/wamux
gh attestation verify oci://ghcr.io/llawli/wamux:<tag> -R LLawli/wamux
```

The image is built only after the release job passed its gates (the test
suite, the tag/version check and the release notes), so a tag that failed them
has no image. The build is `--locked` to the tested `Cargo.lock`, and the base
images are pinned by digest.

## Native (systemd)

The simplest case: daemon and consumer are the same user on the same host, so
permissions take care of themselves.

```sh
cargo build --release -p wamux --bin wamux
mkdir -p ~/.local/bin ~/.config/systemd/user
cp target/release/wamux ~/.local/bin/
cp contrib/wamux.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now wamux
```

The unit uses `StateDirectory=wamux`, so the SQLite database and the socket
both land in `~/.local/state/wamux/`, and `CacheDirectory=wamux` for the
[media spool](#the-media-spool-79), which lands in `~/.cache/wamux/`. The
consumer connects to `~/.local/state/wamux/wamux.sock`.

For a system-wide install, follow the comment at the top of the unit: put it in
`/etc/systemd/system/`, add `User=` and `Group=`, and replace `%h` with an
absolute path. The same directives then give `/var/lib/wamux/` and
`/var/cache/wamux/`. A consumer running as another user reaches the socket
through its group: set `socket_group` (`WAMUX_SOCKET_GROUP`) to a group both
users belong to, see
[How the daemon binds the socket](#how-the-daemon-binds-the-socket-77).

In the system-wide unit, the socket sits next to the database, in a state
directory that systemd creates `0755`. For a consumer of another user, move the
socket out of it (`RuntimeDirectory=wamux`, `RuntimeDirectoryMode=0750`, `Group=` the shared
group, `Environment=WAMUX_SOCKET_PATH=%t/wamux/wamux.sock`) and set
`StateDirectoryMode=0700`, as [The store is a secret](#the-store-is-a-secret)
asks. A user unit cannot serve another user: its `%t` is private to its owner.

## Docker, consumer on the host

This is what `docker-compose.yml` sets up: Postgres behind the daemon, the
socket bind-mounted out to `./run` on the host, and the media spool on the
`wamux-spool` named volume (`WAMUX_MEDIA_SPOOL_DIR=/var/lib/wamux/spool`).

```sh
mkdir -p run
docker compose up -d --build
# socket at ./run/wamux.sock
```

**Create `./run` before the first start.** It is gitignored, so a fresh clone
does not have it, and Docker creates a missing bind-mount source owned by root.
The daemon binds in a private directory it creates next to the socket, so in a
root-owned `./run` startup stops with `binding unix socket`, caused by
`create private bind directory for /run/wamux/wamux.sock` and
`Permission denied`. If Docker already created it,
`sudo chown "$(id -u):$(id -g)" run` (this works when the image's UID is yours,
see below).

**The UID has to match, and this is the step people get wrong.** The image
creates a `wamux` user whose UID is a build argument: the Dockerfile defaults it
to `10001`, and the compose file passes `${WAMUX_UID:-1000}` (the GID likewise).
`./run` must be writable by that UID. If your host user is not 1000, the daemon
cannot create its bind directory in your `./run`: startup stops with the same
`create private bind directory` / `Permission denied` error, and the container
restarts in a loop (`docker compose logs wamux`). Fix it at build time, so the
daemon runs as you:

```sh
WAMUX_UID=$(id -u) WAMUX_GID=$(id -g) docker compose up -d --build
```

If the stack already started under another UID, remove the spool volume first
(`docker compose down`, then `docker volume rm <project>_wamux-spool`; not
`down -v`, which also deletes `wamux-pgdata`). It holds nothing durable, but it
keeps the old owner, and the new user would stop at
`the media spool directory /var/lib/wamux/spool is not writable`.

The published image (`ghcr.io/llawli/wamux`) runs as `10001:10001`. Give `./run`
to that uid (`sudo chown 10001:10001 run`), and keep the compose file's
`WAMUX_MEDIA_SPOOL_DIR`: the image sets only `WAMUX_SOCKET_PATH`, and the default
spool, `/var/cache/wamux`, cannot be created by that user. A host consumer of
that image then runs as uid or gid 10001, or you rebuild.

A **bind mount** is used rather than a named volume on purpose: a named volume
lives under `/var/lib/docker`, which only root can traverse, so the socket in it
is out of reach of a normal user on the host whatever its own owner and mode.

## Docker, consumer in another container

Here a **named volume is the right choice**: the socket never needs to be
visible on the host, and both containers see it at the same path. A new named
volume starts as a copy of the image's `/run/wamux`, which the Dockerfile
creates owned by the `wamux` user, so nothing has to be created by hand.

```yaml
services:
  wamux:
    # ...as in docker-compose.yml, but:
    volumes:
      - wamux-socket:/run/wamux
      - wamux-spool:/var/lib/wamux

  my-edge:
    image: my-edge:latest
    depends_on: [wamux]
    # The wamux user's UID and GID, or the 0660 socket is unreachable. The same
    # expression as the build args, so the two cannot drift apart.
    user: "${WAMUX_UID:-1000}:${WAMUX_GID:-1000}"
    environment:
      WAMUX_SOCKET: /run/wamux/wamux.sock
    volumes:
      - wamux-socket:/run/wamux

volumes:
  wamux-pgdata:
  wamux-socket:
  wamux-spool:
```

Both containers must agree on UID/GID, and exporting `WAMUX_UID` and
`WAMUX_GID` sets both: the compose file passes them as build args and `my-edge`
reads them for `user:`. Export your consumer's uid and gid before
`docker compose up -d --build`. With the published image
(`image: ghcr.io/llawli/wamux:<tag>` in place of `build:`), which runs as
`10001`, export `WAMUX_UID=10001 WAMUX_GID=10001`. Do not pass `--build-arg`
alone: the consumer's `user:` would not follow it.
`depends_on` only orders the starts: the socket appears after the daemon has
opened its store, so the consumer retries its first connect.

Both named volumes copy the image's ownership once, when they are created. If
the stack already started under another UID, run `docker compose down` and
`docker volume rm` both `<project>_wamux-socket` and `<project>_wamux-spool`
before rebuilding (not `down -v`, which also deletes `wamux-pgdata`, the store).

## Troubleshooting

**`permission denied` connecting to the socket.** The connect itself is
refused, so it is the owner and mode of the socket or of a directory above it.
Check both: `ls -l ./run/wamux.sock` (or wherever `socket_path` points) and
`ls -ld` on its parent. A parent the daemon created is `0750` less the umask
(so `0700` under the shipped unit and the image), with the daemon's own group,
not `socket_group`, so a consumer that shares only `socket_group` cannot reach
the socket: create the directory yourself (or with `RuntimeDirectory=`) with
that group. With Docker, if the socket's UID is not yours, give `./run` back to
yourself, remove the spool volume and rebuild with
`WAMUX_UID=$(id -u) WAMUX_GID=$(id -g)`, as in
[Docker, consumer on the host](#docker-consumer-on-the-host). Adding your user
to the socket's group (its `socket_group`, or the daemon's own group) works
only when the directories above the socket let that group through; log in again
for the new group to apply.

**`PermissionDenied` "peer not allowed" from every RPC.** The connect worked,
and the [peer allowlist](#allowlist-by-peer-credentials-77) refused the caller;
the daemon logs a warning with its uid and gid. Add the consumer's uid to
`allowed_uids`: `allowed_gids` only matches its effective gid.

**`path must be shorter than SUN_LEN`** at startup, under `binding unix socket`
and `bind <path>`. A Unix socket path holds at most 107 bytes (the kernel's
108-byte `sun_path`, less the closing NUL), and the daemon fails to bind rather
than truncating. The path it binds is the staging one,
`<dir>/.wamux-bind-<pid>/s`, up to 22 bytes past the directory, so a directory
longer than about 85 bytes fails even when `socket_path` itself fits; clients
need the final path within the limit too. Use a short path:
`/run/wamux/wamux.sock`, not a deeply nested one.

**Socket file exists but nothing answers.** A socket left by an unclean
shutdown. The daemon unlinks it on a clean stop and replaces a socket that
refuses connections at startup, so a leftover means the process died, or never
got as far as binding (that comes after the store opens). Check the logs
(`journalctl --user -u wamux`, `journalctl -u wamux` for a system unit,
`docker compose logs wamux`) before deleting
anything; the next start replaces it on its own. A start that finds a live
socket stops with `another wamux is serving <path>` instead (see
[How the daemon binds the socket](#how-the-daemon-binds-the-socket-77)).

**Startup stops at `preparing the media spool directory`.** The
[spool](#the-media-spool-79) could not be created
(`cannot create the media spool directory <path>`) or written
(`the media spool directory <path> is not writable`). Its default,
`/var/cache/wamux`, is not writable by a normal user, nor by the image's `wamux`
user, so a daemon started by hand outside the unit, or the image run without the
compose file's environment, points `media_spool_dir`
(`WAMUX_MEDIA_SPOOL_DIR`) at a directory it owns, on a real disk. The same goes
for `socket_path`, whose default `/run/wamux.sock` needs write access to `/run`.

**`ready=false` from `AdminService.Check`.** The daemon is serving but storage
is not answering: Postgres is down or unreachable, or a SQLite or Turso
connection stopped answering. `serving` stays true because the RPC layer is
fine; the two fields are separate for exactly this reason. The probe is a
`SELECT 1` (on SQLite, on a reader and on the writer), so it proves the store
answers, not that a write would succeed.

