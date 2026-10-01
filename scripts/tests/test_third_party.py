"""#83: THIRD-PARTY-LICENSES.md is generated from the shipped daemon's graph
and held to it by scripts/check-third-party.sh.

Run: python3 -m unittest discover -s scripts/tests -v
Needs cargo and the dependency sources (a `cargo fetch` worth of registry and
git checkouts); no database.
"""
import importlib.util
import re
import subprocess
import tempfile
import tomllib
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
GENERATOR = REPO / "scripts" / "gen-third-party.py"
CHECK = REPO / "scripts" / "check-third-party.sh"
COMMITTED = REPO / "THIRD-PARTY-LICENSES.md"
FIX_COMMAND = "scripts/gen-third-party.py"


def load_generator():
    spec = importlib.util.spec_from_file_location("gen_third_party", GENERATOR)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def locked_version(name):
    lock = tomllib.loads((REPO / "Cargo.lock").read_text())
    versions = {p["version"] for p in lock["package"] if p["name"] == name}
    assert len(versions) == 1, f"{name} resolves to {versions} in Cargo.lock"
    return versions.pop()


def table_rows(text):
    """Rows of the crates table only: a license text may contain pipes too."""
    section = text.split("## Crates", 1)[-1].split("## License texts", 1)[0]
    return [line for line in section.splitlines() if re.match(r"^\| [^-|][^|]* \| ", line)
            and not line.startswith("| crate |")]


def row_names(text):
    return {line.split("|")[1].strip() for line in table_rows(text)}


def run_check(*args):
    return subprocess.run(["bash", str(CHECK), *map(str, args)], cwd=REPO,
                          capture_output=True, text=True)


class Generated(unittest.TestCase):
    """One generation shared by the read-only assertions."""

    @classmethod
    def setUpClass(cls):
        cls.gen = load_generator()
        cls.tmp = tempfile.TemporaryDirectory()
        cls.out = Path(cls.tmp.name) / "tpl.md"
        code = cls.gen.main(["--output", str(cls.out)])
        assert code == 0, f"generator exited {code}"
        cls.text = cls.out.read_text()
        cls.meta = cls.gen.cargo_metadata()

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def package(self, name):
        found = [p for p in self.meta["packages"] if p["name"] == name]
        self.assertEqual(len(found), 1, f"{name}: {found}")
        return found[0]

    def test_root_is_the_shipped_daemon(self):
        version = locked_version("whatsapp-rust")
        self.assertIn(f"| whatsapp-rust | {version} |", self.text)
        self.assertIn("tonic", row_names(self.text))

    def test_tools_only_crates_are_absent(self):
        names = row_names(self.text)
        self.assertNotIn("qrcode", names)
        self.assertNotIn("image", names)
        ureq = [r for r in table_rows(self.text) if r.startswith("| ureq | ")]
        self.assertTrue(ureq, "the daemon links ureq 3")
        self.assertFalse([r for r in ureq if r.startswith("| ureq | 2.")], ureq)

    def test_workspace_members_are_absent(self):
        for member in ("wamux", "wamux-proto", "wamux-tools"):
            self.assertNotIn(member, row_names(self.text))
            self.assertNotIn(f"- {member} ", self.text)

    def test_git_family_carries_the_checkout_license(self):
        texts = self.gen.license_texts(self.package("wacore"))
        self.assertTrue(texts, "wacore must carry the checkout's LICENSE")
        self.assertTrue(any("MIT" in text or "Permission is hereby granted" in text
                            for _, text in texts), texts)
        missing = self.text.split("## Crates with no license file", 1)
        tail = missing[1] if len(missing) == 2 else ""
        for name in ("wacore", "waproto", "whatsapp-rust-tokio-transport",
                     "whatsapp-rust-ureq-http-client"):
            self.assertNotIn(f"- {name} ", tail, f"{name} is still listed without a text")

    def test_floor_and_count(self):
        match = re.search(r"\*\*(\d+) crates\*\*", self.text)
        self.assertIsNotNone(match, "header count missing")
        count = int(match.group(1))
        self.assertGreaterEqual(count, 300, "absolute floor, not a derived count")
        self.assertEqual(count, len(table_rows(self.text)))

    def test_generation_is_deterministic(self):
        again = Path(self.tmp.name) / "again.md"
        self.assertEqual(self.gen.main(["--output", str(again)]), 0)
        self.assertEqual(again.read_bytes(), self.out.read_bytes())

    def test_output_flag_writes_only_there(self):
        before = COMMITTED.read_bytes()
        other = Path(self.tmp.name) / "elsewhere.md"
        self.assertEqual(self.gen.main(["--output", str(other)]), 0)
        self.assertTrue(other.is_file())
        self.assertEqual(COMMITTED.read_bytes(), before)

    def test_stale_file_fails_the_check_and_names_the_fix(self):
        version = locked_version("whatsapp-rust")
        stale = Path(self.tmp.name) / "stale.md"
        stale.write_text(self.text.replace(f"| whatsapp-rust | {version} |",
                                           "| whatsapp-rust | 0.6.0 |"))
        result = run_check(stale)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(FIX_COMMAND, result.stdout + result.stderr)

    def test_empty_file_fails_the_check(self):
        empty = Path(self.tmp.name) / "empty.md"
        empty.write_text("")
        result = run_check(empty)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)


class WalkUp(unittest.TestCase):
    """The license walk-up is for git checkouts only, and stops at the checkout."""

    def setUp(self):
        self.gen = load_generator()
        self.tmp = tempfile.TemporaryDirectory()
        self.base = Path(self.tmp.name)

    def tearDown(self):
        self.tmp.cleanup()

    def crate_in(self, checkout, source):
        sub = checkout / "crates" / "sub"
        sub.mkdir(parents=True)
        (sub / "Cargo.toml").write_text('[package]\nname = "sub"\n')
        return {"name": "sub", "version": "1.0.0", "source": source,
                "manifest_path": str(sub / "Cargo.toml"), "license": "MIT"}

    def test_git_package_uses_its_checkout_root_license(self):
        checkout = self.base / "rev"
        checkout.mkdir()
        (checkout / ".git").write_text("")
        (checkout / "LICENSE").write_text("MIT License\nPermission is hereby granted")
        package = self.crate_in(checkout, "git+https://example.invalid/x?rev=1#1")
        self.assertEqual([name for name, _ in self.gen.license_texts(package)], ["LICENSE"])

    def test_git_package_does_not_climb_past_its_checkout(self):
        (self.base / "LICENSE").write_text("someone else's license")
        checkout = self.base / "rev"
        checkout.mkdir()
        (checkout / ".git").write_text("")
        package = self.crate_in(checkout, "git+https://example.invalid/x?rev=1#1")
        self.assertEqual(self.gen.license_texts(package), [])

    def test_registry_packages_do_not_walk_up(self):
        checkout = self.base / "pkg"
        checkout.mkdir()
        (checkout / ".git").write_text("")
        (checkout / "LICENSE").write_text("MIT License")
        package = self.crate_in(
            checkout, "registry+https://github.com/rust-lang/crates.io-index")
        self.assertEqual(self.gen.license_texts(package), [])

    def test_missing_root_fails_instead_of_writing_nothing(self):
        meta = {"packages": [], "workspace_members": [],
                "resolve": {"nodes": [], "root": None}}
        with self.assertRaises(SystemExit):
            self.gen.shipped_packages(meta)


class Committed(unittest.TestCase):
    def test_committed_file_passes_the_check(self):
        result = run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
