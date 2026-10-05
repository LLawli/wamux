#!/usr/bin/env bash
# wamux local CI: every quality gate in one unattended run. Exits non-zero on
# the first failing gate. GitHub Actions invokes THIS script rather than
# restating the stages in YAML, so there is exactly one definition of "green".
#
# Usage:
#   scripts/ci.sh                # fmt + clippy (default, stress & turso) + tests + fast stress tests
#   scripts/ci.sh --full         # also the #[ignore] scale tests (load, keepalive, M3)
#   scripts/ci.sh --no-postgres  # only the gates that need no database
#
# Env:
#   DATABASE_URL     (default postgres://wamux:wamux@localhost:5433/wamux)
#   STRESS_ACCOUNTS  M3 connection count in --full (default 199)
set -euo pipefail
cd "$(dirname "$0")/.."

DATABASE_URL="${DATABASE_URL:-postgres://wamux:wamux@localhost:5433/wamux}"
export DATABASE_URL
FULL=0
NO_POSTGRES=0
for arg in "$@"; do
  case "$arg" in
    --full) FULL=1 ;;
    --no-postgres) NO_POSTGRES=1 ;;
    *) echo "unknown flag: $arg (expected --full or --no-postgres)" >&2; exit 2 ;;
  esac
done
if [[ "$FULL" == 1 && "$NO_POSTGRES" == 1 ]]; then
  echo "ERROR: --full needs Postgres (the scale tests are database-backed)." >&2
  exit 2
fi

stage() { printf '\n\033[1m== %s ==\033[0m\n' "$*"; }

# Name-filtered gates pass vacuously if the filter matches zero tests (cargo
# exits 0 on "running 0 tests"), so a renamed/moved test would silently kill
# the gate forever. Assert tests actually ran.
must_run_pkg_tests() {
  local pkg="$1"; shift
  local out
  out=$(cargo test -p "$pkg" "$@" 2>&1) || { printf '%s\n' "$out"; return 1; }
  printf '%s\n' "$out"
  if grep -qE '^running 0 tests$' <<<"$out"; then
    echo "ERROR: gate ran 0 tests (renamed/moved?): cargo test -p $pkg $*" >&2
    return 1
  fi
}
must_run_tests() { must_run_pkg_tests wamux "$@"; }

# The Turso engine (#106) is behind the daemon's off-by-default `turso`
# feature, so its tests exist only in a `--features turso` build. Its storage
# cases name the engine (`turso_`, `turso_and_sqlite_`) and need no database;
# the unit tests are the `$N` rewrite, the DSN and the scheme dispatch.
turso_storage_cases() {
  must_run_tests --features turso --lib storage::
  must_run_tests --features turso --test storage_backend turso_
  must_run_tests --features turso --test bincode_upgrade turso_
  must_run_tests --features turso --test store_parity turso_
  must_run_tests --features turso --test existing_store turso_
}

# wamux-tools (#64): the shared client, the env contract, the exit code, and
# the built binaries refusing a bad config. Every suite but inproc runs its
# daemon fixture on SQLite; inproc is Postgres-backed like the bins it serves.
TOOLS_NO_PG_SUITES=(--test live_env --test report --test delivery --test media_qr
  --test socket_client --test bin_contract --test runbook)

# Fail fast with an actionable message if Postgres isn't reachable (tests need it).
# Parse host:port from any valid URL shape: strip the scheme, then optional
# userinfo, then the path; a portless URL implies Postgres' default 5432.
if [[ "$NO_POSTGRES" == 0 ]]; then
  pg_rest="${DATABASE_URL#*://}"; pg_rest="${pg_rest##*@}"
  pg_host_port="${pg_rest%%/*}"
  pg_host="${pg_host_port%%:*}"
  if [[ "$pg_host_port" == *:* ]]; then pg_port="${pg_host_port##*:}"; else pg_port=5432; fi
  if ! (exec 3<>"/dev/tcp/${pg_host}/${pg_port}") 2>/dev/null; then
    echo "ERROR: Postgres unreachable at ${pg_host}:${pg_port}." >&2
    echo "Start it with: docker run -d --name wamux-pg -e POSTGRES_USER=wamux \\" >&2
    echo "  -e POSTGRES_PASSWORD=wamux -e POSTGRES_DB=wamux -p 5433:5432 postgres:16" >&2
    echo "Or run the database-free subset: scripts/ci.sh --no-postgres" >&2
    exit 1
  fi
fi

stage "fmt --check"
cargo fmt --all --check

stage "clippy (default)"
cargo clippy --workspace --all-targets -- -D warnings

# The stress feature lives on two members: the mock in wamux, the stress_live
# probe in wamux-tools. Both are switched on so neither goes unlinted.
stage "clippy (--features stress)"
cargo clippy --workspace --all-targets --features wamux/stress,wamux-tools/stress -- -D warnings

# #106: the Turso family compiles only with its feature on; lint it too.
stage "clippy (--features turso)"
cargo clippy --workspace --all-targets --features wamux/turso -- -D warnings

