"""#237: scripts/check-rust-cache.py fails CI when a job restores the Rust
cache under a key the runner image can move.

rust-cache hashes every installed toolchain into its key, and the hosted
images ship a preinstalled stable whose version changes between images, so the
same commit restores on one image and builds cold on the next. The rule: every
cached Rust setup (`actions-rust-lang/setup-rust-toolchain` unless `cache:` is
false, or `Swatinem/rust-cache` called directly) comes after a step, in the
same job, that lists and uninstalls every toolchain the image brought.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no network.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-rust-cache.py"
WORKFLOW = ".github/workflows/ci.yml"

SHA = "3d3c42e5aac5ba805825da76410c181273ba90b1"
CHECKOUT = f"      - uses: actions/checkout@{SHA} # v7.0.1\n"
SETUP_USES = f"      - uses: actions-rust-lang/setup-rust-toolchain@{SHA} # v2.0.0\n"
CACHE_TRUE = SETUP_USES + "        with:\n          cache: true\n"
DROP = (
    "      - name: drop the runner's toolchains\n"
    '        run: rustup toolchain list --quiet | while read -r name; do rustup toolchain uninstall "$name"; done\n'
)
DROP_BLOCK = (
    "      - name: drop the runner's toolchains\n"
    "        run: |\n"
    "          for name in $(rustup toolchain list --quiet); do\n"
    '            rustup toolchain uninstall "$name"\n'
    "          done\n"
)
STABLE_ONLY = "      - run: rustup toolchain uninstall stable\n"
RUST_CACHE = f"      - uses: Swatinem/rust-cache@{SHA} # v2.9.2\n"
GATES = "      - run: scripts/ci.sh\n"


def job(name, *steps):
    return f"  {name}:\n    runs-on: ubuntu-26.04\n    steps:\n" + "".join(steps)


def workflow(*jobs):
    return "name: CI\non: [push]\njobs:\n" + "".join(jobs)


GOOD_JOB = job("good", CHECKOUT, DROP, CACHE_TRUE, GATES)


def run_check(root):
    return subprocess.run(
        [str(CHECK), "--root", str(root)],
        capture_output=True,
        text=True,
        check=False,
    )


def check_workflow(text):
    with tempfile.TemporaryDirectory() as tmp:
        folder = Path(tmp) / ".github" / "workflows"
        folder.mkdir(parents=True)
        (folder / "ci.yml").write_text(text)
        return run_check(tmp)


def line_of(text, needle, occurrence=1):
    """1-based line of the `occurrence`-th line containing `needle`."""
    hits = [n for n, line in enumerate(text.splitlines(), start=1) if needle in line]
    return hits[occurrence - 1]


class RustCacheKeyTests(unittest.TestCase):
    def assert_passes(self, text):
        result = check_workflow(text)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("rust cache ok", result.stdout)

    def assert_reported(self, text, line):
        result = check_workflow(text)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        reported = [row for row in result.stdout.splitlines() if row.startswith(f"{WORKFLOW}:{line}:")]
        self.assertEqual(len(reported), 1, f"expected one finding at {WORKFLOW}:{line}:\n{result.stdout}")
        self.assertIn("dropped", reported[0])
        self.assertIn("rust cache FAILED: 1 violations", result.stdout)

    def test_repo_passes(self):
        result = run_check(REPO)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("rust cache ok", result.stdout)

    def test_ci_sh_runs_the_check(self):
        lines = (REPO / "scripts" / "ci.sh").read_text().splitlines()
        self.assertIn("scripts/check-rust-cache.py", [line.strip() for line in lines])

    def test_cached_setup_without_drop_fails(self):
        text = workflow(GOOD_JOB, job("bare", CHECKOUT, CACHE_TRUE, GATES))
        self.assert_reported(text, line_of(text, "setup-rust-toolchain", 2))

    def test_default_cache_without_drop_fails(self):
        text = workflow(GOOD_JOB, job("bare", CHECKOUT, SETUP_USES, GATES))
        self.assert_reported(text, line_of(text, "setup-rust-toolchain", 2))

    def test_drop_after_setup_fails(self):
        text = workflow(GOOD_JOB, job("late", CHECKOUT, CACHE_TRUE, DROP, GATES))
        self.assert_reported(text, line_of(text, "setup-rust-toolchain", 2))

    def test_drop_in_another_job_does_not_count(self):
        text = workflow(GOOD_JOB, job("dropper", CHECKOUT, DROP, GATES), job("bare", CHECKOUT, CACHE_TRUE, GATES))
        self.assert_reported(text, line_of(text, "setup-rust-toolchain", 2))

    def test_named_uninstall_only_fails(self):
        text = workflow(GOOD_JOB, job("named", CHECKOUT, STABLE_ONLY, CACHE_TRUE, GATES))
        self.assert_reported(text, line_of(text, "setup-rust-toolchain", 2))

    def test_direct_rust_cache_without_drop_fails(self):
        text = workflow(GOOD_JOB, job("direct", CHECKOUT, RUST_CACHE, GATES))
        self.assert_reported(text, line_of(text, "Swatinem/rust-cache"))

    def test_cache_false_needs_no_drop(self):
        off = SETUP_USES + "        with:\n          cache: false\n"
        quoted = SETUP_USES + '        with:\n          cache: "false"\n'
        self.assert_passes(workflow(GOOD_JOB, job("off", CHECKOUT, off, GATES), job("quoted", CHECKOUT, quoted, GATES)))

    def test_block_run_counts(self):
        self.assert_passes(workflow(job("block", CHECKOUT, DROP_BLOCK, CACHE_TRUE, GATES)))

    def test_scanned_nothing_fails(self):
        result = check_workflow(workflow(job("plain", CHECKOUT, GATES)))
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("scanned nothing", result.stdout)


if __name__ == "__main__":
    unittest.main()
