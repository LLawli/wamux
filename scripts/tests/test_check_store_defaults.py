"""#93: scripts/check-store-defaults.py fails CI when a wacore store trait
method with a default body is not classified in docs/store-trait-defaults.md,
or when the table and the engines disagree.

Run: python3 -m unittest discover -s scripts/tests -v
The repo case resolves the pinned wacore through `cargo metadata --offline`;
every other case feeds a fake traits.rs through `--traits` and needs nothing.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-store-defaults.py"
FLOOR = 36
HEADER = "| Method | Trait | Decision | Why |\n|---|---|---|---|\n"

# Shapes the parser must get right. `FakeA::default_multiline` has a default
# whose signature spans lines and whose body nests braces; `required_*` end in
# `;`; the free function has a body but lives outside any trait.
TRAITS_PREAMBLE = """\
fn free_helper_with_body() -> StoreError {
    StoreError::Validation(format!("{}", 1))
}

pub struct NotATrait {
    pub field: u8,
}

impl NotATrait {
    pub async fn inherent_with_body(&self) -> u8 { self.field }
}
"""

TRAIT_A_FIXED = """\
    async fn required_a(&self, address: &str) -> Result<Option<[u8; 32]>>;

    async fn default_multiline(
        &self,
        entries: &[(Arc<str>, Bytes)],
    ) -> Result<Vec<(Arc<str>, Bytes)>> {
        let mut out = Vec::new();
        for entry in entries {
            if let Some(x) = self.get(entry).await? {
                out.push(x);
            }
        }
        Ok(out)
    }
