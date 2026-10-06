"""#126: scripts/check-wire-debug.py fails CI when the event mapping puts a Rust
`Debug` string on the wire anywhere but `variant_name`.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no database.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-wire-debug.py"
MAPPING = "crates/wamux/src/domain/event_mapping.rs"

# The allowed site as it is written in the mapping: note the '{' char literal.
VARIANT_NAME = '''fn variant_name(event: &Event) -> String {
    let debug = format!("{event:?}");
    debug
        .split(['(', '{', ' '])
        .next()
        .unwrap_or("Event")
        .to_string()
}
'''


def run_check(root):
    return subprocess.run([str(CHECK), "--root", str(root)], capture_output=True, text=True, check=False)


class CheckWireDebug(unittest.TestCase):
    def check(self, mapping):
        with tempfile.TemporaryDirectory() as root:
            if mapping is not None:
                path = Path(root) / MAPPING
                path.parent.mkdir(parents=True)
                path.write_text(mapping)
            return run_check(root)

    def assert_reported_failure(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        for needle in needles:
            self.assertIn(needle, output)

    def test_the_real_mapping_passes(self):
        result = run_check(REPO)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_debug_inside_variant_name_passes(self):
        result = self.check("use x;\n" + VARIANT_NAME)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_the_three_debug_sites_of_before_126_fail(self):
        mapping = (
            "fn map_event(event: &Event) {\n"
            '    let a = format!("{:?}", l.reason);\n'
            '    let b = format!("{b:?}");\n'
            '    let c = format!("{:#?}", u.unavailable_type);\n'
            "}\n" + VARIANT_NAME
        )
        self.assert_reported_failure(self.check(mapping), f"{MAPPING}:2:", f"{MAPPING}:3:", f"{MAPPING}:4:")

    # The '{' literal inside variant_name must not keep the allowed region open:
    # a Debug spec in a function after it still fails.
    def test_debug_after_variant_name_fails(self):
        mapping = VARIANT_NAME + 'fn later() -> String {\n    format!("{:?}", 1)\n}\n'
        self.assert_reported_failure(self.check(mapping), f"{MAPPING}:10:")

    def test_a_debug_spec_in_a_comment_is_not_read(self):
        result = self.check('// format!("{:?}", reason) was the old shape\n' + VARIANT_NAME)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_missing_mapping_fails(self):
        self.assert_reported_failure(self.check(None), "not found")

    def test_a_missing_variant_name_fails(self):
        self.assert_reported_failure(self.check("fn map_event() {}\n"), "fn variant_name not found")


if __name__ == "__main__":
    unittest.main()
