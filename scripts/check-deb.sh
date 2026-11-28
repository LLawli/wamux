#!/usr/bin/env bash
# Check a .deb built by scripts/build-deb.py with dpkg-deb, the tool that will
# read it on a user's machine (#277): the control fields, the file list, and
# that the packaged binary is byte-identical to the one the release job built
# and checked (compile once, package many). The debian flavor also installs the
# package on this machine and runs the installed binary, then removes it, so a
# package that does not run is caught before anyone downloads it.
#
# The install touches the system (apt-get, /usr/bin), so run it on a disposable
# machine (CI). The termux flavor is only inspected: its binary is aarch64 and
# its paths are Termux's, so it cannot be installed on a runner.
#
# Usage: scripts/check-deb.sh <deb> <debian|termux> <binary>
set -euo pipefail

DEB="${1:?usage: check-deb.sh <deb> <debian|termux> <binary>}"
FLAVOR="${2:?usage: check-deb.sh <deb> <debian|termux> <binary>}"
BINARY="${3:?usage: check-deb.sh <deb> <debian|termux> <binary>}"

fail() { echo "ERROR: $*" >&2; exit 1; }

case "$FLAVOR" in
  debian) ARCH=amd64; PREFIX=usr ;;
  termux) ARCH=aarch64; PREFIX=data/data/com.termux/files/usr ;;
  *) fail "unknown flavor '$FLAVOR' (debian or termux)" ;;
esac

WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

check_control() {
  local info arch
  info=$(dpkg-deb --info "$DEB")
  echo "$info"
  arch=$(dpkg-deb --field "$DEB" Architecture)
  [[ "$arch" == "$ARCH" ]] || fail "Architecture is '$arch', expected '$ARCH'"
  [[ "$(dpkg-deb --field "$DEB" Package)" == wamux ]] || fail "Package is not wamux"
}

check_contents() {
  local contents path
  contents=$(dpkg-deb --contents "$DEB")
  echo "$contents"
  for path in "./${PREFIX}/bin/wamux" "./${PREFIX}/share/doc/wamux/README.md"; do
    grep -qF " ${path}" <<<"$contents" || fail "the package lists no ${path}"
  done
}

# Byte for byte: a repackaged or re-stripped binary would no longer be the one
# the attested tarball carries.
check_binary_bytes() {
  dpkg-deb --extract "$DEB" "$WORKDIR/root"
  cmp "$WORKDIR/root/${PREFIX}/bin/wamux" "$BINARY" \
    || fail "the packaged binary differs from $BINARY"
  [[ -x "$WORKDIR/root/${PREFIX}/bin/wamux" ]] || fail "the packaged binary is not executable"
  echo "binary matches $BINARY"
}

check_install_and_run() {
  # apt reads a local file only when the argument is a path, hence the absolute one.
  sudo apt-get install -y "$(realpath "$DEB")"
  wamux --help >/dev/null || fail "the installed wamux --help failed"
  test -f /usr/lib/systemd/user/wamux.service || fail "the systemd user unit is not installed"
  sudo apt-get remove -y wamux
  # The file, not `command -v`: bash keeps the path it ran above in its hash
  # table and would still report it after the removal.
  [[ ! -e /usr/bin/wamux ]] || fail "/usr/bin/wamux is still there after removal"
  echo "install, run and remove ok"
}

check_control
check_contents
check_binary_bytes
[[ "$FLAVOR" != debian ]] || check_install_and_run
echo "deb ok: $DEB ($FLAVOR)"
