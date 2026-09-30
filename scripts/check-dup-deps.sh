#!/usr/bin/env bash
# Fail if the gRPC/HTTP stack resolves to two versions of the same crate (#61).
# A second tonic, prost, http, hyper or tower in the lock means two copies of
# the transport compiled in, and types that look identical but do not unify.
# The rest of the graph is not checked: rand, hashbrown and friends already
# carry legitimate duplicates pulled by whatsapp-rust.
set -euo pipefail
cd "$(dirname "$0")/.."

WATCHED=(tonic tonic-prost tonic-reflection prost prost-types http http-body hyper hyper-util h2 tower)

# Every package in the full graph (all features, every dependency kind), one
# "name vX.Y.Z" per line. Checked against a floor below so an empty tree can
# never pass as "no duplicates".
packages=$(cargo tree --all-features -e normal,build,dev --prefix none --format '{p}' \
    | sed -E 's/ \(.*$//; s/ \(\*\)$//' | sort -u)
count=$(wc -l <<<"$packages")
if (( count < 100 )); then
    echo "ERROR: cargo tree listed only $count packages; expected hundreds" >&2
    exit 1
fi

failed=0
for name in "${WATCHED[@]}"; do
    versions=$(grep -E "^${name} v" <<<"$packages" | awk '{print $2}' | sort -u || true)
    if [[ -z "$versions" ]]; then
        echo "ERROR: $name is not in the dependency graph at all" >&2
        failed=1
    elif (( $(wc -l <<<"$versions") > 1 )); then
        echo "ERROR: $name resolves to more than one version:" $versions >&2
        failed=1
    fi
done
(( failed == 0 )) || exit 1
echo "no duplicate gRPC/HTTP crates (${#WATCHED[@]} checked over $count packages)"
