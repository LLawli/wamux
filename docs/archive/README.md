# Archive

Historical notes, kept for archaeology. Nothing here is maintained, and nothing
here describes the daemon as it is today: read the code, the `.proto` files and
the current docs instead.

- `crate-notes/`: a snapshot of the `whatsapp-rust` 0.6.0 API (bot, device,
  events, storage traits, the SQLite reference backend, transport), taken
  during the port. The project has since moved to a pinned git revision of the
  library, so these APIs, types and byte formats are out of date. Check the
  pinned revision's source before relying on any of it.
- `pre-0.1-breaking-changes.md`: contract changes made before the first
  release, 0.1.0. Nobody upgrades across them from a release; most were
  superseded by the 0.2.0 contract. The upgrade guides start at 0.1:
  [docs/upgrading/](../upgrading/README.md).