"""


def fake_defaults(count):
    return [f"default_method_{i:02d}" for i in range(count)]


def traits_source(a_defaults, b_defaults):
    """Two traits: FakeA (multiline default first) and FakeB."""

    def bodies(names):
        return "".join(f"    async fn {n}(&self) -> Result<()> {{\n        Ok(())\n    }}\n" for n in names)

    return (
        TRAITS_PREAMBLE
        + "#[async_trait]\npub trait FakeA: Send + Sync {\n"
        + TRAIT_A_FIXED
        + bodies(a_defaults)
        + "}\n\n#[async_trait]\npub trait FakeB: Send + Sync {\n"
        + "    async fn required_b(&self) -> Result<u32>;\n"
        + bodies(b_defaults)
        + "}\n"
    )


def split(names):
    """First half on FakeA, the rest on FakeB."""
    half = len(names) // 2
    return names[:half], names[half:]


def rows(a_defaults, b_defaults, overrides=(), trait_of=None, why="kept for a reason"):
    """A doc row per default; `overrides` are marked override, the rest default."""
    out = []
    for trait, names in (("FakeA", ["default_multiline", *a_defaults]), ("FakeB", b_defaults)):
        for n in names:
            decision = "override" if n in overrides else "default"
            shown = (trait_of or {}).get(n, trait)
            out.append(f"| `{n}` | {shown} | {decision} | {why} |\n")
    return out


def write_tree(root, traits_text, doc_rows, impls_by_family):
    """A minimal checkout: one `*_store.rs` per family, the doc, a traits.rs."""
    storage = root / "crates" / "wamux" / "src" / "storage"
    for family, names in impls_by_family.items():
        engine_dir = storage / family
        engine_dir.mkdir(parents=True)
        body = "\n".join(f"    async fn {n}(&self) -> Result<()> {{ Ok(()) }}" for n in names)
        (engine_dir / "protocol_store.rs").write_text(f"impl FakeA for X {{\n{body}\n}}\n")
        # Not a store module: its async fns must not count as overrides.
        (engine_dir / "tc_token_sql.rs").write_text("async fn default_method_00() {}\n")
    docs = root / "docs"
    docs.mkdir()
    (docs / "store-trait-defaults.md").write_text("# Store trait defaults\n\n" + HEADER + "".join(doc_rows))
    traits = root / "traits.rs"
    traits.write_text(traits_text)
    return traits


def run_check(root, traits=None, families=None):
    args = [str(CHECK), "--root", str(root)]
    if traits is not None:
        args += ["--traits", str(traits)]
    if families is not None:
        args += ["--families", ",".join(families)]
    return subprocess.run(args, capture_output=True, text=True, check=False)


class CheckStoreDefaults(unittest.TestCase):
    def assert_reported_failure(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        for needle in needles:
            self.assertIn(needle, output)

    def check_fake(self, a, b, doc_rows, impls_by_family):
        with tempfile.TemporaryDirectory() as tmp:
            traits = write_tree(Path(tmp), traits_source(a, b), doc_rows, impls_by_family)
            return run_check(tmp, traits, list(impls_by_family))

    def test_repo_passes_against_the_pinned_wacore(self):
        result = run_check(REPO)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("store defaults ok: 36 trait defaults, 5 overridden, 31 kept", result.stdout)

    def test_fake_tree_fully_classified_passes(self):
        a, b = split(fake_defaults(FLOOR - 1))
        over = [a[0], b[0]]
        result = self.check_fake(a, b, rows(a, b, over), {"sql": over})
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"store defaults ok: {FLOOR} trait defaults, 2 overridden, {FLOOR - 2} kept", result.stdout)

    def test_required_methods_and_bodies_outside_traits_are_not_defaults(self):
        a, b = split(fake_defaults(FLOOR - 1))
        extra = [
            "| `required_a` | FakeA | default | not a default |\n",
            "| `free_helper_with_body` | FakeA | default | not in a trait |\n",
            "| `inherent_with_body` | FakeA | default | not in a trait |\n",
        ]
        result = self.check_fake(a, b, rows(a, b) + extra, {"sql": []})
        self.assert_reported_failure(
            result,
            "stale row, not a trait default: required_a",
            "stale row, not a trait default: free_helper_with_body",
            "stale row, not a trait default: inherent_with_body",
        )

    def test_unclassified_default_fails_and_is_named(self):
        a, b = split(fake_defaults(FLOOR - 1))
        doc = [r for r in rows(a, b) if f"`{b[-1]}`" not in r]
        result = self.check_fake(a, b, doc, {"sql": []})
        self.assert_reported_failure(result, f"unclassified trait default: {b[-1]}")

    def test_multiline_default_with_nested_braces_counts(self):
        a, b = split(fake_defaults(FLOOR - 1))
        doc = [r for r in rows(a, b) if "`default_multiline`" not in r]
        result = self.check_fake(a, b, doc, {"sql": []})
        self.assert_reported_failure(result, "unclassified trait default: default_multiline")

    def test_override_row_missing_in_one_family_fails(self):
        a, b = split(fake_defaults(FLOOR - 1))
        result = self.check_fake(a, b, rows(a, b, [a[0]]), {"sql": [a[0]], "turso": []})
        self.assert_reported_failure(result, f"marked override but not implemented by turso: {a[0]}")

    def test_override_row_missing_in_the_only_family_fails(self):
        a, b = split(fake_defaults(FLOOR - 1))
        result = self.check_fake(a, b, rows(a, b, [a[0]]), {"sql": []})
        self.assert_reported_failure(result, f"marked override but not implemented by sql: {a[0]}")

    def test_default_row_overridden_by_a_family_fails(self):
        a, b = split(fake_defaults(FLOOR - 1))
        result = self.check_fake(a, b, rows(a, b), {"sql": [], "turso": [b[0]]})
        self.assert_reported_failure(result, f"marked default but overridden by turso: {b[0]}")

    def test_missing_family_directory_fails_cleanly(self):
        a, b = split(fake_defaults(FLOOR - 1))
        with tempfile.TemporaryDirectory() as tmp:
            traits = write_tree(Path(tmp), traits_source(a, b), rows(a, b), {"sql": []})
            result = run_check(tmp, traits, ["sql", "turso"])
        self.assert_reported_failure(result, "family directory missing: storage/turso")

    def test_default_without_a_reason_fails(self):
        a, b = split(fake_defaults(FLOOR - 1))
        doc = rows(a, b)
        doc[1] = f"| `{a[0]}` | FakeA | default |  |\n"
        result = self.check_fake(a, b, doc, {"sql": []})
        self.assert_reported_failure(result, f"default kept without a reason: {a[0]}")

    def test_wrong_trait_column_fails(self):
        a, b = split(fake_defaults(FLOOR - 1))
        result = self.check_fake(a, b, rows(a, b, trait_of={b[0]: "FakeA"}), {"sql": []})
        self.assert_reported_failure(result, f"trait column says FakeA, wacore declares {b[0]} in FakeB")

    def test_too_few_defaults_fails_the_floor(self):
        a, b = split(fake_defaults(3))
        result = self.check_fake(a, b, rows(a, b), {"sql": []})
        self.assert_reported_failure(result, f"expected at least {FLOOR}")

    def test_missing_traits_file_fails_cleanly(self):
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), "", [], {"sql": []})
            result = run_check(tmp, Path(tmp) / "absent" / "traits.rs")
        self.assert_reported_failure(result, "cannot resolve wacore")


if __name__ == "__main__":
    unittest.main()
