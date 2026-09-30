# Legacy bincode blobs from the 23846f7e build

Real blobs, not built at test time. Bincode is positional, so a "legacy" blob
made from the current `wacore` types only proves the current layout converts,
which is never the layout a legacy store holds. Upstream #1565 added
`Device.status_privacy` in the middle of the struct (#86), and every test that
built its legacy blob from the current `Device` stayed green while the real
conversion broke.

Written on 2026-09-30 by the wamux build at `d90f648` (whatsapp-rust pinned at
`23846f7e34c9842ed93c7a099a8f87818089fab7`), before the #86 bump, with a
throwaway test that is not kept:

| file | value |
|---|---|
| `device.bincode` | `every_field_device()` (storage/blob_codec/tests.rs as of `d90f648`), bincode standard |
| `device.pb` | the same `Device`, `encode_device` (DeviceBlob protobuf) |
| `hash_state.bincode` / `.pb` | `hash_state(7, true, false)` (storage/bincode_upgrade/tests.rs) |
| `sync_key.bincode` / `.pb` | `sync_key()` (same file) |

The key pairs are fresh test keys from `Device::new()`, not an account's.

The protobuf twin is field-tagged, so it decodes on any later build: it is what
the converted blob is compared against. Never regenerate these files: a new
fixture set for a new layout goes in its own directory, next to this one.
