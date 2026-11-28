# Upgrading

What each release asks of an operator (who runs the daemon) or an edge author
(who builds a client of its socket), one guide per version step.

## How to use them

- **Upgrade one step at a time.** Going from 0.1.0 to 0.2.4, read every guide
  from 0.1 to 0.2.4, in order, and do what each one says before the next. A
  step can depend on the one before it: the store must reach the 0.2 format
  before any later release opens it, and 0.3 will refuse a store that never
  went through 0.2.
- **Back up the store before each step.**
- Each guide has an "Operators" part and an "Edge authors" part. The
  `CHANGELOG.md` section of the same version lists everything else that
  changed, additive features included.
- A step with no guide asks nothing of you.

## Guides, newest first

- [0.2.4 to the next release](0.2.4-to-next.md): unreleased. `DeleteMessage`
  for everyone follows `from_me`; event order after a reconnect holds per chat.
- [0.2.3 to 0.2.4](0.2.3-to-0.2.4.md): pairing refuses an account that holds an
  identity; device lists are verified; a shutdown timeout for blocking tasks.
- 0.2.2 to 0.2.3: nothing to do. It adds the macOS package; on macOS the socket
  defaults to `/var/run/wamux.sock`.
- [0.2.1 to 0.2.2](0.2.1-to-0.2.2.md): a musl tarball and a `scratch` image;
  channel polls follow WA Web; four `RawEvent` kinds became typed.
- [0.2.0 to 0.2.1](0.2.0-to-0.2.1.md): library-imposed refusals and answers;
  Postgres `.pgpass` escapes.
- [0.1 to 0.2](0.1-to-0.2.md): the store migration, the socket bind, the media
  spool, and the 0.2 contract (every jid a `Jid`, enums, typed messages instead
  of JSON).

Contract changes made before the first release are in
[docs/archive/](../archive/README.md).

## Writing a guide

A breaking change, or one an operator or edge author has to act on, adds its
instructions to `docs/upgrading/<latest release>-to-next.md` (create it if it
does not exist) in the same change, next to its CHANGELOG entry. Write what to
do, not the story. When the release is cut, the file is renamed to
`<previous>-to-<new>.md` and listed here.
