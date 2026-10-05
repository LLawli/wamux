"""#115 / #116: scripts/check-domain-inputs.py fails CI when a function in
domain/ or state/ takes a generated `pb::*` input struct, or an identifier as a
`String`, instead of a wamux-types type.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no database.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-domain-inputs.py"
DOMAIN = "crates/wamux/src/domain"
PENDING_FILES = {
    f"{DOMAIN}/event_mapping.rs": "pub fn map_sent(\n    key: pb::MessageKey,\n    chat: &str,\n",
}


def run_check(root):
    return subprocess.run([str(CHECK), "--root", str(root)], capture_output=True, text=True, check=False)


def write_tree(root, files):
    for rel, text in files.items():
        path = Path(root) / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)


def clean_tree():
    files = dict(PENDING_FILES)
    for i in range(20):
        files[f"{DOMAIN}/clean_{i}.rs"] = "pub fn f(target: &MessageTarget) -> pb::SendResult {\n"
    files[f"{DOMAIN}/contacts.rs"] = "pub async fn subscribe_presence(client: &Client, jid: &Jid) {\n"
    files[f"{DOMAIN}/messaging.rs"] = (
        "pub async fn send_text(client: &Client, to: Jid, text: &OutgoingText) -> Result<SendResult, WamuxError> {\n"
        "pub fn sent_message_key(message_id: String, to: &Jid) -> pb::MessageKey {\n"
        "    pb::MessageKey {\n"
        "// was: req: &pb::SendTextRequest\n"
    )
    files["crates/wamux/src/state/send_echo.rs"] = (
        "pub async fn publish_sent(handle: &Arc<AccountHandle>, message_id: &MessageId) {\n"
    )
    files[f"{DOMAIN}/messaging_tests.rs"] = "fn key() -> pb::MessageKey { todo!() }\nlet k: pb::MessageKey;\n"
    files[f"{DOMAIN}/interactive_reply/tests.rs"] = "fn r(req: &pb::SendInteractiveReplyRequest) {}\n"
    return files


class CheckDomainInputs(unittest.TestCase):
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
        self.assertIn("1 pending", result.stdout)

    def test_each_input_shape_fails(self):
        shapes = [
            "    req: &pb::SendTextRequest,",
            "    header: &pb::SendMediaHeader,",
            "    target: pb::MessageKey,",
            "    mentions: &[pb::Mention],",
            "    quote: Option<&pb::QuoteContext>,",
            "    votes: Vec<pb::PollVote>,",
            "    preview: &mut pb::LinkPreview,",
            "    descriptor: &pb::MediaDescriptor,",
            "    reply: &pb::send_interactive_reply_request::Reply,",
            "    button: &pb::ButtonReply,",
        ]
        for shape in shapes:
            with self.subTest(shape=shape):
                files = clean_tree()
                files[f"{DOMAIN}/polls.rs"] = f"pub fn f(\n{shape}\n) {{}}\n"
                self.assert_reported_failure(self.check(files), f"{DOMAIN}/polls.rs:2")

    def test_state_is_scanned_too(self):
        files = clean_tree()
        files["crates/wamux/src/state/send_echo.rs"] = "    key: pb::MessageKey,\n"
        self.assert_reported_failure(self.check(files), "crates/wamux/src/state/send_echo.rs:1")

    def test_a_pending_file_that_no_longer_takes_one_fails(self):
        files = clean_tree()
        files[f"{DOMAIN}/event_mapping.rs"] = "pub fn map_sent(key: &SentKey) {}\n"
        self.assert_reported_failure(
            self.check(files), f"{DOMAIN}/event_mapping.rs takes no pb:: input any more"
        )

    def test_each_string_identifier_fails(self):
        shapes = [
            "    group: &str,",
            "    participants: &[String],",
            "    jids: Vec<String>,",
            "pub async fn get_about(client: Arc<Client>, jid: &str) -> Result<(), WamuxError> {",
            "fn row_to_proto(row: &NewsletterMessage, chat: &str) -> pb::NewsletterMessage {",
            "    sender: String,",
            "    query: &str,",
            "    recipients: Vec<String>,",
        ]
        for shape in shapes:
            with self.subTest(shape=shape):
                files = clean_tree()
                files[f"{DOMAIN}/groups.rs"] = f"pub fn f(\n{shape}\n) {{}}\n"
                self.assert_reported_failure(self.check(files), f"{DOMAIN}/groups.rs:2")

    def test_other_strings_and_typed_ids_pass(self):
        files = clean_tree()
        files[f"{DOMAIN}/groups.rs"] = (
            "pub async fn set_subject(client: &Client, group: &Jid, subject: &str) -> Result<(), WamuxError> {\n"
            "pub async fn join_with_invite(client: &Client, code: &str) -> Result<(), WamuxError> {\n"
            "    participants: &[Jid],\n"
            "    jid: c.jid.to_string(),\n"
            "    query: query.query.clone(),\n"
        )
        result = self.check(files)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_below_the_floor_fails(self):
        files = {k: v for k, v in clean_tree().items() if "clean_" not in k}
        self.assert_reported_failure(self.check(files), "expected at least 20")


if __name__ == "__main__":
    unittest.main()
