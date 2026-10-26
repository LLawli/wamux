#!/usr/bin/env bash
# Build the release tarball: one static musl binary.
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
# Not the host triple: the shipped artifact is the static musl binary (#218),
# the same file the scratch image carries, whatever machine builds it.
TARGET="x86_64-unknown-linux-musl"
NAME="wamux-${VERSION}-${TARGET}"

# The manifest is the source of truth for the version; a tag that disagrees
# with it would ship an artifact whose --version lies about what it is.
manifest_version=$(awk '/^\[workspace\.package\]/{p=1} p && /^version = /{gsub(/[",]/,"",$3); print $3; exit}' Cargo.toml)
if [[ "$manifest_version" != "$VERSION" ]]; then
  echo "ERROR: version mismatch - argument '$VERSION', workspace Cargo.toml '$manifest_version'" >&2
  exit 1
fi

echo "building wamux $VERSION for $TARGET"
# --locked: the release must build the dependency tree CI tested, so a stale
# Cargo.lock fails here instead of shipping unreviewed versions (#78).
cargo build --release --locked --target "$TARGET" -p wamux --bin wamux

rm -rf "${OUTDIR:?}/${NAME}"
mkdir -p "${OUTDIR}/${NAME}/contrib" "${OUTDIR}/${NAME}/docs"

# Stripped once, at a fixed path: the workflow checks this file is static,
# uploads it, and the image job compares the image's binary to it. Symbols are
# ~12MB and nothing consumes them in a release artifact.
mkdir -p "${OUTDIR}/bin"
install -m 0755 "target/${TARGET}/release/wamux" "${OUTDIR}/bin/wamux"
strip "${OUTDIR}/bin/wamux"
install -m 0755 "${OUTDIR}/bin/wamux" "${OUTDIR}/${NAME}/wamux"

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
