"""#81: scripts/check-doc-links.py fails CI when a document or a comment points
at something that does not exist.

Three kinds of pointer are checked:
- a relative Markdown link (`[t](dest)`, `![t](dest)`, `[id]: dest`): the file
  or directory must exist, relative to the linking file;
- its `#anchor`, against the GitHub slugs of the target file's headings (or of
  the same file for a bare `#anchor`);
- a mention of `docs/<...>.md` (relative to the repo root) in code, scripts and
  Markdown text. CHANGELOG.md and docs/BREAKING-CHANGES-*.md are history: they
  name files as they were, so their mentions are not checked (their links are).

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no database.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-doc-links.py"
FLOOR_FILES = 10
FLOOR_LINKS = 5
PAGE = "# Page {i}\n\nSome text.\n\n## Section two\n\nMore text.\n"


def run_check(root):
    return subprocess.run(
        [str(CHECK), "--root", str(root)],
        capture_output=True,
        text=True,
        check=False,
    )


def write_tree(root, extra=None, md_files=FLOOR_FILES, links=6):
    """README.md plus `md_files - 1` pages under docs/, `links` relative links
    from the README (one with an anchor), one mention in the README and one in
    a Rust comment; then `extra` {path: body}."""
    docs = root / "docs"
    docs.mkdir(parents=True)
    for i in range(md_files - 1):
        (docs / f"page_{i:02d}.md").write_text(PAGE.format(i=i))
    lines = ["# Demo", ""]
    lines += [f"- [page {i}](docs/page_{i:02d}.md)" for i in range(links - 1)]
    lines += ["- [section](docs/page_01.md#section-two)", "", "Spec: `docs/page_02.md`.", ""]
    (root / "README.md").write_text("\n".join(lines))
    src = root / "crates" / "demo" / "src"
    src.mkdir(parents=True)
    (src / "lib.rs").write_text("// The layout is in docs/page_00.md.\npub fn demo() {}\n")
    for rel, body in (extra or {}).items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)


class CheckDocLinks(unittest.TestCase):
    def check_tree(self, extra=None, **kwargs):
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), extra, **kwargs)
            return run_check(tmp)

    def assert_passes(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("doc links ok:", result.stdout)

    def assert_reported_failure(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        self.assertIn("doc links FAILED", output)
        for needle in needles:
            self.assertIn(needle, output)

    def test_repo_passes(self):
        self.assert_passes(run_check(REPO))

    def test_clean_tree_passes_and_counts(self):
        result = self.check_tree()
        self.assert_passes(result)
        self.assertIn(
            f"doc links ok: {FLOOR_FILES} markdown files, 6 links, 2 mentions",
            result.stdout,
        )

    def test_missing_link_target_fails_with_file_and_line(self):
        body = "# Broken\n\nA line.\nSee [the old notes](gone.md).\n"
        result = self.check_tree({"docs/broken.md": body})
        self.assert_reported_failure(result, "docs/broken.md:4", "gone.md")
        self.assertNotIn("README.md:", result.stdout + result.stderr)

    def test_missing_anchor_in_other_file_fails(self):
        body = "# Anchors\n[x](page_00.md#no-such-heading)\n[y](page_00.md#section-two)\n"
        result = self.check_tree({"docs/anchors.md": body})
        self.assert_reported_failure(result, "docs/anchors.md:2", "no-such-heading")
        self.assertNotIn("docs/anchors.md:3", result.stdout + result.stderr)

    def test_same_file_anchor_is_checked(self):
        body = "# Top\n\n## Real one\n[ok](#real-one)\n[bad](#fake)\n"
        result = self.check_tree({"docs/self.md": body})
        self.assert_reported_failure(result, "docs/self.md:5", "fake")
        self.assertNotIn("docs/self.md:4", result.stdout + result.stderr)

    def test_github_slug_rules(self):
        target = (
            "# Why `foo`, and Bar?\n\n## Repeated\n\n## Repeated\n\n"
            "## Café & co.\n\n## The store is a secret\n"
        )
        good = (
            "[a](slugs.md#why-foo-and-bar)\n[b](slugs.md#repeated)\n"
            "[c](slugs.md#repeated-1)\n[d](slugs.md#café--co)\n"
            "[e](slugs.md#the-store-is-a-secret)\n"
        )
        self.assert_passes(self.check_tree({"docs/slugs.md": target, "docs/use.md": good}))
        bad = "[f](slugs.md#repeated-2)\n"
        result = self.check_tree({"docs/slugs.md": target, "docs/use.md": bad})
        self.assert_reported_failure(result, "docs/use.md:1", "repeated-2")

    def test_external_links_are_ignored(self):
        body = (
            "# Out\n[a](https://example.com/gone.md)\n[b](http://example.com)\n"
            "[c](mailto:someone@example.com)\n"
        )
        result = self.check_tree({"docs/out.md": body})
        self.assert_passes(result)
        self.assertIn("6 links", result.stdout)

    def test_links_in_code_are_ignored(self):
        body = (
            "# Code\n\n```markdown\n[x](nope.md)\n```\n\n"
            "Inline `[y](nope2.md)` is not a link.\n"
        )
        self.assert_passes(self.check_tree({"docs/code.md": body}))

    def test_directory_link_and_title_and_angle_brackets(self):
        body = (
            "# Forms\n[dir](../crates/demo/)\n[titled](page_00.md \"The title\")\n"
            "[angled](<page_01.md>)\n"
        )
        self.assert_passes(self.check_tree({"docs/forms.md": body}))

    def test_reference_definition_is_checked(self):
        body = "[ref]: missing-ref.md\n[ok]: page_00.md\n\n# Refs\n"
        result = self.check_tree({"docs/refs.md": body})
        self.assert_reported_failure(result, "docs/refs.md:1", "missing-ref.md")
        self.assertNotIn("docs/refs.md:2", result.stdout + result.stderr)

    def test_mention_in_rust_comment_of_missing_doc_fails(self):
        result = self.check_tree(
            {
                "crates/demo/src/stress.rs": "pub fn a() {}\n/// See docs/STATUS.md Sprint 3.\n",
                "crates/demo/tests/common/mod.rs": "// The B5 pattern, docs/BACKLOG.md.\n",
            }
        )
        self.assert_reported_failure(
            result,
            "crates/demo/src/stress.rs:2",
            "docs/STATUS.md",
            "crates/demo/tests/common/mod.rs:1",
            "docs/BACKLOG.md",
        )

    def test_mention_in_changelog_and_breaking_changes_is_history(self):
        history = {
            "CHANGELOG.md": "# Changelog\n\n- Removed `docs/STATUS.md`.\n",
            "docs/BREAKING-CHANGES-2026-01-01.md": "# Old\n\nSee docs/OLD.md.\n",
        }
        self.assert_passes(self.check_tree(history))
        linked = {"CHANGELOG.md": "# Changelog\n\n- See [old](docs/OLD.md).\n"}
        result = self.check_tree(linked)
        self.assert_reported_failure(result, "CHANGELOG.md:3", "docs/OLD.md")

    def test_hidden_target_and_script_fixture_dirs_are_skipped(self):
        skipped = {
            ".claude/notes.md": "[x](gone.md)\n",
            ".github/notes.md": "[x](gone.md)\n",
            "target/doc/page.md": "[y](gone.md)\n",
            "scripts/tests/test_fixture.py": 'BODY = "docs/notes.md"\n',
        }
        result = self.check_tree(skipped)
        self.assert_passes(result)
        self.assertIn(f"{FLOOR_FILES} markdown files", result.stdout)

    def test_every_problem_is_reported(self):
        result = self.check_tree(
            {
                "docs/multi.md": "# Multi\n[a](x1.md)\n[b](x2.md)\n",
                "scripts/ci.sh": "#!/bin/sh\n# the stages are listed in docs/GONE.md\n",
            }
        )
        self.assert_reported_failure(
            result, "docs/multi.md:2", "docs/multi.md:3", "scripts/ci.sh:2", "docs/GONE.md"
        )

    def test_too_few_files_fails_the_floor(self):
        few_files = self.check_tree(md_files=FLOOR_FILES - 1)
        self.assert_reported_failure(few_files, "markdown files")
        few_links = self.check_tree(links=FLOOR_LINKS - 1)
        self.assert_reported_failure(few_links, "links")


if __name__ == "__main__":
    unittest.main()
