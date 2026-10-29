#!/usr/bin/env bash
# Install the macOS PKG on this machine and prove it works: both slices run,
# the signature is valid, the docs landed, and the installed daemon starts,
# serves a socket and stops clean on SIGTERM.
#
# macOS only, and it installs into /, so run it on a disposable machine (CI).
# Written for the system bash 3.2: no mapfile, no associative arrays.
#
# Usage: scripts/check-macos-pkg.sh dist/wamux-0.1.0-macos-universal.pkg
set -euo pipefail

PKG="${1:?usage: check-macos-pkg.sh <pkg>}"
IDENTIFIER="io.github.llawli.wamux"
BINARY="/usr/local/bin/wamux"
SHARE="/usr/local/share/wamux"
# Cold start of a release build plus SQLite migrations on a busy runner.
SOCKET_WAIT_SECONDS=60
STOP_WAIT_SECONDS=30

# Set once the daemon test starts; the EXIT trap removes the directory and stops
# a daemon a failed check left running.
WORKDIR=""
DAEMON_PID=""
cleanup() {
  [[ -z "$DAEMON_PID" ]] || kill -KILL "$DAEMON_PID" 2>/dev/null || true
  [[ -z "$WORKDIR" ]] || rm -rf "$WORKDIR"
}
trap cleanup EXIT

fail() { echo "ERROR: $*" >&2; exit 1; }

# $1 is a command and its arguments, run with output kept for the failure message.
expect_ok() {
  local description="$1"; shift
  local output
  output=$("$@" 2>&1) || fail "$description: '$*' failed. Output: $output"
}

install_pkg() {
  [[ "$(uname -s)" == "Darwin" ]] || fail "this checks a macOS installer and needs macOS (found $(uname -s))"
  [[ -f "$PKG" ]] || fail "no such pkg: $PKG"
  sudo installer -pkg "$PKG" -target / || fail "installer rejected $PKG"
  expect_ok "the pkg receipt" pkgutil --pkg-info "$IDENTIFIER"
}

check_universal_binary() {
  [[ -x "$BINARY" ]] || fail "$BINARY is missing or not executable after the install"
  local archs
  archs=$(lipo -archs "$BINARY")
  for arch in x86_64 arm64; do
    case " $archs " in
      *" $arch "*) ;;
      *) fail "$BINARY has slices '$archs', expected both x86_64 and arm64" ;;
    esac
  done
  expect_ok "the ad-hoc signature" codesign -v "$BINARY"
}

# Rosetta is not on a fresh machine, and without it the x86_64 slice cannot run.
ensure_rosetta() {
  arch -x86_64 /usr/bin/true >/dev/null 2>&1 && return 0
  echo "installing Rosetta to run the x86_64 slice"
  sudo softwareupdate --install-rosetta --agree-to-license || fail "could not install Rosetta"
  arch -x86_64 /usr/bin/true || fail "x86_64 binaries still do not run after installing Rosetta"
}

check_each_slice_runs() {
  ensure_rosetta
  local arch output
  for arch in arm64 x86_64; do
    output=$(arch "-$arch" "$BINARY" --help 2>&1) || fail "$arch slice: '$BINARY --help' failed. Output: $output"
    case "$output" in
      *"usage:"*) ;;
      *) fail "$arch slice: --help printed no usage text. Output: $output" ;;
    esac
  done
}

check_docs_installed() {
  local file
  for file in README.md CHANGELOG.md LICENSE-MIT LICENSE-APACHE THIRD-PARTY-LICENSES.md \
    wamux.toml.example docs/DEPLOYMENT.md "contrib/${IDENTIFIER}.plist"; do
    [[ -s "$SHARE/$file" ]] || fail "$SHARE/$file is missing or empty, expected it in the pkg payload"
  done
  expect_ok "the example LaunchAgent" plutil -lint "$SHARE/contrib/${IDENTIFIER}.plist"
}

# An exited child stays a zombie until `wait`, and `kill -0` still succeeds on
# a zombie, so liveness is read from the process state instead.
daemon_alive() {
  local state
  state=$(ps -o stat= -p "$1" 2>/dev/null || true)
  [[ -n "$state" && "$state" != Z* ]]
}

# The log is shown only when something went wrong.
wait_for_socket() {
  local pid="$1" socket="$2" log="$3" waited=0
  while [[ ! -S "$socket" ]]; do
    daemon_alive "$pid" || fail "the daemon exited before it bound $socket. Log: $(cat "$log")"
    (( waited < SOCKET_WAIT_SECONDS )) || fail "no socket at $socket after ${SOCKET_WAIT_SECONDS}s. Log: $(cat "$log")"
    sleep 1
    waited=$((waited + 1))
  done
}

# bash 3.2 has no `wait` with a timeout: a watchdog SIGKILLs a daemon that
# ignores SIGTERM, and the status 137 (128 + 9) tells that case apart.
stop_and_expect_clean_exit() {
  local pid="$1" socket="$2" log="$3" status=0 watchdog
  kill -TERM "$pid"
  ( sleep "$STOP_WAIT_SECONDS"; kill -KILL "$pid" 2>/dev/null ) >/dev/null 2>&1 &
  watchdog=$!
  wait "$pid" || status=$?
  kill "$watchdog" 2>/dev/null || true
  (( status != 137 )) || fail "the daemon ignored SIGTERM for ${STOP_WAIT_SECONDS}s. Log: $(cat "$log")"
  (( status == 0 )) || fail "the daemon exited with status $status after SIGTERM, expected 0. Log: $(cat "$log")"
  [[ ! -e "$socket" ]] || fail "the daemon left $socket behind after SIGTERM"
}

# /tmp, not $TMPDIR: a macOS socket path is limited to 104 bytes and $TMPDIR
# under /var/folders is already about 50.
check_daemon_lifecycle() {
  local dir socket log pid
  WORKDIR=$(mktemp -d /tmp/wamux.XXXXXX)
  dir="$WORKDIR"
  socket="$dir/wamux.sock"
  log="$dir/daemon.log"
  WAMUX_DATABASE_URL="sqlite://$dir/wamux.db" WAMUX_SOCKET_PATH="$socket" \
    WAMUX_MEDIA_SPOOL_DIR="$dir/spool" NO_COLOR=1 "$BINARY" >"$log" 2>&1 &
  pid=$!
  DAEMON_PID="$pid"
  wait_for_socket "$pid" "$socket" "$log"
  stop_and_expect_clean_exit "$pid" "$socket" "$log"
  DAEMON_PID=""
}

install_pkg
check_universal_binary
check_each_slice_runs
check_docs_installed
check_daemon_lifecycle
echo "macos pkg ok: $PKG"
