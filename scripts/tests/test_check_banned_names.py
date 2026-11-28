"""#280: scripts/check-banned-names.py fails CI when a `fn`, `mod` or
`macro_rules!` is named exactly `handle`, `process`, `data`, `util` or
`manager`, and when an exception no longer matches a declaration.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no database.
"""
import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-banned-names.py"
FLOOR = 100


def load_check():
    spec = importlib.util.spec_from_file_location("check_banned_names", CHECK)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run_check(root):
    return subprocess.run([str(CHECK), "--root", str(root)], capture_output=True, text=True, check=False)


def write_tree(root, extra=None, clean_files=FLOOR):
    """`clean_files` Rust files with specific names, plus `extra` {path: body}."""
    for i in range(clean_files):
        path = Path(root) / "crates" / "demo" / "src" / f"m{i:03d}.rs"
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(f"fn send_receipt_{i}() {{}}\n")
    for rel, body in (extra or {}).items():
        path = Path(root) / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)


class CheckBannedNames(unittest.TestCase):
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
        self.assertIn("banned names ok:", result.stdout)

    def test_each_banned_function_name_fails_with_its_line(self):
        for word in ("handle", "process", "data", "util", "manager"):
            body = f"struct S;\nimpl S {{\n    pub async fn {word}(&self) {{}}\n}}\n"
            result = self.check({"crates/demo/src/bad.rs": body})
            self.assert_reported_failure(result, f"bad.rs:3: `{word}` is a banned name")

    def test_a_test_helper_named_handle_fails(self):
        body = "#[cfg(test)]\nmod tests {\n    fn handle() -> u8 { 1 }\n}\n"
        result = self.check({"crates/demo/src/thing.rs": body})
        self.assert_reported_failure(result, "thing.rs:3: `handle`")

    def test_a_module_and_a_macro_named_util_fail(self):
        body = "mod util;\npub mod data {}\nmacro_rules! manager { () => {} }\n"
        result = self.check({"crates/demo/src/lib.rs": body})
        self.assert_reported_failure(result, "lib.rs:1: `util`", "lib.rs:2: `data`", "lib.rs:3: `manager`")

    def test_specific_compounds_pass(self):
        body = (
            "fn handle_receipt() {}\n"
            "fn process_batch() {}\n"
            "mod data_dir;\n"
            "struct AccountHandle;\n"
            "fn handles() {}\n"
        )
        result = self.check({"crates/demo/src/ok.rs": body})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_bindings_parameters_fields_and_paths_are_out_of_scope(self):
        body = (
            "use std::process::ExitCode;\n"
            "struct Fixture { handle: Arc<AccountHandle>, data: Vec<u8> }\n"
            "fn forward(handle: Arc<AccountHandle>) {\n"
            "    let data = handle.data.clone();\n"
            "    std::process::exit(0);\n"
            "}\n"
        )
        result = self.check({"crates/demo/src/ok.rs": body})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_comments_and_strings_are_not_read(self):
        body = (
            "// fn handle() was renamed\n"
            "/* mod util;\n   fn data() */\n"
            'const S: &str = "fn process()";\n'
        )
        result = self.check({"crates/demo/src/ok.rs": body})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_scanning_too_few_files_fails(self):
        result = self.check(clean_files=FLOOR - 1)
        self.assert_reported_failure(result, f"only {FLOOR - 1} Rust files scanned")

    def test_an_excepted_declaration_passes(self):
        check = load_check()
        hits = [("src/a.rs", 3, "process")]
        self.assertEqual(check.name_problems(hits, {"src/a.rs:process": "mirrors a trait"}), [])

    def test_an_exception_that_matches_nothing_fails_so_the_table_shrinks(self):
        check = load_check()
        problems = check.name_problems([], {"src/a.rs:process": "mirrors a trait"})
        self.assertEqual(problems, ["src/a.rs:process: in EXCEPTIONS but no such declaration: remove the entry"])

    def test_every_exception_in_the_script_names_a_reason(self):
        for key, reason in load_check().EXCEPTIONS.items():
            self.assertTrue(reason.strip(), f"{key} has no reason")


if __name__ == "__main__":
    unittest.main()
