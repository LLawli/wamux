"""#140: how scripts/check-store-defaults.py resolves the pinned wacore.

The first hosted run of a PR failed with `cargo metadata exited 101` on a cold
cache: `cargo metadata --offline` without a platform filter wants the sources
of every target's packages (wasm, windows, redox), which no build step had
downloaded. These cases run the check against a fake `cargo` on PATH that
records its arguments, so they need no network and no particular cache.

Run: python3 -m unittest discover -s scripts/tests -v
"""
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-store-defaults.py"

# Records argv, prints the canned metadata, writes the canned stderr, exits
# with the canned code. Everything comes from the environment.
FAKE_CARGO = """#!/bin/sh
printf '%s\\n' "$@" > "$FAKE_CARGO_ARGS"
[ -n "$FAKE_CARGO_STDERR" ] && printf '%s\\n' "$FAKE_CARGO_STDERR" >&2
[ -n "$FAKE_CARGO_STDOUT" ] && cat "$FAKE_CARGO_STDOUT"
exit "${FAKE_CARGO_EXIT:-0}"
"""


def real_traits() -> Path:
    """The pinned wacore's traits.rs, found the way the fixed check finds it."""
    done = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--offline", "--filter-platform", "host-tuple"],
        cwd=REPO, capture_output=True, text=True, check=True,
    )
    for package in json.loads(done.stdout)["packages"]:
        if package["name"] == "wacore":
            return Path(package["manifest_path"]).parent / "src" / "store" / "traits.rs"
    raise AssertionError("the pinned wacore is not in cargo metadata")


class ResolveWacore(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)
        bin_dir = self.tmp / "bin"
        bin_dir.mkdir()
        cargo = bin_dir / "cargo"
        cargo.write_text(FAKE_CARGO)
        cargo.chmod(0o755)
        self.args_file = self.tmp / "args"
        self.env = {
            **os.environ,
            "PATH": f"{bin_dir}{os.pathsep}{os.environ['PATH']}",
            "FAKE_CARGO_ARGS": str(self.args_file),
        }

    def metadata_with(self, packages):
        out = self.tmp / "metadata.json"
        out.write_text(json.dumps({"packages": packages}))
        self.env["FAKE_CARGO_STDOUT"] = str(out)

    def run_check(self):
        return subprocess.run(
            [str(CHECK), "--root", str(REPO)], env=self.env, capture_output=True, text=True, check=False
        )

    def assert_reported_failure(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        for needle in needles:
            self.assertIn(needle, output)

    def test_metadata_is_asked_offline_for_the_host_only(self):
        wacore = self.tmp / "wacore"
        (wacore / "src" / "store").mkdir(parents=True)
        shutil.copy(real_traits(), wacore / "src" / "store" / "traits.rs")
        self.metadata_with([{"name": "wacore", "manifest_path": str(wacore / "Cargo.toml")}])
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        args = self.args_file.read_text().split()
        self.assertEqual(args[0], "metadata")
        for flag in ("--locked", "--offline"):
            self.assertIn(flag, args)
        # The cause of #140: without the filter, cargo wants every target's sources.
        self.assertIn("--filter-platform", args)
        self.assertEqual(args[args.index("--filter-platform") + 1], "host-tuple")

    def test_a_failing_cargo_reports_its_stderr(self):
        self.env["FAKE_CARGO_EXIT"] = "101"
        self.env["FAKE_CARGO_STDERR"] = (
            "error: failed to download `atomic v0.6.1`\n\nCaused by:\n"
            "  attempting to make an HTTP request, but --offline was specified"
        )
        self.assert_reported_failure(
            self.run_check(), "cannot resolve wacore", "exited 101", "failed to download `atomic v0.6.1`"
        )

    def test_no_wacore_in_the_metadata_still_fails(self):
        self.metadata_with([{"name": "tokio", "manifest_path": str(self.tmp / "tokio" / "Cargo.toml")}])
        self.assert_reported_failure(self.run_check(), "no `wacore` package")


if __name__ == "__main__":
    unittest.main()
