#!/usr/bin/env bash
# Build the release tarball for the current host target.
#
# The archive is meant to be self-sufficient: the binary alone is not a
# distribution. It carries the license texts (the MIT/Apache terms of ~356
# linked crates require their notices to travel with the binary), the systemd
# unit, the config example, and the deployment doc.
#
# Usage: scripts/release-tarball.sh 0.1.0 [outdir]
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:?usage: release-tarball.sh <version> [outdir]}"
OUTDIR="${2:-dist}"
TARGET="$(rustc -vV | awk '/^host:/ {print $2}')"
NAME="wamux-${VERSION}-${TARGET}"

# The manifest is the source of truth for the version; a tag that disagrees
# with it would ship an artifact whose --version lies about what it is.
manifest_version=$(awk '/^\[package\]/{p=1} p && /^version = /{gsub(/[",]/,"",$3); print $3; exit}' Cargo.toml)
if [[ "$manifest_version" != "$VERSION" ]]; then
  echo "ERROR: version mismatch - argument '$VERSION', Cargo.toml '$manifest_version'" >&2
  exit 1
fi

echo "building wamux $VERSION for $TARGET"
cargo build --release --bin wamux

rm -rf "${OUTDIR:?}/${NAME}"
mkdir -p "${OUTDIR}/${NAME}/contrib" "${OUTDIR}/${NAME}/docs"

install -m 0755 target/release/wamux "${OUTDIR}/${NAME}/wamux"
# Symbols are ~12MB of the binary and nothing consumes them in a release
# artifact; debug info is already absent.
strip "${OUTDIR}/${NAME}/wamux"

install -m 0644 README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE \
  THIRD-PARTY-LICENSES.md wamux.toml.example "${OUTDIR}/${NAME}/"
install -m 0644 contrib/wamux.service "${OUTDIR}/${NAME}/contrib/"
install -m 0644 docs/DEPLOYMENT.md "${OUTDIR}/${NAME}/docs/"

tar -C "$OUTDIR" -czf "${OUTDIR}/${NAME}.tar.gz" "$NAME"
rm -rf "${OUTDIR:?}/${NAME}"

( cd "$OUTDIR" && sha256sum "${NAME}.tar.gz" > "${NAME}.tar.gz.sha256" )

echo
ls -la "${OUTDIR}/${NAME}.tar.gz" "${OUTDIR}/${NAME}.tar.gz.sha256"
cat "${OUTDIR}/${NAME}.tar.gz.sha256"
