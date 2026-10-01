#!/usr/bin/env bash
# Hold THIRD-PARTY-LICENSES.md to the dependency graph of the shipped daemon (#83).
#
# The file went stale (it listed whatsapp-rust 0.6.0 after the port to 0.7.0)
# and its generator once wrote 0 crates and exited 0, with nothing in CI to
# notice. This regenerates into a temp file and fails on any difference, and
# demands an absolute floor of crates plus the locked whatsapp-rust row so an
# empty graph can never pass as "unchanged".
#
# Usage: scripts/check-third-party.sh [FILE]   (default: THIRD-PARTY-LICENSES.md)
set -euo pipefail
cd "$(dirname "$0")/.."

FILE="${1:-THIRD-PARTY-LICENSES.md}"
FIX="python3 scripts/gen-third-party.py"
FLOOR=300

fail() { echo "ERROR: $*" >&2; exit 1; }

[[ -s "$FILE" ]] || fail "$FILE is missing or empty. Fix: $FIX"

fresh="$(mktemp)"
trap 'rm -f "$fresh"' EXIT
python3 scripts/gen-third-party.py --output "$fresh" >/dev/null \
    || fail "the generator failed; no license file was produced"

count="$(grep -oE '^\*\*[0-9]+ crates\*\*' "$fresh" | grep -oE '[0-9]+' || true)"
(( ${count:-0} >= FLOOR )) || fail "regenerated file lists ${count:-0} crates; expected at least $FLOOR"

# Read independently of the generator, straight from the lock.
locked="$(python3 -c 'import tomllib;print(next(p["version"] for p in tomllib.load(open("Cargo.lock","rb"))["package"] if p["name"]=="whatsapp-rust"))')"
grep -qF "| whatsapp-rust | $locked |" "$FILE" \
    || fail "$FILE does not list whatsapp-rust $locked (Cargo.lock). Fix: $FIX"

if ! diff -q "$FILE" "$fresh" >/dev/null; then
    diff -u "$FILE" "$fresh" | head -40 >&2 || true
    fail "$FILE is out of date with the dependency graph. Fix: $FIX"
fi
echo "third-party licenses ok: $count crates, whatsapp-rust $locked"
