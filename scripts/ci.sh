#!/usr/bin/env bash
# wamux local CI: every quality gate in one unattended run (no GitHub here —
# this script IS the pipeline). Exits non-zero on the first failing gate.
#
# Usage:
#   scripts/ci.sh           # fmt + clippy (default & stress) + tests + fast stress tests
#   scripts/ci.sh --full    # also the #[ignore] scale tests (load, keepalive, M3)
#
# Env:
#   DATABASE_URL     (default postgres://wamux:wamux@localhost:5433/wamux)
#   STRESS_ACCOUNTS  M3 connection count in --full (default 199)
set -euo pipefail
cd "$(dirname "$0")/.."

DATABASE_URL="${DATABASE_URL:-postgres://wamux:wamux@localhost:5433/wamux}"
export DATABASE_URL
FULL=0
[[ "${1:-}" == "--full" ]] && FULL=1

stage() { printf '\n\033[1m== %s ==\033[0m\n' "$*"; }

# Fail fast with an actionable message if Postgres isn't reachable (tests need it).
pg_host_port="${DATABASE_URL#*@}"; pg_host_port="${pg_host_port%%/*}"
pg_host="${pg_host_port%%:*}"; pg_port="${pg_host_port##*:}"
if ! (exec 3<>"/dev/tcp/${pg_host}/${pg_port}") 2>/dev/null; then
  echo "ERROR: Postgres unreachable at ${pg_host}:${pg_port}." >&2
  echo "Start it with: docker run -d --name wamux-pg -e POSTGRES_USER=wamux \\" >&2
  echo "  -e POSTGRES_PASSWORD=wamux -e POSTGRES_DB=wamux -p 5433:5432 postgres:16" >&2
  exit 1
fi

stage "fmt --check"
cargo fmt --check

stage "clippy (default)"
cargo clippy --all-targets -- -D warnings

stage "clippy (--features stress)"
cargo clippy --features stress --all-targets -- -D warnings

stage "tests (unit + integration)"
cargo test

stage "stress tests (fast: M1/M2a/M2b)"
cargo test --features stress --test stress_handshake

if [[ "$FULL" == 1 ]]; then
  stage "FULL: load test (HOL blocking + gap)"
  cargo test --test load_multi_account -- --ignored

  stage "FULL: keepalive longevity (~25s)"
  cargo test --features stress --test stress_handshake \
    connection_survives_keepalive_window -- --ignored

  stage "FULL: M3 scale (${STRESS_ACCOUNTS:-199} clients vs mock)"
  cargo test --features stress --test stress_handshake \
    connect_many_clients_against_mock -- --ignored
fi

if [[ "$FULL" == 1 ]]; then stage "CI PASSED (full)"; else stage "CI PASSED"; fi
