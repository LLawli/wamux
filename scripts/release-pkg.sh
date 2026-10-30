#!/usr/bin/env bash
# Build the release PKG: one universal (arm64 + x86_64) macOS installer.
#
# macOS only: lipo, pkgbuild and codesign ship with the system and nothing
# here cross-compiles. The PKG is unsigned and not notarized (a deliberate
# choice, see README.md for the Gatekeeper path) and it starts nothing: it
# installs /usr/local/bin/wamux plus docs, licenses and an example LaunchAgent
# under /usr/local/share/wamux/.
#
# Usage: scripts/release-pkg.sh 0.1.0 [outdir]
# Writes <outdir>/wamux-<version>-macos-universal.pkg and its .sha256.
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:?usage: release-pkg.sh <version> [outdir]}"
OUTDIR="${2:-dist}"
IDENTIFIER="io.github.llawli.wamux"
PLIST="contrib/${IDENTIFIER}.plist"
PKG_NAME="wamux-${VERSION}-macos-universal.pkg"
SHARE="usr/local/share/wamux"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "ERROR: release-pkg.sh builds a macOS installer and needs macOS (found $(uname -s))" >&2
  exit 1
fi

# The manifest is the source of truth for the version; a tag that disagrees
# with it would ship an artifact whose --version lies about what it is.
manifest_version=$(awk '/^\[workspace\.package\]/{p=1} p && /^version = /{gsub(/[",]/,"",$3); print $3; exit}' Cargo.toml)
if [[ "$manifest_version" != "$VERSION" ]]; then
  echo "ERROR: version mismatch - argument '$VERSION', workspace Cargo.toml '$manifest_version'" >&2
  exit 1
fi

# --locked: the release must build the dependency tree CI tested, so a stale
# Cargo.lock fails here instead of shipping unreviewed versions (#78).
build_slices() {
  local target
  for target in aarch64-apple-darwin x86_64-apple-darwin; do
    echo "building wamux $VERSION for $target"
    # Empty, not unset: cargo then ignores the dev-only `-Zthreads` in
    # .cargo/config.toml, so the attested binary is built as before (#233).
    RUSTFLAGS="" cargo build --release --locked --target "$target" -p wamux --bin wamux
  done
}

# Strip first, join second: lipo records each slice as it finds it. Stripping
# rewrites the Mach-O and can drop the linker's ad-hoc signature, and an arm64
# binary with no valid signature is killed by the kernel at exec, so the
# universal file is signed (ad hoc, no identity) after it is assembled.
make_universal_binary() {
  local dest="$1" target
  local slices=()
  for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cp "target/${target}/release/wamux" "${STAGE}/wamux-${target}"
    strip "${STAGE}/wamux-${target}"
    slices+=("${STAGE}/wamux-${target}")
  done
  lipo -create -output "$dest" "${slices[@]}"
  codesign --force --sign - "$dest"
}

stage_payload() {
  local root="$1"
  install -d -m 0755 "${root}/usr/local/bin" "${root}/${SHARE}/docs" "${root}/${SHARE}/contrib"
  make_universal_binary "${root}/usr/local/bin/wamux"
  chmod 0755 "${root}/usr/local/bin/wamux"
  install -m 0644 README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE \
    THIRD-PARTY-LICENSES.md wamux.toml.example "${root}/${SHARE}/"
  install -m 0644 docs/DEPLOYMENT.md "${root}/${SHARE}/docs/"
  install -m 0644 "$PLIST" "${root}/${SHARE}/contrib/"
}

build_slices
mkdir -p "$OUTDIR"
STAGE="$(mktemp -d "${TMPDIR:-/tmp}/wamux-pkg.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "${STAGE}/root"
stage_payload "${STAGE}/root"

rm -f "${OUTDIR}/${PKG_NAME}"
pkgbuild --root "${STAGE}/root" --identifier "$IDENTIFIER" --version "$VERSION" \
  --install-location / "${OUTDIR}/${PKG_NAME}"

# `shasum -a 256` prints the `sha256sum` format; run from the output directory
# so the recorded name is bare and `sha256sum -c` works wherever it is downloaded.
( cd "$OUTDIR" && shasum -a 256 "$PKG_NAME" > "${PKG_NAME}.sha256" )

echo
ls -la "${OUTDIR}/${PKG_NAME}" "${OUTDIR}/${PKG_NAME}.sha256"
cat "${OUTDIR}/${PKG_NAME}.sha256"
