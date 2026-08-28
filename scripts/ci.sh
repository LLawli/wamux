#!/usr/bin/env bash
# wamux local CI: every quality gate in one unattended run. Exits non-zero on
# the first failing gate. GitHub Actions invokes THIS script rather than
# restating the stages in YAML, so there is exactly one definition of "green".
#
# Usage:
#   scripts/ci.sh                # fmt + clippy (default & stress) + tests + fast stress tests
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
must_run_tests() {
  local out
  out=$(cargo test "$@" 2>&1) || { printf '%s\n' "$out"; return 1; }
  printf '%s\n' "$out"
  if grep -qE '^running 0 tests$' <<<"$out"; then
    echo "ERROR: gate ran 0 tests (renamed/moved?): cargo test $*" >&2
    return 1
  fi
}

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
cargo fmt --check

stage "clippy (default)"
cargo clippy --all-targets -- -D warnings

stage "clippy (--features stress)"
cargo clippy --features stress --all-targets -- -D warnings

if [[ "$NO_POSTGRES" == 1 ]]; then
  # The database-free subset. NOT the whole suite with a flag: storage_backend
  # deliberately keeps Postgres-backed cases (engine parity is only provable
  # with both engines present), and the stress suite is database-backed too.
  # Those are the full run's job; this one exists for fast PR feedback and for
  # a machine with no container runtime.
  stage "no-postgres: unit tests"
  must_run_tests --lib

  stage "no-postgres: service suites (sqlite engine)"
  WAMUX_TEST_ENGINE=sqlite must_run_tests --test grpc_server --test event_subscription

  stage "no-postgres: sqlite-only storage cases"
  must_run_tests --test storage_backend sqlite_

  stage "CI PASSED (no-postgres subset)"
  exit 0
fi

stage "tests (unit + integration, postgres engine)"
cargo test

# Same suite, SQLite engine. The service-level suites honor WAMUX_TEST_ENGINE,
# so this re-runs them against the other backend; storage_backend.rs exercises
# both engines in either pass (parity is only testable with both present).
stage "tests (sqlite engine)"
WAMUX_TEST_ENGINE=sqlite cargo test

stage "stress tests (fast: M1/M2a/M2b)"
cargo test --features stress --test stress_handshake

if [[ "$FULL" == 1 ]]; then
  stage "FULL: load test (HOL blocking + gap)"
  must_run_tests --test load_multi_account -- --ignored

  stage "FULL: keepalive longevity (~25s)"
  must_run_tests --features stress --test stress_handshake \
    connection_survives_keepalive_window -- --ignored

  stage "FULL: M3 scale (${STRESS_ACCOUNTS:-199} clients vs mock)"
  must_run_tests --features stress --test stress_handshake \
    connect_many_clients_against_mock -- --ignored
fi

if [[ "$FULL" == 1 ]]; then stage "CI PASSED (full)"; else stage "CI PASSED"; fi
