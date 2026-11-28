"""#280: scripts/check-file-size.py fails CI when a Rust file reaches 500 lines
without a named exception, and when an exception is no longer needed.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no database.
"""
import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-file-size.py"
FLOOR = 100


def load_check():
    spec = importlib.util.spec_from_file_location("check_file_size", CHECK)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run_check(root):
    return subprocess.run([str(CHECK), "--root", str(root)], capture_output=True, text=True, check=False)


def lines(count):
    return "".join(f"fn f{i}() {{}}\n" for i in range(count))


def write_tree(root, extra=None, clean_files=FLOOR):
    """`clean_files` short Rust files, plus `extra` {path: body}."""
    for i in range(clean_files):
        path = Path(root) / "crates" / "demo" / "src" / f"m{i:03d}.rs"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(lines(3))
    for rel, body in (extra or {}).items():
        path = Path(root) / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)


class CheckFileSize(unittest.TestCase):
    def assert_reported_failure(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        for needle in needles:
            self.assertIn(needle, output)

    def check(self, extra=None, clean_files=FLOOR):
        with tempfile.TemporaryDirectory() as root:
            write_tree(root, extra, clean_files)
            return run_check(root)

    def test_repo_passes(self):
        result = run_check(REPO)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("file size ok:", result.stdout)

    def test_a_file_of_499_lines_passes(self):
        result = self.check({"crates/demo/src/big.rs": lines(499)})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_file_of_500_lines_fails_with_its_count(self):
        result = self.check({"crates/demo/src/big.rs": lines(500)})
        self.assert_reported_failure(result, "crates/demo/src/big.rs: 500 lines")

    def test_test_files_are_in_scope(self):
        result = self.check(
            {
                "crates/demo/src/big_tests.rs": lines(600),
                "crates/demo/tests/suite.rs": lines(700),
            }
        )
        self.assert_reported_failure(result, "big_tests.rs: 600 lines", "suite.rs: 700 lines")

    def test_build_output_and_dot_dirs_are_skipped(self):
        result = self.check(
            {
                "target/debug/build/out/generated.rs": lines(900),
                ".jj/repo/stray.rs": lines(900),
                "crates/demo/target/x.rs": lines(900),
            }
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_scanning_too_few_files_fails(self):
        result = self.check(clean_files=FLOOR - 1)
        self.assert_reported_failure(result, f"only {FLOOR - 1} Rust files scanned")

    def test_an_excepted_file_over_the_limit_passes(self):
        check = load_check()
        problems = check.size_problems({"a.rs": 800, "b.rs": 10}, {"a.rs": "one big match"})
        self.assertEqual(problems, [])

    def test_an_exception_under_the_limit_fails_so_the_table_shrinks(self):
        check = load_check()
        problems = check.size_problems({"a.rs": 499}, {"a.rs": "one big match"})
        self.assertEqual(len(problems), 1)
        self.assertIn("a.rs: 499 lines, under 500: remove it from EXCEPTIONS", problems[0])

    def test_an_exception_for_a_missing_file_fails(self):
        check = load_check()
        problems = check.size_problems({"b.rs": 10}, {"gone.rs": "split long ago"})
        self.assertEqual(problems, ["gone.rs: in EXCEPTIONS but not found: remove the entry"])

    def test_every_exception_in_the_script_names_a_reason(self):
        for path, reason in load_check().EXCEPTIONS.items():
            self.assertTrue(reason.strip(), f"{path} has no reason")


if __name__ == "__main__":
    unittest.main()
