# shellcheck shell=bash
# Sourced by scripts/ci.sh. libtest filters are OR-ed substrings, so "suite AND
# engine" inside one consolidated test binary has no filter of its own (#235).
#
# must_run_matching [cargo test args...] -- <regex> [libtest args...]
#   Lists the tests of `cargo test -p wamux <cargo test args>` (with the libtest
#   args, so `--ignored` lists the ignored ones), keeps the names matching the
#   extended regex (unanchored, like a libtest substring), fails naming the
#   regex when none match, and runs exactly those names with `--exact`.

# Splits "<cargo args> -- <regex> <libtest args>" into the globals the helper
# reads: MRM_CARGO_ARGS, MRM_REGEX, MRM_LIBTEST_ARGS. The ${a[@]+"${a[@]}"}
# spelling keeps an empty array legal under `set -u` on the macOS bash 3.2 that
# the macOS job may run (no mapfile either, hence the read loop below).
split_filter_args() {
  MRM_CARGO_ARGS=()
  while [[ $# -gt 0 && "$1" != -- ]]; do MRM_CARGO_ARGS+=("$1"); shift; done
  [[ $# -ge 2 ]] || { echo "ERROR: must_run_matching needs '-- <regex>'" >&2; return 2; }
  MRM_REGEX="$2"
  shift 2
  MRM_LIBTEST_ARGS=()
  [[ $# -eq 0 ]] || MRM_LIBTEST_ARGS=("$@")
}

must_run_matching() {
  split_filter_args "$@" || return $?
  local listing name names=()
  # Captured, not piped: a pipe would hide cargo's exit status (a compile error)
  # from `set -e`.
  listing=$(cargo test -p wamux ${MRM_CARGO_ARGS[@]+"${MRM_CARGO_ARGS[@]}"} -- --list --format terse \
    ${MRM_LIBTEST_ARGS[@]+"${MRM_LIBTEST_ARGS[@]}"}) || return 1
  # grep exits 1 on no match; that is the empty-selection error below, not a crash.
  while IFS= read -r name; do names+=("$name"); done \
    < <(sed -n 's/: test$//p' <<<"$listing" | grep -E -- "$MRM_REGEX" || true)
  if [[ ${#names[@]} -eq 0 ]]; then
    echo "ERROR: gate ran 0 tests (renamed/moved?): regex '$MRM_REGEX' matched nothing in: cargo test -p wamux ${MRM_CARGO_ARGS[*]-}" >&2
    return 1
  fi
  cargo test -p wamux ${MRM_CARGO_ARGS[@]+"${MRM_CARGO_ARGS[@]}"} -- --exact \
    ${MRM_LIBTEST_ARGS[@]+"${MRM_LIBTEST_ARGS[@]}"} "${names[@]}"
}