stage "no duplicate gRPC/HTTP crates"
scripts/check-dup-deps.sh

stage "crate boundaries (daemon and wamux-proto)"
scripts/check-crate-deps.sh

# THIRD-PARTY-LICENSES.md ships in the image and the tarball, and the MIT and
# Apache-2.0 terms of the crates it lists require their notices to travel with
# the binary (#83). It went stale for every whatsapp-rust bump until this
# stage: regenerate from the shipped daemon's graph and fail on any drift.
# Needs no database, so both modes run it.
stage "third-party licenses"
out=$(python3 -m unittest discover -s scripts/tests -v 2>&1) || { printf '%s\n' "$out"; exit 1; }
printf '%s\n' "$out"
if ! grep -qE '^Ran [1-9][0-9]* tests? in ' <<<"$out"; then
  echo "ERROR: scripts/tests ran no tests" >&2
  exit 1
fi
scripts/check-third-party.sh

# #60: a store method no test calls is a hole the storage rewrite (#65) would cross
# with no net. Pure text check over the sources, so both modes run it.
stage "store method coverage"
scripts/check-store-coverage.py

# #93: a wacore trait default nobody classified is behavior the engines inherit
# unseen. A whatsapp-rust bump that adds one fails here until it is classified in
# the store defaults doc. Needs cargo (metadata, offline), not the database.
stage "store trait defaults"
scripts/check-store-defaults.py

# #106: two engine families run the same statements, so they are written once
# in storage/statements/ and never inline in a family. Pure text check.
stage "store SQL written once"
scripts/check-store-sql-shared.py

# #67: a sleep in a test is a synchronization bug unless it says why it is not
# one. Pure text check over the sources, so both modes run it.
stage "test sleeps are marked"
scripts/check-test-sleeps.py

# #68: an RPC of GroupService no socket test calls is an RPC nobody pinned. The
# count (21) makes a new RPC fail here until it gets a test. Pure text check.
stage "GroupService RPC coverage"
scripts/check-service-coverage.py GroupService crates/wamux/tests/group_service 21

# #69: same net under MediaService, whose one RPC is DownloadMedia.
stage "MediaService RPC coverage"
scripts/check-service-coverage.py MediaService crates/wamux/tests/media_service.rs 1

# #70: same net under NewsletterService (6 RPCs).
stage "NewsletterService RPC coverage"
scripts/check-service-coverage.py NewsletterService crates/wamux/tests/newsletter_service 6

# #71: same net under MessagingService (23 RPCs).
stage "MessagingService RPC coverage"
scripts/check-service-coverage.py MessagingService crates/wamux/tests/messaging_service 23

if [[ "$NO_POSTGRES" == 1 ]]; then
  # The database-free subset. NOT the whole suite with a flag: storage_backend
  # deliberately keeps Postgres-backed cases (engine parity is only provable
  # with both engines present), and the stress suite is database-backed too.
  # Those are the full run's job; this one exists for fast PR feedback and for
  # a machine with no container runtime.
  stage "no-postgres: unit tests"
  must_run_tests --lib

  stage "no-postgres: service suites (sqlite engine)"
  WAMUX_TEST_ENGINE=sqlite must_run_tests --test grpc_server --test event_subscription \
    --test reflection

  stage "no-postgres: sqlite-only storage cases"
  must_run_tests --test storage_backend sqlite_
  must_run_tests --test bincode_upgrade sqlite_
  must_run_tests --test store_parity sqlite_
  # #65: a store the pre-unification code wrote, opened on the unified SQL.
  must_run_tests --test existing_store sqlite_

  stage "no-postgres: turso engine (storage cases and service suites)"
  turso_storage_cases
  WAMUX_TEST_ENGINE=turso must_run_tests --features turso --test grpc_server \
    --test event_subscription --test reflection

  stage "no-postgres: wamux-tools (sqlite daemon fixture)"
  must_run_pkg_tests wamux-tools "${TOOLS_NO_PG_SUITES[@]}"

  stage "CI PASSED (no-postgres subset)"
  exit 0
fi

# #67: remember what the shared database holds before any test touches it, so
# the last stage can prove the run left nothing behind. Not "the table is
# empty": the database is also the dev one and may hold rows of its own.
ACCOUNT_SNAPSHOT="$(mktemp)"
export WAMUX_ACCOUNT_SNAPSHOT="$ACCOUNT_SNAPSHOT"
trap 'rm -f "$ACCOUNT_SNAPSHOT"' EXIT
stage "account snapshot (before the first database test)"
must_run_tests --test account_leftovers -- --ignored --exact snapshot_accounts_before_the_run

# The root manifest is virtual with default-members = wamux, so a bare `cargo
# test` would only test the daemon. The daemon's stages say `-p wamux` and the
# tools' stage says `-p wamux-tools` (#64); wamux-proto has no tests and is
# still compiled by the clippy --workspace stages above.
stage "tests (unit + integration, postgres engine)"
cargo test -p wamux

