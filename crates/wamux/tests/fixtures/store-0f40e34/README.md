# A store written by `0f40e34`, before the SQL was unified (#65)

Real stores, not built at test time. #65 replaced two hand-written copies of
every statement (`storage/postgres/` and `storage/sqlite/`) with one. The risk
in that is not what the new SQL writes, which the parity tests cover, but
whether it still reads what the old SQL wrote. Only a store the old code
actually wrote can show that.

Written on 2026-10-03 by the wamux build at `0f40e34` (main before #65), with
a throwaway test that is not kept. It created one account
(`external_ref = fixture-0f40e34`, `device_id = 1`) on each engine and wrote
every value in `tests/existing_store/values.rs` through the store traits, one
write per table:

| file | what |
|---|---|
| `wamux.db` | the SQLite store, WAL checkpointed and switched to `journal_mode=DELETE` so the one file holds everything. Account uuid `08d9f022-cba8-4ed7-8baa-3d077b219d8b`. |
| `postgres.sql` | `pg_dump --no-owner --no-privileges --column-inserts` (pg_dump 16.15) of a throwaway database. The `\restrict` / `\unrestrict` psql meta-commands were stripped so the file runs as plain SQL. Account uuid `f7d0ffd6-1b78-4963-9a49-814141641187`. |
| `device.pb` | `encode_device` of the `Device` both engines saved. The fixture's device row must load back to exactly these bytes. |

The `Device` holds fresh test keys from `Device::new()`, not an account's. No
file here is a real WhatsApp identity.

Both stores carry their `_sqlx_migrations` rows and `blob_format = protobuf`,
as a daemon on `0f40e34` leaves them. Tests copy `wamux.db` to a temp dir and
load `postgres.sql` into a database of their own: opening a store writes to
it, so the committed files are never opened in place.

Never regenerate these files. A fixture for a later layout goes in its own
directory, next to this one.
