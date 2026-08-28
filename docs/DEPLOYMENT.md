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
```

SQLite pins its pool to one connection so the process serializes its own
writes, which is correct for a handful of accounts and a bottleneck for many.
Postgres is the answer when accounts pile up.

## Native (systemd)

The simplest case: daemon and consumer are the same user on the same host, so
permissions take care of themselves.

```sh
cp target/release/wamux ~/.local/bin/
cp contrib/wamux.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now wamux
```

The unit uses `StateDirectory=wamux`, so the database and the socket both land
in `~/.local/state/wamux/`. The consumer connects to
`~/.local/state/wamux/wamux.sock`.

## Docker, consumer on the host

This is what `docker-compose.yml` sets up. The socket is bind-mounted out to
`./run` on the host.

```sh
docker compose up -d --build
# socket at ./run/wamux.sock
```

**The UID has to match, and this is the step people get wrong.** The image
creates a `wamux` user whose UID defaults to `1000` (the compose file passes
`${WAMUX_UID:-1000}`). If your host user is not 1000, the socket comes out
owned by someone else and every connection fails with permission denied, which
looks exactly like the daemon being down. Fix it at build time:

```sh
WAMUX_UID=$(id -u) WAMUX_GID=$(id -g) docker compose up -d --build
```

A **bind mount** is used rather than a named volume on purpose: a named volume
lives under `/var/lib/docker` and is root-owned, which puts the socket out of
reach of a normal user on the host.

## Docker, consumer in another container

Here a **named volume is the right choice** — the socket never needs to be
visible on the host, and both containers see it at the same path.

```yaml
services:
  wamux:
    # ...as in docker-compose.yml, but:
    volumes:
      - wamux-socket:/run/wamux

  my-edge:
    image: my-edge:latest
    depends_on: [wamux]
    # Same UID as the wamux container, or the 0660 socket is unreachable.
    user: "10001:10001"
    environment:
      WAMUX_SOCKET: /run/wamux/wamux.sock
    volumes:
      - wamux-socket:/run/wamux

volumes:
  wamux-socket:
```

Both containers must agree on UID/GID. Build the image with
`--build-arg WAMUX_UID=...` to match whatever your consumer runs as.

## Troubleshooting

**`permission denied` connecting to the socket.** Check the owner:
`ls -l ./run/wamux.sock`. If the UID is not yours, rebuild the image with
`WAMUX_UID=$(id -u) WAMUX_GID=$(id -g)`. Adding your user to the socket's group
also works if you own that group.

**`path must be shorter than SUN_LEN`** at startup. The kernel caps Unix socket
paths at about 107 bytes, and the daemon fails to bind rather than truncating.
Use a short path: `/run/wamux/wamux.sock`, not a deeply nested one.

**Socket file exists but nothing answers.** A stale socket from an unclean
shutdown. The daemon removes it on a clean stop and unlinks a leftover on
startup, so this usually means the process died before binding — check the logs
before deleting anything.

**`ready=false` from `AdminService.Check`.** The daemon is serving but storage
is not answering: Postgres is down or unreachable, or the SQLite file is not
writable. `serving` stays true because the RPC layer is fine; the two fields
are separate for exactly this reason.
