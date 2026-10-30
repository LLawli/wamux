"""#235: scripts/lib/test-filter.sh `must_run_matching`, the "suite AND engine"
filter ci.sh runs inside a consolidated test binary. libtest filters are OR-ed
substrings, so the helper lists the tests, keeps the names a regex matches and
runs exactly those.

Run: python3 -m unittest discover -s scripts/tests -v
Needs bash and the checkout; a stub `cargo` stands in for the real one.
"""
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
HELPER = REPO / "scripts" / "lib" / "test-filter.sh"

# What `cargo test ... -- --list --format terse` prints for a consolidated binary.
LISTING = """store_parity::signal::sqlite_identities_round_trip: test
store_parity::signal::postgres_identities_round_trip: test
store_parity::blobs::turso_and_sqlite_blob_parity: test
storage_backend::sqlite_opens_a_fresh_store: test
existing_store::sqlite_store_written_before_unification_is_converted: test
existing_store::sqlite_store_written_before_unification_is_converted_twice: test
"""

# Logs every call, one argv per line (args joined by \\x1f), and answers a
# `--list` with $STUB_LISTING. $STUB_LIST_STATUS fails the listing.
STUB = """#!/usr/bin/env bash
printf '%s\\x1f' "$@" >> "$STUB_LOG"; printf '\\n' >> "$STUB_LOG"
for arg in "$@"; do
  if [[ "$arg" == --list ]]; then
    printf '%s' "$STUB_LISTING"
    exit "${STUB_LIST_STATUS:-0}"
  fi
done
echo "running the selected tests"
exit 0
"""


class MustRunMatching(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        cargo = self.dir / "cargo"
        cargo.write_text(STUB)
        cargo.chmod(cargo.stat().st_mode | stat.S_IEXEC)
        self.log = self.dir / "calls.log"
        self.log.write_text("")

    def tearDown(self):
        self.tmp.cleanup()

    def run_helper(self, *args, list_status=0):
        env = dict(os.environ)
        env.update(
            PATH=f"{self.dir}:{env['PATH']}",
            STUB_LOG=str(self.log),
            STUB_LISTING=LISTING,
            STUB_LIST_STATUS=str(list_status),
        )
        script = f'set -euo pipefail; source "{HELPER}"; must_run_matching "$@"'
        return subprocess.run(
            ["bash", "-c", script, "helper", *args],
            capture_output=True,
            text=True,
            env=env,
            check=False,
        )

    def calls(self):
        return [line.split("\x1f")[:-1] for line in self.log.read_text().splitlines()]

    def run_call(self):
        """The one cargo call that is not the listing."""
        runs = [c for c in self.calls() if "--list" not in c]
        self.assertEqual(len(runs), 1, self.calls())
        return runs[0]

    def test_a_regex_that_matches_nothing_fails_naming_it(self):
        result = self.run_helper("--test", "storage", "--", "^no_such_suite::")
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("^no_such_suite::", result.stdout + result.stderr)
        listing = [c for c in self.calls() if "--list" in c]
        self.assertEqual(len(listing), 1, "the helper lists before it decides")
        self.assertEqual([c for c in self.calls() if "--list" not in c], [])

    def test_only_the_matching_names_run_exactly(self):
        regex = "^existing_store::sqlite_store_written_before_unification_is_converted$"
        result = self.run_helper("--test", "storage", "--", regex)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        run = self.run_call()
        self.assertEqual(run[:3], ["test", "-p", "wamux"], run)
        self.assertIn("--test", run)
        self.assertIn("storage", run)
        self.assertIn("--exact", run)
        after = run[run.index("--") + 1 :]
        names = [a for a in after if not a.startswith("-")]
        self.assertEqual(
            names, ["existing_store::sqlite_store_written_before_unification_is_converted"]
        )

    def test_the_match_is_an_and_of_suite_and_engine(self):
        result = self.run_helper("--test", "storage", "--", "^store_parity::.*sqlite_")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        run = self.run_call()
        names = [a for a in run[run.index("--") + 1 :] if not a.startswith("-")]
        self.assertEqual(
            sorted(names),
            [
                "store_parity::blobs::turso_and_sqlite_blob_parity",
                "store_parity::signal::sqlite_identities_round_trip",
            ],
        )

    def test_extra_libtest_args_reach_the_list_and_the_run(self):
        result = self.run_helper(
            "--features", "stress", "--test", "storage", "--", "^storage_backend::", "--ignored"
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        listing = [c for c in self.calls() if "--list" in c]
        self.assertEqual(len(listing), 1, self.calls())
        self.assertIn("--ignored", listing[0])
        run = self.run_call()
        self.assertIn("--ignored", run)
        self.assertIn("--features", run)
        self.assertIn("stress", run)

    def test_a_failing_list_fails(self):
        result = self.run_helper("--test", "storage", "--", "^store_parity::", list_status=101)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        listing = [c for c in self.calls() if "--list" in c]
        self.assertEqual(len(listing), 1, "the helper lists before it runs")
        self.assertEqual([c for c in self.calls() if "--list" not in c], [])


if __name__ == "__main__":
    unittest.main()
