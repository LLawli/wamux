# Breaking changes for the operator: the media spool (#79, 2026-10-13)

No `.proto` changes. SendMedia, PostStatusMedia and DownloadMedia now stage
files on disk instead of holding them in memory, so the daemon needs a place to
put them, and a few refusals are new. Check them before upgrading.

| # | Change | Impact |
|---|--------|--------|
| 1 | `media_spool_dir` must exist or be creatable, and be writable | **BREAKING** (startup fails where it used to start) |
| 2 | DownloadMedia refuses a file above `media_max_bytes` | **BREAKING** (it used to download any size) |
| 3 | A global budget, `media_spool_max_bytes`, can refuse a transfer | New `ResourceExhausted` |
| 4 | An invalid media type is refused before the bytes are read | Error order changes in one case |

## 1. The spool directory is required and writable

**Before:** the daemon needed no scratch space.

**Now:** `media_spool_dir` (default `/var/cache/wamux`, env
`WAMUX_MEDIA_SPOOL_DIR`) is prepared at startup, before the store opens. Missing
levels are created `0700`; an existing directory keeps its mode. The daemon then
creates one file there to prove it is writable, and stops with the path in the
error if it cannot (`cannot create the media spool directory <path>` or `the
media spool directory <path> is not writable`).

**To migrate:** the shipped `contrib/wamux.service` now has `CacheDirectory=wamux`
and `Environment=WAMUX_MEDIA_SPOOL_DIR=%C/wamux`; a unit written by hand needs
the same, or a writable directory of your choosing. Under `ProtectSystem=strict`
the directory must be one systemd makes writable (`CacheDirectory=` does).
Never point it at a tmpfs or `/tmp`: that is RAM. See `docs/DEPLOYMENT.md`.

## 2. DownloadMedia above `media_max_bytes`

**Before:** `media_max_bytes` limited only uploads, and a download of any size
was held in memory.

**Now:** a descriptor whose `file_length` is above `media_max_bytes` is refused
with `ResourceExhausted("media exceeds size limit")` before anything is fetched.
A descriptor with `file_length = 0` is allowed, and the file is capped at
`media_max_bytes` while it is written.

**To migrate:** raise `media_max_bytes` if you download larger files.

## 3. The global spool budget

`media_spool_max_bytes` (default 2 GiB, `0` = unlimited) bounds the bytes all
transfers in flight hold in the spool, summed. A transfer that does not fit gets
`ResourceExhausted("media spool budget exhausted")`: a download before anything
is fetched, an upload mid-stream with nothing sent. The bytes return when the
transfer ends, fails, or the client goes away. Retry is edge policy.

## 4. A bad media type is refused before the bytes

The encryptor needs the media type to start, so SendMedia and PostStatusMedia
validate it from the header before reading any byte, with the same messages as
before. This changes the order of errors only for a stream that has an invalid
type and also a later error (an over-limit chunk, a second header): it used to
report the later error, and now reports the type.
