"""#68: scripts/check-service-coverage.py fails CI when an RPC of a service is
never called by the service's socket test suite, or when the service gained or
lost an RPC the count does not know about.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no database.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-service-coverage.py"

PROTO = """syntax = "proto3";
package wamux.v1;

service DemoService {
  rpc CreateGroup(CreateGroupRequest) returns (GroupJidResponse);
  // rpc CommentedOut(Empty) returns (Empty);
  rpc GetInviteLink(GroupRef) returns (InviteLinkResponse);
  rpc ListParticipating(AccountRef) returns (ListGroupsResponse);
}

service OtherService {
  rpc NotMine(Empty) returns (Empty);
}
"""


def run_check(root, *args):
    return subprocess.run(
        [str(CHECK), *args, "--root", str(root)],
        capture_output=True,
        text=True,
        check=False,
    )


def write_tree(root, test_body, proto=PROTO):
    protos = root / "crates" / "wamux-proto" / "proto"
    protos.mkdir(parents=True)
    (protos / "demo.proto").write_text(proto)
    tests = root / "crates" / "wamux" / "tests" / "demo_service"
    tests.mkdir(parents=True)
    (tests / "main.rs").write_text(test_body)


ALL_CALLED = (
    "    groups.create_group(req()).await;\n"
    "    groups\n        .get_invite_link(req())\n        .await;\n"
    "    groups.list_participating(acct()).await;\n"
)
DIR = "crates/wamux/tests/demo_service"


class CheckServiceCoverage(unittest.TestCase):
    def assert_reported_failure(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        for needle in needles:
            self.assertIn(needle, output)

    def test_group_service_passes_on_the_repo(self):
        result = run_check(REPO, "GroupService", "crates/wamux/tests/group_service", "21")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("service coverage ok: GroupService, 21 RPCs", result.stdout)

    def test_every_rpc_called_passes(self):
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), ALL_CALLED)
            result = run_check(tmp, "DemoService", DIR, "3")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("service coverage ok: DemoService, 3 RPCs", result.stdout)

    def test_uncalled_rpc_fails_and_is_named(self):
        body = ALL_CALLED.replace("groups.list_participating(acct()).await;\n", "")
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), body)
            result = run_check(tmp, "DemoService", DIR, "3")
        self.assert_reported_failure(result, "list_participating")
        self.assertNotIn("create_group", result.stdout + result.stderr)

    def test_call_in_a_comment_does_not_count(self):
        body = ALL_CALLED.replace(
            "    groups.list_participating(acct()).await;\n",
            "    // groups.list_participating(acct()) is covered elsewhere\n",
        )
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), body)
            result = run_check(tmp, "DemoService", DIR, "3")
        self.assert_reported_failure(result, "list_participating")

    def test_count_mismatch_fails_with_both_numbers(self):
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), ALL_CALLED)
            result = run_check(tmp, "DemoService", DIR, "4")
        self.assert_reported_failure(result, "3", "4")

    def test_other_services_and_commented_rpcs_are_not_counted(self):
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), ALL_CALLED)
            result = run_check(tmp, "DemoService", DIR, "3")
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 0, output)
        self.assertNotIn("not_mine", output)
        self.assertNotIn("commented_out", output)

    def test_unknown_service_fails(self):
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), ALL_CALLED)
            result = run_check(tmp, "MissingService", DIR, "3")
        self.assert_reported_failure(result, "MissingService")


if __name__ == "__main__":
    unittest.main()
