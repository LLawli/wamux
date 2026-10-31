#!/usr/bin/env bash
# Fail unless the default image ships /tmp root-owned with mode 1777 (#229).
#
# Usage: scripts/check-image-tmp.sh
#
# Why: `scratch` has no /tmp, and a COPY of a directory creates its destination
# with the default mode (0755), so a Dockerfile that reads right can still ship
# a /tmp the runtime user cannot write; `--chmod=1777` drops the sticky bit.
# Only the built image tells. The final stage is built with the `binary` stage
# replaced by a placeholder file, the way the release passes its binary in, so
# nothing compiles: the build only pulls the pinned base of the `dirs` stage.
# A capped builder is used when BUILDX_BUILDER names one.
#
# Linux only, like check-sqlite-no-tmp.sh: elsewhere it says so and exits 0;
# on Linux a missing docker is an error, not a skip.
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ "$(uname -s)" != Linux ]]; then
  echo "skipped: the image check needs Linux and docker (this is $(uname -s))"
  exit 0
fi
command -v docker >/dev/null || { echo "ERROR: docker is required on Linux" >&2; exit 1; }

mkdir -p target
work=$(realpath "$(mktemp -d target/image-tmp.XXXXXX)")
tag="wamux-image-tmp-check:$(basename "$work" | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9\n' '-')"
container=""
cleanup() {
  [[ -z "$container" ]] || docker rm --force "$container" >/dev/null 2>&1 || true
  docker image rm --force "$tag" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

mkdir "$work/binary"
printf 'placeholder for the image check\n' >"$work/binary/wamux"
chmod +x "$work/binary/wamux"

docker buildx build --load --quiet -t "$tag" --build-context binary="$work/binary" . >/dev/null
container=$(docker create "$tag")
listing=$(docker export "$container" | tar -tvf -)

line=$(grep -E ' tmp/?$' <<<"$listing" || true)
[[ -n "$line" ]] || { echo "ERROR: the image has no /tmp" >&2; exit 1; }
if ! grep -qE '^drwxrwxrwt 0/0 ' <<<"$line"; then
  echo "ERROR: /tmp must be drwxrwxrwt (1777) owned by 0/0, got: $line" >&2
  exit 1
fi
echo "image /tmp ok: $line"
