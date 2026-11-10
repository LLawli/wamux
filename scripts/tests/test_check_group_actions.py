"""scripts/check-group-actions.py fails CI when a GroupNotificationAction
variant of the pinned wacore has no arm of its own in action_of.

Run: python3 -m unittest discover -s scripts/tests -v
The repo case resolves the pinned wacore through `cargo metadata --offline`;
every other case feeds a fake groups.rs through `--groups` and a fake mapper
through `--root`, and needs nothing.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-group-actions.py"
FLOOR = 40


def enum_text(variants: list[str]) -> str:
    """A groups.rs whose enum carries the shapes the parser must get right:
    doc comments naming other variants, `#[wire]` attributes, struct variants
    whose capitalized field types sit one brace deeper, and a second enum."""
    arms = []
    for i, name in enumerate(variants):
        arms.append(f"    /// `<{name.lower()}>`, see also `Other{name}` {{ braces }}")
        arms.append(f'    #[wire = "{name.lower()}"]')
        arms.append("    #[non_exhaustive]")
        if i % 2:
            arms.append(f"    {name} {{ participants: Vec<GroupParticipantInfo>, raw: Option<Node> }},")
        else:
            arms.append(f"    {name},")
    return (
        "pub enum NotThisOne { Decoy, AlsoDecoy }\n\n"
        "#[derive(Debug, Clone, WireEnum)]\n#[non_exhaustive]\n"
        "pub enum GroupNotificationAction {\n" + "\n".join(arms) + "\n}\n"
    )


def mapper_text(variants: list[str]) -> str:
    arms = "\n".join(f"        Lib::{v} {{ .. }} => Action::{v}(pb::Empty {{}})," for v in variants)
    return (
        "// Lib::InAComment is not an arm.\n"
        "fn action_of(action: &Lib) -> Action {\n    match action {\n"
        + arms
        + "\n        other => Action::Unknown(other.tag_name()),\n    }\n}\n\n"
        "fn elsewhere() { let _ = Lib::NotInActionOf; }\n"
    )


def names(count: int) -> list[str]:
    return [f"Variant{i}" for i in range(count)]


class CheckGroupActions(unittest.TestCase):
    def run_check(self, variants: list[str], mapped: list[str]) -> subprocess.CompletedProcess:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            mapper = root / "crates" / "wamux" / "src" / "domain" / "group_update.rs"
            mapper.parent.mkdir(parents=True)
            mapper.write_text(mapper_text(mapped), encoding="utf-8")
            groups = root / "groups.rs"
            groups.write_text(enum_text(variants), encoding="utf-8")
            return subprocess.run(
                ["python3", str(CHECK), "--root", str(root), "--groups", str(groups)],
                capture_output=True,
                text=True,
                check=False,
            )

    def test_repo_passes_against_the_pinned_wacore(self):
        result = subprocess.run(["python3", str(CHECK)], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("group actions ok: 44 variants, each mapped in action_of", result.stdout)

    def test_every_variant_mapped_passes(self):
        variants = names(FLOOR)
        result = self.run_check(variants, variants)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"group actions ok: {FLOOR} variants", result.stdout)

    def test_a_variant_added_upstream_fails_by_name(self):
        variants = names(FLOOR + 1)
        result = self.run_check(variants, variants[:-1])
        self.assertEqual(result.returncode, 1)
        self.assertIn(f"unmapped variant: Variant{FLOOR}", result.stdout)

    def test_an_arm_for_a_removed_variant_is_stale(self):
        variants = names(FLOOR)
        result = self.run_check(variants, variants + ["Gone"])
        self.assertEqual(result.returncode, 1)
        self.assertIn("stale arm, not a variant: Lib::Gone", result.stdout)

    def test_comments_attributes_fields_and_other_code_are_not_names(self):
        # A decoy read as a variant would be unmapped, one read as an arm stale:
        # either fails the run and changes the count.
        variants = names(FLOOR)
        result = self.run_check(variants, variants)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"group actions ok: {FLOOR} variants", result.stdout)

    def test_too_few_variants_fails_the_floor(self):
        variants = names(FLOOR - 1)
        result = self.run_check(variants, variants)
        self.assertEqual(result.returncode, 1)
        self.assertIn(f"found {FLOOR - 1} GroupNotificationAction variants", result.stdout)

    def test_a_missing_enum_is_reported_not_raised(self):
        with tempfile.TemporaryDirectory() as tmp:
            groups = Path(tmp) / "groups.rs"
            groups.write_text("pub enum Other {}\n", encoding="utf-8")
            result = subprocess.run(
                ["python3", str(CHECK), "--groups", str(groups)],
                capture_output=True,
                text=True,
                check=False,
            )
        self.assertEqual(result.returncode, 1)
        self.assertIn("pub enum GroupNotificationAction not found", result.stdout)
        self.assertNotIn("Traceback", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
