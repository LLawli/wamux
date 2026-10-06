"""#72 / #120 / #122: scripts/check-proto-jids.py fails CI when a field of the socket
contract carries a jid as a bare `string` instead of the `Jid` message.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no database.
"""
import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-proto-jids.py"
PROTO = "crates/wamux-proto/proto"


def load_check():
    spec = importlib.util.spec_from_file_location("check_proto_jids", CHECK)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def run_check(root):
    return subprocess.run([str(CHECK), "--root", str(root)], capture_output=True, text=True, check=False)


def clean_tree():
    files: dict[str, str] = {}
    for i in range(8):
        files[f"clean_{i}.proto"] = 'syntax = "proto3";\nmessage Empty {}\n'
    files["common.proto"] = (
        "message Jid {\n  string value = 1;\n}\n"
        "message MessageKey {\n  reserved 1, 4;\n  reserved \"remote_jid\";\n"
        "  Jid chat = 5;\n  string id = 2;\n  Jid participant = 6;\n}\n"
        "message Mention {\n  Jid jid = 2; // was string jid\n}\n"
    )
    files["account.proto"] = "message PairPhoneRequest {\n  string phone_number = 2;\n  string push_name = 3;\n}\n"
    files["store/blobs.proto"] = "message JidWire {\n  string user = 1;\n  string jid = 2;\n}\n"
    return files


class CheckProtoJids(unittest.TestCase):
    def assert_reported_failure(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        for needle in needles:
            self.assertIn(needle, output)

    def check(self, files):
        with tempfile.TemporaryDirectory() as root:
            for rel, text in files.items():
                path = Path(root) / PROTO / rel
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
            return run_check(root)

    def test_the_real_contract_passes(self):
        result = run_check(REPO)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_clean_tree_passes(self):
        result = self.check(clean_tree())
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_a_string_field_named_jid_fails(self):
        files = clean_tree()
        files["clean_0.proto"] += "message Foo {\n  string owner_jid = 3;\n  repeated string jids = 4;\n}\n"
        self.assert_reported_failure(self.check(files), "Foo.owner_jid", "Foo.jids")

    def test_a_string_field_named_for_a_jid_role_fails(self):
        files = clean_tree()
        files["clean_1.proto"] += "message Bar {\n  string chat = 1;\n  repeated string recipients = 2;\n  optional string sender = 3;\n}\n"
        self.assert_reported_failure(self.check(files), "Bar.chat", "Bar.recipients", "Bar.sender")

    def test_the_offender_is_named_by_its_own_message_after_a_one_line_message(self):
        files = clean_tree()
        files["clean_2.proto"] += "message Outer {\n  message Inner {}\n  string participant = 1;\n}\n"
        self.assert_reported_failure(self.check(files), "Outer.participant")

    # #122 migrated the last pending field: no list of exceptions is left for a
    # new string jid to hide in, and the summary no longer mentions one.
    def test_nothing_is_pending_any_more(self):
        self.assertFalse(hasattr(load_check(), "PENDING"))
        result = run_check(REPO)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("pending", result.stdout)
        # The events fields were the last ones: one of them as a string fails.
        files = clean_tree()
        files["events.proto"] = "message ServerAckEvent {\n  string from = 3;\n}\n"
        self.assert_reported_failure(self.check(files), "ServerAckEvent.from")

    def test_a_scan_that_reads_too_few_files_fails(self):
        self.assert_reported_failure(self.check({"one.proto": "message Empty {}\n"}), "expected at least")

    def test_comments_and_store_blobs_are_not_read(self):
        files = clean_tree()
        files["clean_3.proto"] += "message Baz {\n  // string chat = 1; was the old shape\n  Jid chat = 2;\n}\n"
        result = self.check(files)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
