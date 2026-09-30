#!/usr/bin/env bash
# Hold the workspace's crate boundaries (#62).
#
# 1. The daemon links none of the crates only the wamux-tools binaries use:
#    ureq 2 (the daemon's HTTP goes through ureq 3, inside
#    whatsapp-rust-ureq-http-client), qrcode and image. Before the split every
#    src/bin/*.rs was built with the daemon, and these rode along.
# 2. wamux-proto depends on no wamux crate: it is generated code, the bottom
#    of the graph.
#
# Each tree is checked against an absolute floor, so an empty `cargo tree`
# (wrong package name, broken manifest) can never pass as "nothing forbidden".
set -euo pipefail
cd "$(dirname "$0")/.."

# One "name vX.Y.Z" per line, every feature on, the given edge kinds.
packages_of() {
    local pkg="$1" edges="$2"
    cargo tree -p "$pkg" --all-features -e "$edges" --prefix none --format '{p}' \
        | sed -E 's/ \(.*$//' | sort -u
}

require_floor() {
    local pkg="$1" count="$2" floor="$3"
    if (( count < floor )); then
        echo "ERROR: cargo tree -p $pkg listed only $count packages; expected at least $floor" >&2
        exit 1
    fi
}

failed=0

daemon=$(packages_of wamux normal)
require_floor wamux "$(wc -l <<<"$daemon")" 200
grep -qE '^wamux-proto v' <<<"$daemon" || { echo "ERROR: wamux does not depend on wamux-proto" >&2; failed=1; }
for pattern in '^ureq v2\.' '^qrcode v' '^image v'; do
    if hits=$(grep -E "$pattern" <<<"$daemon"); then
        echo "ERROR: the daemon links a tools-only crate: $hits (move it to wamux-tools)" >&2
        failed=1
    fi
done

proto=$(packages_of wamux-proto normal,build)
require_floor wamux-proto "$(wc -l <<<"$proto")" 20
grep -qE '^tonic v' <<<"$proto" || { echo "ERROR: wamux-proto does not depend on tonic" >&2; failed=1; }
if hits=$(grep -E '^wamux(-[a-z]+)? v' <<<"$proto" | grep -vE '^wamux-proto v'); then
    echo "ERROR: wamux-proto depends on a wamux crate: $hits" >&2
    failed=1
fi

(( failed == 0 )) || exit 1
echo "crate boundaries hold: daemon $(wc -l <<<"$daemon") packages, wamux-proto $(wc -l <<<"$proto")"
