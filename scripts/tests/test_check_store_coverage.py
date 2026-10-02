"""#60: scripts/check-store-coverage.py fails CI when a method of either store
engine is never called from crates/wamux/tests/.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no database.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-store-coverage.py"
FLOOR = 59


def run_check(root):
    return subprocess.run(
        [str(CHECK), "--root", str(root)],
        capture_output=True,
        text=True,
        check=False,
    )


def fake_names(count):
    return [f"store_method_{i:02d}" for i in range(count)]


def write_tree(root, postgres_names, sqlite_names, test_body):
    """A minimal checkout: one `*_store.rs` per engine and one test file."""
    storage = root / "crates" / "wamux" / "src" / "storage"
    for engine, names in (("postgres", postgres_names), ("sqlite", sqlite_names)):
        engine_dir = storage / engine
        engine_dir.mkdir(parents=True)
        body = "\n".join(f"    async fn {n}(&self) -> Result<()> {{ Ok(()) }}" for n in names)
        (engine_dir / "fake_store.rs").write_text(f"impl FakeStore for X {{\n{body}\n}}\n")
        # Not a store module: its async fns must not count.
        (engine_dir / "accounts.rs").write_text("async fn not_a_store_method() {}\n")
    tests = root / "crates" / "wamux" / "tests" / "nested"
    tests.mkdir(parents=True)
    (tests / "calls.rs").write_text(test_body)


def calls(names):
    return "\n".join(f"    backend.{n}().await.unwrap();" for n in names) + "\n"


class CheckStoreCoverage(unittest.TestCase):
    def assert_reported_failure(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        for needle in needles:
            self.assertIn(needle, output)

    def test_repo_passes_with_every_method_called(self):
        result = run_check(REPO)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"store coverage ok: {FLOOR} methods per engine", result.stdout)

    def test_unreferenced_method_fails_and_is_named(self):
        names = fake_names(FLOOR)
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), names, names, calls(names[1:]))
            result = run_check(tmp)
        self.assert_reported_failure(result, names[0])
        self.assertNotIn(names[1], result.stdout + result.stderr)

    def test_reference_in_comment_does_not_count(self):
        names = fake_names(FLOOR)
        body = calls(names[1:]) + f"    // backend.{names[0]}().await is covered elsewhere\n"
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), names, names, body)
            result = run_check(tmp)
        self.assert_reported_failure(result, names[0])

    def test_bare_word_without_call_does_not_count(self):
        names = fake_names(FLOOR)
        body = calls(names[1:]) + f'    let label = "{names[0]}";\n    let f = {names[0]};\n'
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), names, names, body)
            result = run_check(tmp)
        self.assert_reported_failure(result, names[0])

    def test_engines_with_different_methods_fail(self):
        names = fake_names(FLOOR + 1)
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), names, names[:-1], calls(names))
            result = run_check(tmp)
        self.assert_reported_failure(result, names[-1])

    def test_too_few_methods_fails_the_floor(self):
        names = fake_names(3)
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), names, names, calls(names))
            result = run_check(tmp)
        self.assert_reported_failure(result, str(FLOOR))


if __name__ == "__main__":
    unittest.main()
