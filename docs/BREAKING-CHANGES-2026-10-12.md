# Breaking changes for the operator: the socket bind (#77, 2026-10-12)

No `.proto` changes. These are changes to how the daemon starts and who it
serves, so a deployment that worked yesterday can fail to start or answer
`PermissionDenied` today. Check them before upgrading.

| # | Change | Impact |
|---|--------|--------|
| 1 | The daemon refuses a live socket, or a non-socket, at `socket_path` | **BREAKING** (startup fails where it used to take over) |
| 2 | `socket_group` now `chown`s the socket and fails startup when it cannot | **BREAKING** (the key used to be informational) |
| 3 | A parent directory the daemon creates is `0750` | Behavior change |
| 4 | `allowed_uids` / `allowed_gids` are new | Additive; empty keeps today's behavior |

## 1. A live socket or a non-socket at the path stops startup

**Before:** the daemon removed whatever was at `socket_path` and bound. A second
daemon pointed at the same path silently stole the socket from the first, which
kept running on a socket nobody could reach. A regular file at the path was
deleted.

**Now:** the daemon probes the path first.

- A socket that accepts a connection: startup fails with `another wamux is
  serving <path>`, and the first daemon is untouched.
- Something that is not a socket: startup fails with `<path> exists and is not a
  socket; refusing to remove it`, and the file is left as it was.
- A socket that refuses connections (a stale file after a crash) is replaced, as
  before.
- Any other probe error (for example `EACCES`) fails startup: a socket that
  cannot be proven dead is not removed.

The probe runs before the store opens, so the refused daemon exits before it
runs any migration on the live daemon's database.

**To migrate:** a deployment that relied on the takeover (a restart that starts
the new daemon before the old one is gone) must stop the old daemon first.
`systemctl restart` already does: it stops the unit before starting it again.

## 2. `socket_group` is real

**Before:** `socket_group` was informational; the socket kept the daemon's
process group.

**Now:** the value is a group name (resolved through NSS) or a number (taken as a
gid), and the daemon `chown`s the socket to it. Startup fails when the group
does not exist, and when the daemon's user is not a member of it (root may give
the socket to any group). A deployment that had `socket_group` set to a group
the user is not in, because it did nothing, now fails to start: add the user to
the group or remove the key.

## 3. The created parent is `0750`

The daemon creates the missing levels of the socket's parent directory with mode
`0750` (before umask). A parent that already exists keeps its mode. The socket is
bound in a private `0700` sibling directory (`.wamux-bind-<pid>`) and renamed
onto `socket_path`, so it never shows up wider than `socket_mode`. The private
directory is removed on success and on failure.

## 4. Optional peer allowlist

`allowed_uids` and `allowed_gids` (lists of numbers, empty by default). With
either set, only a peer whose uid is listed, or whose effective gid is listed,
is served; others get `PermissionDenied("peer not allowed")`, on every service,
reflection included. There is no implicit uid, not even the daemon's own. The
gid is the effective gid from `SO_PEERCRED`; supplementary groups are not
visible, so list the uid for a peer that is in a group only as a supplementary
one.