# Same suite, SQLite engine. The service-level suites honor WAMUX_TEST_ENGINE,
# so this re-runs them against the other backend. Convention (#67): a test that
# names its engine in its own name (prefix `postgres_`, `sqlite_` or
# `both_engines_`) builds that engine itself, so it already ran in the pass
# above, with both engines available, and is skipped here. `--skip` matches
# substrings, so no other test may contain those fragments in its name (the
# gate for #67 checks the skip removes exactly the prefixed tests).
stage "tests (sqlite engine)"
WAMUX_TEST_ENGINE=sqlite cargo test -p wamux -- --skip postgres_ --skip sqlite_ --skip both_engines_

# Same suite, Turso engine (#106), in a `--features turso` build. The tests
# that name an engine are skipped as above, `turso_` included: those are the
# storage cases, run by name right before.
stage "tests (turso engine)"
turso_storage_cases
WAMUX_TEST_ENGINE=turso cargo test -p wamux --features turso -- --skip postgres_ \
  --skip sqlite_ --skip both_engines_ --skip turso_

stage "tests (wamux-tools)"
must_run_pkg_tests wamux-tools --test '*'

# Every stress stage runs on both engines (#67): the registry under test is
# built from WAMUX_TEST_ENGINE, the mock stays the same.
stage "stress tests (fast: M1/M2a/M2b)"
cargo test -p wamux --features stress --test stress_handshake
WAMUX_TEST_ENGINE=sqlite cargo test -p wamux --features stress --test stress_handshake

# Channel metadata through the real library, against answers the mock plays
# back (#56): the relayed tokens, NotFound, the list skip, the MEX refusal, and
# canaries on the two losses accepted when the core stopped querying itself.
stage "stress tests (newsletter metadata)"
must_run_tests --features stress --test stress_newsletter_parse
WAMUX_TEST_ENGINE=sqlite must_run_tests --features stress --test stress_newsletter_parse

# Channel history through the real library over captured pages (#56): every
# edge case #40, #43, #44 and #51 pinned, asserted by value.
stage "stress tests (newsletter history)"
must_run_tests --features stress --test stress_newsletter_history
WAMUX_TEST_ENGINE=sqlite must_run_tests --features stress --test stress_newsletter_history

# The channel poll vote, own add-ons and live-update subscription (#26): the
# stanza a vote puts on the wire, and the captured answers the reads relay.
stage "stress tests (newsletter poll vote)"
must_run_tests --features stress --test stress_newsletter_poll_vote
WAMUX_TEST_ENGINE=sqlite must_run_tests --features stress --test stress_newsletter_poll_vote

# GroupService through the socket (#68): the five reads replay answers captured
# live, the writes are built in the shape the library's parser accepts.
stage "stress tests (group service)"
must_run_tests --features stress --test group_service
WAMUX_TEST_ENGINE=sqlite must_run_tests --features stress --test group_service

# MediaService through the socket (#69): DownloadMedia against a loopback CDN,
# reached by the production ureq client behind a loopback-only https rewrite.
stage "stress tests (media service)"
must_run_tests --features stress --test media_service
WAMUX_TEST_ENGINE=sqlite must_run_tests --features stress --test media_service

# NewsletterService through the socket (#70): the five reads replay answers
# captured live, the vote is checked against the stanza measured in #26.
stage "stress tests (newsletter service)"
must_run_tests --features stress --test newsletter_service
WAMUX_TEST_ENGINE=sqlite must_run_tests --features stress --test newsletter_service

# MessagingService through the socket (#71): every send is opened by a parked
# Signal peer the mock serves, and chat actions are read back from the patch.
stage "stress tests (messaging service)"
must_run_tests --features stress --test messaging_service
WAMUX_TEST_ENGINE=sqlite must_run_tests --features stress --test messaging_service

if [[ "$FULL" == 1 ]]; then
  stage "FULL: load test (HOL blocking + gap)"
  must_run_tests --test load_multi_account -- --ignored
  WAMUX_TEST_ENGINE=sqlite must_run_tests --test load_multi_account -- --ignored

  stage "FULL: keepalive longevity (~25s)"
  must_run_tests --features stress --test stress_handshake \
    connection_survives_keepalive_window -- --ignored

  stage "FULL: M3 scale (${STRESS_ACCOUNTS:-199} clients vs mock)"
  must_run_tests --features stress --test stress_handshake \
    connect_many_clients_against_mock -- --ignored
  WAMUX_TEST_ENGINE=sqlite must_run_tests --features stress --test stress_handshake \
    connect_many_clients_against_mock -- --ignored
fi

# #67: last, so it sees everything the run created. Fails on any account or
# throwaway database that did not exist when the snapshot was taken.
stage "no account outlives the run"
must_run_tests --test account_leftovers -- --ignored --exact no_account_outlives_the_run

if [[ "$FULL" == 1 ]]; then stage "CI PASSED (full)"; else stage "CI PASSED"; fi
