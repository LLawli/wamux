"""#135 (closes #74): scripts/check-wire-json.py fails CI when the daemon puts
JSON on the wire anywhere but the RawEvent catch-all, `raw_event_of`.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no database.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-wire-json.py"
SRC = "crates/wamux/src"
MAPPING = f"{SRC}/domain/event_mapping.rs"

# The allowed site: the catch-all for a library event wamux does not know.
RAW_EVENT_OF = """fn raw_event_of(other: &Event) -> pb::RawEvent {
    pb::RawEvent {
        kind: variant_name(other),
        payload: serde_json::to_vec(other).unwrap_or_default(),
        note: String::new(),
    }
}
"""


def run_check(root):
    return subprocess.run([str(CHECK), "--root", str(root)], capture_output=True, text=True, check=False)


class CheckWireJson(unittest.TestCase):
    def check(self, files):
        """Run the check over a tree holding `files` ({path under SRC: text}).
        The mapping is written with only the allowed site unless `files` names it."""
        with tempfile.TemporaryDirectory() as root:
            tree = {"domain/event_mapping.rs": RAW_EVENT_OF, **files}
            for relative, text in tree.items():
                if text is None:
                    continue
                path = Path(root) / SRC / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
            return run_check(root)

    def assert_passed(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def assert_reported_failure(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        for needle in needles:
            self.assertIn(needle, output)

    def test_the_real_tree_passes(self):
        self.assert_passed(run_check(REPO))

    def test_the_catch_all_alone_passes(self):
        self.assert_passed(self.check({}))

    # The site #135 removed: the call event serialized the whole library value.
    def test_to_vec_in_another_mapping_fails(self):
        call = "fn call_event_of(call: &IncomingCall) -> pb::CallEvent {\n" \
               "    let raw = serde_json::to_vec(call).unwrap_or_default();\n" \
               "}\n"
        self.assert_reported_failure(self.check({"domain/call_event.rs": call}), f"{SRC}/domain/call_event.rs:2:")

    def test_to_vec_after_the_catch_all_fails(self):
        mapping = RAW_EVENT_OF + "fn later(x: &X) -> Vec<u8> {\n    serde_json::to_vec(x).unwrap()\n}\n"
        self.assert_reported_failure(self.check({"domain/event_mapping.rs": mapping}), f"{MAPPING}:9:")

    def test_every_json_serializer_fails(self):
        service = (
            "fn a(x: &X) {\n"
            "    let s = serde_json::to_string(x);\n"
            "    let v = serde_json::to_value(x);\n"
            "    let w = serde_json::to_writer(out, x);\n"
            "    let p = serde_json::to_vec_pretty(x);\n"
            '    let j = serde_json::json!({"a": 1});\n'
            '    let k = json!({"a": 1});\n'
            "}\n"
        )
        lines = [f"{SRC}/services/groups.rs:{n}:" for n in range(2, 8)]
        self.assert_reported_failure(self.check({"services/groups.rs": service}), *lines)

    def test_a_serializer_in_a_comment_is_not_read(self):
        text = "// raw: serde_json::to_vec(call) was the old shape\nfn a() {}\n"
        self.assert_passed(self.check({"domain/call_event.rs": text}))

    def test_test_files_are_not_read(self):
        test = "fn t() {\n    serde_json::to_vec(&event).unwrap();\n}\n"
        self.assert_passed(self.check({
            "domain/event_mapping_tests.rs": test,
            "domain/group/tests.rs": test,
            "domain/test_xml.rs": test,
        }))

    # The store's own rows are not the wire: the device list is a JSON column.
    def test_the_store_is_not_read(self):
        row = "fn devices_json(r: &R) -> String {\n    serde_json::to_string(&*r.devices).unwrap()\n}\n"
        self.assert_passed(self.check({"storage/protocol_rows.rs": row}))

    def test_reading_json_is_not_writing_it(self):
        text = "fn a(s: &str) {\n    let v: V = serde_json::from_str(s).unwrap();\n}\n"
        self.assert_passed(self.check({"domain/call_event.rs": text}))

    def test_a_missing_mapping_fails(self):
        self.assert_reported_failure(self.check({"domain/event_mapping.rs": None}), "not found")

    def test_a_missing_catch_all_fails(self):
        mapping = "fn map_event() {}\n"
        self.assert_reported_failure(
            self.check({"domain/event_mapping.rs": mapping}), "fn raw_event_of not found"
        )


if __name__ == "__main__":
    unittest.main()
