"""#82: scripts/check-doc-language.py fails CI when prose is written in Portuguese.

What is read:
- every Markdown file, outside fenced code blocks and inline code spans;
- the comments (line, block, doc) of every .rs and .proto file under crates/.
String literals are data, not prose: tests carry real WhatsApp content (poll
options, captions) in Portuguese on purpose. Text in double quotes inside a
comment is quoted data for the same reason. THIRD-PARTY-LICENSES.md is
generated from other people's license texts and is skipped.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no database.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-doc-language.py"
FLOOR_MARKDOWN = 10
FLOOR_SOURCES = 20
PAGE = "# Page {i}\n\nThe daemon relays what the library hands it.\n\n```sh\nls\n```\n"
SOURCE = "// The supervisor owns the handle.\n/// Relays one account.\npub fn demo_{i}() {{}}\n"


def run_check(root):
    return subprocess.run(
        [str(CHECK), "--root", str(root)],
        capture_output=True,
        text=True,
        check=False,
    )


def write_tree(root, extra=None, md_files=FLOOR_MARKDOWN, sources=FLOOR_SOURCES):
    """README.md plus `md_files - 1` pages under docs/, `sources` Rust files
    under crates/demo/src, all in English; then `extra` {path: body}."""
    docs = root / "docs"
    docs.mkdir(parents=True)
    (root / "README.md").write_text("# Demo\n\nA relay for many accounts.\n")
    for i in range(md_files - 1):
        (docs / f"page_{i:02d}.md").write_text(PAGE.format(i=i))
    src = root / "crates" / "demo" / "src"
    src.mkdir(parents=True)
    for i in range(sources):
        (src / f"mod_{i:02d}.rs").write_text(SOURCE.format(i=i))
    for rel, body in (extra or {}).items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body, encoding="utf-8")


class CheckDocLanguage(unittest.TestCase):
    def check_tree(self, extra=None, **kwargs):
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), extra, **kwargs)
            return run_check(tmp)

    def assert_passes(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("doc language ok:", result.stdout)

    def assert_reported_failure(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        self.assertIn("doc language FAILED", output)
        for needle in needles:
            self.assertIn(needle, output)

    def test_repo_passes(self):
        self.assert_passes(run_check(REPO))

    def test_clean_tree_passes(self):
        result = self.check_tree()
        self.assert_passes(result)
        self.assertIn(
            f"doc language ok: {FLOOR_MARKDOWN} markdown files, {FLOOR_SOURCES} sources",
            result.stdout,
        )

    def test_portuguese_markdown_prose_fails_with_file_and_line(self):
        body = "# Guide\n\nThe edge reads this.\nA borda não precisa mudar.\n"
        result = self.check_tree({"docs/guide.md": body})
        self.assert_reported_failure(result, "docs/guide.md:4", "não")
        self.assertNotIn("docs/guide.md:3", result.stdout + result.stderr)

    def test_uppercase_portuguese_word_fails(self):
        body = "# Deferred (deliberately NÃO implemented)\n"
        result = self.check_tree({"docs/deferred.md": body})
        self.assert_reported_failure(result, "docs/deferred.md:1", "NÃO")

    def test_markdown_code_fence_and_span_are_ignored(self):
        body = (
            "# Wire\n\nThe label is `não muda`.\n\n"
            "```proto\nstring push_name = 2; // não usado\n```\n\n"
            "~~~\nagora só isso\n~~~\n"
        )
        self.assert_passes(self.check_tree({"docs/wire.md": body}))

    def test_portuguese_rust_line_block_and_doc_comments_fail(self):
        body = (
            "//! Módulo que relays.\n"
            "/// Relays sem retry.\n"
            "fn a() {} // agora relays\n"
            "/* outer /* nested */ já dentro */\n"
            "fn b() {}\n"
        )
        result = self.check_tree({"crates/demo/src/pt.rs": body})
        self.assert_reported_failure(
            result,
            "crates/demo/src/pt.rs:1",
            "crates/demo/src/pt.rs:2",
            "crates/demo/src/pt.rs:3",
            "crates/demo/src/pt.rs:4",
        )
        self.assertNotIn("crates/demo/src/pt.rs:5", result.stdout + result.stderr)

    def test_rust_string_literals_are_ignored(self):
        body = (
            'const OPTIONS: [&str; 2] = ["Sim", "Não"];\n'
            'const PROMPT: &str = "responda: isso foi lido? // não é comentário";\n'
            'const RAW: &str = r#"uma "citação" sem fim"#;\n'
            "const C: char = '\"'; // The quote char is data.\n"
            "fn f<'a>(s: &'a str) -> &'a str { s } // Lifetimes are not chars.\n"
        )
        self.assert_passes(self.check_tree({"crates/demo/tests/data.rs": body}))

    def test_proto_comment_is_checked(self):
        body = (
            'syntax = "proto3";\n'
            "message M {\n"
            "  string label = 1; // vazio para contas pessoais, já que não há nome\n"
            "}\n"
        )
        result = self.check_tree({"crates/demo/proto/m.proto": body})
        self.assert_reported_failure(result, "crates/demo/proto/m.proto:3")

    def test_quoted_text_inside_a_comment_is_ignored(self):
        body = '//! It votes "Sim" and, five seconds later, "Não" through the RPC.\nfn v() {}\n'
        self.assert_passes(self.check_tree({"crates/demo/src/bin/poll.rs": body}))

    def test_english_lookalikes_pass(self):
        body = (
            "# Lookalikes\n\nA deque, a semaphore, semver, tin foil, a unique id,\n"
            "the agorafobia of nobody, upload and download.\n"
        )
        source = "// Pop the deque under the semaphore; semver says so.\nfn d() {}\n"
        self.assert_passes(
            self.check_tree({"docs/lookalikes.md": body, "crates/demo/src/d.rs": source})
        )

    def test_third_party_licenses_is_skipped(self):
        body = "# Licenses\n\nEste texto não é nosso para traduzir.\n"
        self.assert_passes(self.check_tree({"THIRD-PARTY-LICENSES.md": body}))

    def test_hidden_and_target_dirs_are_skipped(self):
        skipped = {
            ".claude/loop/scope.md": "# Escopo\n\nO que não entra.\n",
            ".github/notes.md": "Isso fica fora.\n",
            "target/doc/page.md": "Gerado, não lido.\n",
            "target/debug/build/out.rs": "// gerado pelo build, não lido\n",
        }
        result = self.check_tree(skipped)
        self.assert_passes(result)
        self.assertIn(f"{FLOOR_MARKDOWN} markdown files", result.stdout)

    def test_every_hit_is_reported(self):
        result = self.check_tree(
            {
                "docs/multi.md": "# Multi\n\nPrimeiro: não.\nSegundo: também.\n",
                "crates/demo/src/x.rs": "// então isso\nfn x() {}\n",
            }
        )
        self.assert_reported_failure(
            result, "docs/multi.md:3", "docs/multi.md:4", "crates/demo/src/x.rs:1"
        )

    def test_too_few_files_fails_the_floor(self):
        few_markdown = self.check_tree(md_files=FLOOR_MARKDOWN - 1)
        self.assert_reported_failure(few_markdown, "markdown files")
        few_sources = self.check_tree(sources=FLOOR_SOURCES - 1)
        self.assert_reported_failure(few_sources, "sources")


if __name__ == "__main__":
    unittest.main()
