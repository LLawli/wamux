"""#115: scripts/check-domain-inputs.py fails CI when a function in domain/ or
state/ takes a generated `pb::*` input struct instead of a wamux-types type.

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
    f"{DOMAIN}/newsletters.rs": "pub async fn get_messages(\n    req: &pb::GetNewsletterMessagesRequest,\n",
    f"{DOMAIN}/newsletters/poll_votes.rs": "    req: &pb::SendNewsletterPollVoteRequest,\n",
    f"{DOMAIN}/event_mapping.rs": "pub fn map_sent(\n    key: pb::MessageKey,\n",
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
        self.assertIn("3 pending", result.stdout)

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
        files[f"{DOMAIN}/newsletters.rs"] = "pub async fn get_messages(req: &NewsletterQuery) {}\n"
        self.assert_reported_failure(
            self.check(files), f"{DOMAIN}/newsletters.rs takes no pb:: input any more"
        )

    def test_below_the_floor_fails(self):
        files = {k: v for k, v in clean_tree().items() if "clean_" not in k}
        self.assert_reported_failure(self.check(files), "expected at least 20")


if __name__ == "__main__":
    unittest.main()
