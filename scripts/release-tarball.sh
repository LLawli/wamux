#!/usr/bin/env bash
# Build a release tarball: one binary per platform, built once.
#
# The musl target (default) is the static binary the tarball, the scratch image
# and the Debian package share (#218). aarch64-linux-android is the Termux build
# (#277): bionic, not musl, because a static musl binary cannot resolve names on
# Android. Each target keeps its own binary path so one run does not clobber the
# other's.
#
# The archive is meant to be self-sufficient: the binary alone is not a
# distribution. It carries the license texts (the MIT/Apache terms of ~356
# linked crates require their notices to travel with the binary), the service
# definition for the platform, the config example, and the deployment doc.
#
# Usage: scripts/release-tarball.sh 0.1.0 [outdir] [target]
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="${1:?usage: release-tarball.sh <version> [outdir] [target]}"
OUTDIR="${2:-dist}"
# Not the host triple: the shipped artifact is the static musl binary (#218),
# the same file the scratch image carries, whatever machine builds it.
TARGET="${3:-x86_64-unknown-linux-musl}"
NAME="wamux-${VERSION}-${TARGET}"

# The manifest is the source of truth for the version; a tag that disagrees
# with it would ship an artifact whose --version lies about what it is.
manifest_version=$(awk '/^\[workspace\.package\]/{p=1} p && /^version = /{gsub(/[",]/,"",$3); print $3; exit}' Cargo.toml)
if [[ "$manifest_version" != "$VERSION" ]]; then
  echo "ERROR: version mismatch - argument '$VERSION', workspace Cargo.toml '$manifest_version'" >&2
  exit 1
fi

# Sets BINARY (where the stripped binary is kept), STRIP and CARGO_FEATURES.
configure_target() {
  CARGO_FEATURES=()
  case "$TARGET" in
    x86_64-unknown-linux-musl)
      # Stripped once, at a fixed path: the workflow checks this file is static,
      # uploads it, and the image job compares the image's binary to it.
      BINARY="${OUTDIR}/bin/wamux"
      STRIP=strip
      ;;
    aarch64-linux-android)
      # The defaults of a Termux build live behind the `termux` feature (#277).
      # The host strip cannot read aarch64, so the NDK's does the stripping.
      # shellcheck source=scripts/android-env.sh
      source scripts/android-env.sh
      use_android_ndk
      BINARY="${OUTDIR}/bin/${TARGET}/wamux"
      STRIP="$ANDROID_LLVM_STRIP"
      CARGO_FEATURES=(--features termux)
      ;;
    *)
      echo "ERROR: unsupported target '$TARGET' (x86_64-unknown-linux-musl or aarch64-linux-android)" >&2
      exit 1
      ;;
  esac
}

# What goes in the archive besides the binary, licenses and docs.
stage_platform_files() {
  local stage="$1"
  if [[ "$TARGET" == aarch64-linux-android ]]; then
    mkdir -p "${stage}/contrib/termux/log"
    install -m 0755 contrib/termux/run "${stage}/contrib/termux/"
    install -m 0755 contrib/termux/log/run "${stage}/contrib/termux/log/"
    install -m 0644 docs/TERMUX.md "${stage}/docs/"
    return
  fi
  install -m 0644 contrib/wamux.service "${stage}/contrib/"
  install -m 0644 docs/DEPLOYMENT.md "${stage}/docs/"
}

configure_target

echo "building wamux $VERSION for $TARGET"
# --locked: the release must build the dependency tree CI tested, so a stale
# Cargo.lock fails here instead of shipping unreviewed versions (#78).
cargo build --release --locked --target "$TARGET" -p wamux --bin wamux "${CARGO_FEATURES[@]}"

rm -rf "${OUTDIR:?}/${NAME}"
mkdir -p "${OUTDIR}/${NAME}/contrib" "${OUTDIR}/${NAME}/docs" "$(dirname "$BINARY")"

# Symbols are ~12MB and nothing consumes them in a release artifact.
install -m 0755 "target/${TARGET}/release/wamux" "$BINARY"
"$STRIP" "$BINARY"
install -m 0755 "$BINARY" "${OUTDIR}/${NAME}/wamux"

install -m 0644 README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE \
  THIRD-PARTY-LICENSES.md wamux.toml.example "${OUTDIR}/${NAME}/"
stage_platform_files "${OUTDIR}/${NAME}"

tar -C "$OUTDIR" -czf "${OUTDIR}/${NAME}.tar.gz" "$NAME"
rm -rf "${OUTDIR:?}/${NAME}"

( cd "$OUTDIR" && sha256sum "${NAME}.tar.gz" > "${NAME}.tar.gz.sha256" )

echo
ls -la "${OUTDIR}/${NAME}.tar.gz" "${OUTDIR}/${NAME}.tar.gz.sha256"
cat "${OUTDIR}/${NAME}.tar.gz.sha256"
