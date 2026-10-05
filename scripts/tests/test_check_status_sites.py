"""#114: scripts/check-status-sites.py fails CI when a `tonic::Status` is
constructed outside WamuxError's mapping.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no database.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-status-sites.py"
ALLOWED = "crates/wamux-types/src/error.rs"


def run_check(root):
    return subprocess.run([str(CHECK), "--root", str(root)], capture_output=True, text=True, check=False)


def mapping(count):
    return "\n".join(f'        X{i} => Status::internal("m{i}"),' for i in range(count)) + "\n"


def write_tree(root, files):
    for rel, text in files.items():
        path = Path(root) / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)


def clean_tree():
    return {
        ALLOWED: mapping(7),
        "crates/wamux/src/services/mod.rs": "let handle = registry.resolve(&account)?;\n",
        "crates/wamux/src/services/messaging_service.rs": (
            "let first = stream.message().await?;\n"
            "Err(tonic::Status::from(WamuxError::NotConnected))\n"
            "// was Status::failed_precondition(\"account is not connected\")\n"
        ),
        "crates/wamux/src/domain/newsletters/metadata_tests.rs": 'Status::not_found("x")\n',
        "crates/wamux/src/storage/blob_codec/tests.rs": 'Status::internal("x")\n',
    }


class CheckStatusSites(unittest.TestCase):
    def assert_reported_failure(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        for needle in needles:
            self.assertIn(needle, output)

    def check(self, files):
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(tmp, files)
            return run_check(tmp)

    def test_repo_passes(self):
        result = subprocess.run([str(CHECK)], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_clean_tree_passes(self):
        result = self.check(clean_tree())
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("(7 sites)", result.stdout)

    def test_a_hand_built_status_in_a_service_fails(self):
        files = clean_tree()
        files["crates/wamux/src/services/mod.rs"] = (
            '.ok_or_else(|| Status::failed_precondition("account is not connected"))?;\n'
        )
        self.assert_reported_failure(self.check(files), "crates/wamux/src/services/mod.rs:1")

    def test_a_status_new_in_wamux_types_outside_the_mapping_fails(self):
        files = clean_tree()
        files["crates/wamux-types/src/jid.rs"] = "tonic::Status::new(Code::Internal, msg)\n"
        self.assert_reported_failure(self.check(files), "crates/wamux-types/src/jid.rs:1")

    def test_a_multi_line_call_is_still_found(self):
        files = clean_tree()
        files["crates/wamux/src/domain/groups.rs"] = "return Err(Status::invalid_argument(\n    msg,\n));\n"
        self.assert_reported_failure(self.check(files), "crates/wamux/src/domain/groups.rs:1")

    def test_below_the_floor_fails(self):
        files = clean_tree()
        files[ALLOWED] = mapping(6)
        self.assert_reported_failure(self.check(files), "holds 6 Status constructions")


if __name__ == "__main__":
    unittest.main()
