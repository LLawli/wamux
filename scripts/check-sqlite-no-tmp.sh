#!/usr/bin/env bash
# Run the SQLite pairing-batch test where no temporary directory is usable (#229).
#
# Usage: scripts/check-sqlite-no-tmp.sh
#
# Why: the published image is `scratch` (no /tmp, no /var/tmp, cwd `/`), and a
# SQLite savepoint over 64 KiB spills to a temp file. A host always has a /tmp,
# so the test passes there whatever the store does. This runs the storage test
# binary in a container that sees the host's root read-only (the binary needs
# the host's libc), with /tmp and /var/tmp masked by empty read-only mounts and
# TMPDIR unset. The store lives in a directory of its own under target/.
#
# Linux only: it needs docker. Elsewhere it says so and exits 0; on Linux a
# missing docker is an error, not a skip.
set -euo pipefail
cd "$(dirname "$0")/.."

TEST="sqlite_temp_store::sqlite_prekey_upload_batch_commits_without_a_temp_dir"
# busybox:1.37, by digest: the harness must run the same tools every time.
BUSYBOX="busybox@sha256:bdf57e528e45e4433820e045b29b4597825a1c9e38353532d90a01445013f82e"

if [[ "$(uname -s)" != Linux ]]; then
  echo "skipped: the no-tmp container needs Linux and docker (this is $(uname -s))"
  exit 0
fi
command -v docker >/dev/null || { echo "ERROR: docker is required on Linux" >&2; exit 1; }

# The executable cargo built for the `storage` test target.
bin=$(cargo test -p wamux --test storage --no-run --message-format=json 2>/dev/null |
  python3 -c '
import json, sys
for line in sys.stdin:
    msg = json.loads(line)
    if msg.get("reason") == "compiler-artifact" and msg["target"]["name"] == "storage" \
            and msg.get("executable"):
        print(msg["executable"])
')
[[ -x "$bin" ]] || { echo "ERROR: no storage test binary found (got '$bin')" >&2; exit 1; }
bin=$(realpath "$bin")

# Under target/, not /tmp: /tmp is the thing being masked, and target/ exists
# on the host side of the read-only root, so it can be a mount point.
mkdir -p target
work=$(realpath "$(mktemp -d target/no-tmp.XXXXXX)")
trap 'rm -rf "$work"' EXIT
mkdir "$work/empty" "$work/store"

# The probe fails the run if any temp location SQLite tries is writable after
# all: a harness that masks nothing would pass the test vacuously.
out=$(docker run --rm --network none -w / \
  -e WAMUX_TEST_STORE_DIR="$work/store" \
  -v /:/host:ro \
  -v "$work/empty":/host/tmp:ro \
  -v "$work/empty":/host/var/tmp:ro \
  -v "$work/store":"/host$work/store" \
  "$BUSYBOX" sh -c '
    for dir in /host/tmp /host/var/tmp /host/usr/tmp /host; do
      if touch "$dir/.wamux-probe" 2>/dev/null; then
        echo "HARNESS BROKEN: $dir is writable"; exit 3
      fi
    done
    exec chroot /host "$@"
  ' -- "$bin" --exact "$TEST" 2>&1) || { printf '%s\n' "$out"; exit 1; }
printf '%s\n' "$out"

grep -qE "^test ${TEST} \.\.\. ok$" <<<"$out" || {
  echo "ERROR: $TEST did not pass in the no-tmp container" >&2; exit 1; }
grep -qE '^test result: ok\. 1 passed' <<<"$out" || {
  echo "ERROR: expected exactly one test to run" >&2; exit 1; }
echo "sqlite without a temp dir: ok"
