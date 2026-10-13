"""#85: scripts/check-comment-provenance.py fails CI when a source names the
review that produced it instead of the reason the code is this way.

The rule: no line of `crates/*/src/**/*.rs` or `crates/*/proto/**/*.proto`
matches `code-review 20<yy>` or `edge-review`. Who reviewed a line, and when,
is history: it goes to the commit message, not the code. Unlike `used to` or
`no longer`, the pattern has no legitimate use, so there is no allowlist.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no database.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-comment-provenance.py"
FLOOR = 20
CLEAN = "// A quote in a DM has no participant, so none is sent.\nfn quote() {}\n"


def run_check(root):
    return subprocess.run(
        [str(CHECK), "--root", str(root)],
        capture_output=True,
        text=True,
        check=False,
    )


def write_tree(root, extra=None, clean_files=FLOOR):
    """`clean_files` clean sources, plus `extra` {path: body}."""
    src = root / "crates" / "demo" / "src"
    src.mkdir(parents=True)
    for i in range(clean_files):
        (src / f"unit_{i:02d}.rs").write_text(CLEAN)
    for rel, body in (extra or {}).items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)


class CheckCommentProvenance(unittest.TestCase):
    def assert_reported_failure(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        for needle in needles:
            self.assertIn(needle, output)

    def test_repo_passes_with_no_review_provenance(self):
        result = run_check(REPO)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("comment provenance ok:", result.stdout)

    def test_clean_tree_passes_and_counts_the_files(self):
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp))
            result = run_check(tmp)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"comment provenance ok: {FLOOR} files", result.stdout)

    def test_dated_code_review_in_rust_fails_with_its_line(self):
        body = "fn a() {}\n// Regression (code-review 2026-06-11): a DM quote.\nfn b() {}\n"
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), {"crates/demo/src/domain/quote_tests.rs": body})
            result = run_check(tmp)
        self.assert_reported_failure(result, "quote_tests.rs:2", "code-review 2026")
        self.assertNotIn("unit_00.rs", result.stdout + result.stderr)

    def test_edge_review_in_a_doc_comment_fails(self):
        body = "/// Mapped this way (edge-review-insights.md achado #3).\npub enum E {}\n"
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), {"crates/demo/src/error.rs": body})
            result = run_check(tmp)
        self.assert_reported_failure(result, "error.rs:1", "edge-review")

    def test_proto_comments_are_scanned(self):
        body = 'syntax = "proto3";\n\n// Shaped by the code-review 2026-06-11 pass.\nmessage M {}\n'
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), {"crates/demo/proto/store/blobs.proto": body})
            result = run_check(tmp)
        self.assert_reported_failure(result, "blobs.proto:3")

    def test_every_hit_is_reported_not_just_the_first(self):
        extra = {
            "crates/demo/src/a.rs": "// edge-review said so\nfn a() {}\n",
            "crates/demo/src/b.rs": "fn b() {}\n\n// (code-review 2026-07-01)\n",
        }
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), extra)
            result = run_check(tmp)
        self.assert_reported_failure(result, "a.rs:1", "b.rs:3")

    def test_review_without_provenance_passes(self):
        body = "// The edge reviews the receipt; a code review found nothing here.\nfn r() {}\n"
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), {"crates/demo/src/receipt.rs": body})
            result = run_check(tmp)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_integration_tests_and_docs_are_not_scanned(self):
        extra = {
            "crates/demo/tests/suite.rs": "// Regression (code-review 2026-06-11)\n",
            "docs/notes.md": "edge-review-insights.md\n",
        }
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), extra)
            result = run_check(tmp)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_too_few_files_fails_the_floor(self):
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), clean_files=3)
            result = run_check(tmp)
        self.assert_reported_failure(result, str(FLOOR))


if __name__ == "__main__":
    unittest.main()
