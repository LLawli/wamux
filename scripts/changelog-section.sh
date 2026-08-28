#!/usr/bin/env bash
# Print the CHANGELOG section for one version, for a release body.
#
# Fails loudly when the section is missing or empty. A release whose notes are
# blank because an awk range silently matched nothing is worse than no release:
# it looks finished and tells nobody what changed.
#
# Usage: scripts/changelog-section.sh 0.1.0
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:?usage: changelog-section.sh <version>}"

section=$(awk -v ver="## [$VERSION]" '
  index($0, ver) == 1 { inside = 1; next }
  inside && /^## / { exit }
  inside { print }
' CHANGELOG.md)

# Strip leading/trailing blank lines, then insist there is prose left.
section=$(printf '%s\n' "$section" | sed -e '/./,$!d' -e :a -e '/^\n*$/{$d;N;ba' -e '}')

if [[ -z "${section// /}" ]]; then
  echo "ERROR: no content under '## [$VERSION]' in CHANGELOG.md" >&2
  echo "Sections present:" >&2
  grep -E '^## \[' CHANGELOG.md >&2 || true
  exit 1
fi

printf '%s\n' "$section"
