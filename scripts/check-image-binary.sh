#!/usr/bin/env bash
# Fail unless the image carries exactly the binary the release tested (#218).
#
# Usage: scripts/check-image-binary.sh <image> <binary>
#
# Why: the image job compiles nothing; it is handed the release job's binary.
# This extracts /usr/local/bin/wamux from the image that was pushed and
# compares its sha256 to that file, so a wrong or stale build context cannot
# publish bytes nobody checked.
set -euo pipefail

IMAGE="${1:?usage: check-image-binary.sh <image> <binary>}"
BINARY="${2:?usage: check-image-binary.sh <image> <binary>}"

workdir=$(mktemp -d)
container=""
# A scratch image has no shell to exec into, so the container is only created,
# never started; it is removed on every exit path.
cleanup() {
  [[ -z "$container" ]] || docker rm --force "$container" >/dev/null 2>&1 || true
  rm -rf "$workdir"
}
trap cleanup EXIT

container=$(docker create "$IMAGE")
docker cp "${container}:/usr/local/bin/wamux" "${workdir}/wamux"

want=$(sha256sum "$BINARY" | awk '{print $1}')
have=$(sha256sum "${workdir}/wamux" | awk '{print $1}')
if [[ "$want" != "$have" ]]; then
  echo "ERROR: image binary differs from the release binary" >&2
  echo "  release ($BINARY): $want" >&2
  echo "  image   ($IMAGE): $have" >&2
  exit 1
fi
echo "image binary ok: $have"
