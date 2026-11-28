# Security policy

## Reporting a vulnerability

**Do not open a public issue for a vulnerability.** Report it privately through
GitHub's private vulnerability reporting:

**https://github.com/LLawli/wamux/security/advisories/new**

The report stays visible only to you and the maintainer until an advisory is
published. If you are not sure whether something is a vulnerability, report it
there anyway; it can be moved to a public issue later.

A useful report has:

- the wamux version (the release tag, or the commit when built from source),
  the platform (tarball, Debian package, container image, macOS, Termux) and
  the store engine (Postgres, SQLite, Turso);
- what an attacker needs (local user, member of the socket's group, read
  access to the store, a WhatsApp contact, a network position) and what they
  gain;
- the steps to reproduce, or a proof of concept.

Leave real secrets out: no store dumps, no `store_key_file`, no database
passwords, no pairing codes or QR strings, no phone numbers. Reproduce with a
spare account you can afford to lose.

## Why this project needs one

- **The store holds every account's keys.** wamux persists whatsapp-rust's
  protocol state: the Signal identity keys, sessions, prekeys and sender keys,
  the device, the app-state sync keys. Whoever reads the store can act as every
  paired account. It is plaintext unless `store_key_file` turns on encryption
  at rest (see [The store is a secret](docs/DEPLOYMENT.md#the-store-is-a-secret)).
- **The socket has no auth of its own.** The core is a relay without users or
  tokens: the socket's filesystem permissions (`socket_mode`, `socket_group`)
  and the optional peer allowlist (`allowed_uids`, `allowed_gids`) are the
  whole boundary. Anything that lets a process past them reaches every account.
- **It speaks an unofficial protocol.** wamux talks to WhatsApp through
  [`whatsapp-rust`](https://github.com/oxidezap/whatsapp-rust), a
  reimplementation of a protocol Meta does not publish. Input from the
  WhatsApp servers and from other users' messages crosses into the daemon and
  out to the edge, and none of it was designed with this code in mind.

## In scope

- **The daemon** (`crates/wamux`): a request, an event or a message from
  WhatsApp that crashes it, exhausts its memory or disk, reads or writes
  another account's state, or leaks key material into a response, an event or
  a log.
- **The store**: key material reaching disk in plaintext with encryption at
  rest on, weaknesses in that encryption or in key rotation
  (`wamux store rotate-key`), a store or spool file created with a mode wider
  than documented, decrypted media touching disk in the media spool.
- **Socket handling**: a way to reach the API without permission to open the
  socket, or past `allowed_uids` / `allowed_gids`; a socket created with a
  mode or group other than the configured one; the stale-socket and
  parent-directory handling at startup.
- **Release artifacts and their provenance**: the tarballs, the Debian and
  Termux packages, the macOS PKG and the `ghcr.io/llawli/wamux` image; their
  build provenance attestations (see
  [Verifying a release](docs/DEPLOYMENT.md#verifying-a-release)); the release
  and CI workflows; a vulnerable dependency as shipped in a release.

## Not a vulnerability here

- **A client that can open the socket controls every account.** That is the
  design: the core has no auth, and access control belongs to the filesystem
  and to the edge. A way around the socket's permissions is in scope; what a
  permitted client can do is not.
- **A deployment that widens the boundary**: a socket made world-writable, or
  forwarded over TCP, or a store readable by other users.
- **Account bans and Terms of Service enforcement** by WhatsApp.
- **The development tools** in `crates/wamux-tools`, which are not shipped:
  open a regular issue.

## Elsewhere

- **WhatsApp or Meta itself**: Meta's bug bounty program,
  https://www.facebook.com/whitehat.
- **The `whatsapp-rust` library** (the protocol, Signal, the transport): its
  own repository, https://github.com/oxidezap/whatsapp-rust. If you cannot
  tell whether a bug is in wamux or in the library, report it here privately
  and it will be routed.
- **Edges and clients built on wamux**, such as
  [`wamux-http`](https://github.com/LLawli/wamux-http) and
  [`wamux-easy-bots`](https://github.com/LLawli/wamux-easy-bots): their own
  repositories. Authentication, permissions and any HTTP API live there.

## Supported versions

Only the latest release gets security fixes. A fix ships in the next release,
not as a backport to an older line. `main` is fixed first and is the base of
that release.

## What to expect

wamux has one maintainer, working on it in their own time. Reports are handled
on a best-effort basis, with no fixed response time and no bounty. The answer
comes on the advisory itself, and a fix is released when it is ready; when it
ships, the advisory is published with credit to the reporter unless you ask
otherwise.
