"""#67: scripts/check-test-sleeps.py fails CI when a test sleeps without
saying why the sleep is not a synchronization point.

The rule: every `sleep(` in `crates/*/tests/**/*.rs`, and in the test files of
`crates/*/src` (`tests.rs`, `*_tests.rs`), has a comment containing
`not a sync point:` and a reason on the line right above it.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no database.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-test-sleeps.py"
FLOOR = 20
MARKED = (
    "    // not a sync point: the polling interval of a bounded wait\n"
    "    tokio::time::sleep(Duration::from_millis(10)).await;\n"
)


def run_check(root):
    return subprocess.run(
        [str(CHECK), "--root", str(root)],
        capture_output=True,
        text=True,
        check=False,
    )


def write_tree(root, extra=None, clean_files=FLOOR):
    """`clean_files` test files with marked sleeps, plus `extra` {path: body}."""
    tests = root / "crates" / "demo" / "tests"
    tests.mkdir(parents=True)
    for i in range(clean_files):
        (tests / f"suite_{i:02d}.rs").write_text(f"async fn t() {{\n{MARKED}}}\n")
    for rel, body in (extra or {}).items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)


class CheckTestSleeps(unittest.TestCase):
    def assert_reported_failure(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        for needle in needles:
            self.assertIn(needle, output)

    def test_repo_passes_with_every_sleep_marked(self):
        result = run_check(REPO)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("test sleeps ok:", result.stdout)
        self.assertIn("every sleep is marked", result.stdout)

    def test_unmarked_sleep_fails_and_is_named_with_its_line(self):
        body = "async fn t() {\n    let x = 1;\n    tokio::time::sleep(Duration::from_millis(150)).await;\n}\n"
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), {"crates/demo/tests/flaky.rs": body})
            result = run_check(tmp)
        self.assert_reported_failure(result, "flaky.rs:3")
        self.assertNotIn("suite_00.rs", result.stdout + result.stderr)

    def test_mark_two_lines_above_does_not_count(self):
        body = (
            "async fn t() {\n"
            "    // not a sync point: too far away\n"
            "    let x = 1;\n"
            "    tokio::time::sleep(Duration::from_millis(150)).await;\n"
            "}\n"
        )
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), {"crates/demo/tests/far.rs": body})
            result = run_check(tmp)
        self.assert_reported_failure(result, "far.rs:4")

    def test_mark_without_a_reason_fails(self):
        body = "async fn t() {\n    // not a sync point:\n    sleep(D).await;\n}\n"
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), {"crates/demo/tests/bare.rs": body})
            result = run_check(tmp)
        self.assert_reported_failure(result, "bare.rs:3")

    def test_production_source_is_not_scanned(self):
        body = "pub async fn drain() {\n    tokio::time::sleep(GRACE).await;\n}\n"
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), {"crates/demo/src/transport/shutdown.rs": body})
            result = run_check(tmp)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_test_files_under_src_are_scanned(self):
        unmarked = "async fn t() {\n    tokio::time::sleep(SOON).await;\n}\n"
        extra = {
            "crates/demo/src/transport/shutdown/tests.rs": unmarked,
            "crates/demo/src/storage/codec_tests.rs": unmarked,
        }
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), extra)
            result = run_check(tmp)
        self.assert_reported_failure(result, "shutdown/tests.rs:2", "codec_tests.rs:2")

    def test_too_few_files_fails_the_floor(self):
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), clean_files=3)
            result = run_check(tmp)
        self.assert_reported_failure(result, str(FLOOR))


if __name__ == "__main__":
    unittest.main()
