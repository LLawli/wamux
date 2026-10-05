"""#106: scripts/check-store-sql-shared.py fails CI when a store statement is
written outside storage/statements/, or spelled for one engine only.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no database.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-store-sql-shared.py"
FLOOR = 70


def run_check(root):
    return subprocess.run(
        [str(CHECK), "--root", str(root)], capture_output=True, text=True, check=False
    )


def statements(count):
    return "\n".join(
        f'pub(crate) const S{i}: &str = "SELECT a FROM t WHERE id = $1 AND x = $2";'
        for i in range(count)
    ) + "\n"


def write_tree(root, files):
    """A minimal checkout: `files` maps a path under storage/ to its text."""
    storage = root / "crates" / "wamux" / "src" / "storage"
    for rel, text in files.items():
        path = storage / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)


def clean_tree(count=FLOOR):
    return {
        "statements/signal.rs": statements(count),
        "sql/signal_store.rs": "execute_sql!(&self.pool, statements::PUT_IDENTITY, a)?;\n",
        "sql/prekeys_sql.rs": 'sqlx::query("UPDATE prekeys SET uploaded = TRUE WHERE id = ANY($2)")\n',
        "turso/signal_store.rs": "self.conn.execute(statements::PUT_IDENTITY, p).await?;\n",
        "turso/placeholders.rs": 'out.push_str("?1");\n',
        "turso/migrations.rs": 'conn.query("SELECT version FROM _sqlx_migrations WHERE version = ?1", p)\n',
        "turso/signal_store_tests.rs": 'conn.query("SELECT 1 WHERE a = ?1", p)\n',
    }


class CheckStoreSqlShared(unittest.TestCase):
    def assert_reported_failure(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        for needle in needles:
            self.assertIn(needle, output)

    def check(self, files):
        with tempfile.TemporaryDirectory() as tmp:
            write_tree(Path(tmp), files)
            return run_check(tmp)

    def test_repo_passes(self):
        result = subprocess.run([str(CHECK)], capture_output=True, text=True, check=False)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_clean_tree_passes_with_the_exceptions(self):
        result = self.check(clean_tree())
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn(f"store SQL ok: {FLOOR} statements", result.stdout)

    def test_inline_sql_in_a_family_fails(self):
        for family in ("sql", "turso"):
            files = clean_tree()
            files[f"{family}/device_store.rs"] = 'let q = "SELECT data FROM device WHERE device_id = $1";\n'
            self.assert_reported_failure(self.check(files), f"inline SQL in storage/{family}/device_store.rs:1")

    def test_inline_format_sql_fails(self):
        files = clean_tree()
        files["sql/signal_sql.rs"] = 'format!("SELECT id FROM prekeys WHERE id IN ({list})")\n'
        self.assert_reported_failure(self.check(files), "inline SQL in storage/sql/signal_sql.rs:1")

    def test_hand_written_qmark_fails(self):
        files = clean_tree()
        files["turso/protocol_store.rs"] = "let sql = format!(\"{} ?1\", base);\n"
        self.assert_reported_failure(self.check(files), "hand-written ?N in storage/turso/protocol_store.rs:1")

    def test_a_dollar_that_is_not_a_placeholder_fails(self):
        files = clean_tree()
        files["statements/broken.rs"] = 'pub(crate) const X: &str = "SELECT \'$x\' FROM t";\n'
        self.assert_reported_failure(self.check(files), "not a placeholder in storage/statements/broken.rs:1")

    def test_comments_are_ignored(self):
        files = clean_tree()
        files["sql/device_store.rs"] = '// was "SELECT data FROM device WHERE device_id = ?1"\n'
        result = self.check(files)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_below_the_floor_fails(self):
        self.assert_reported_failure(self.check(clean_tree(FLOOR - 1)), f"holds {FLOOR - 1} statements")

    def test_missing_statements_dir_fails(self):
        files = clean_tree()
        del files["statements/signal.rs"]
        self.assert_reported_failure(self.check(files), "storage/statements/ is missing")


if __name__ == "__main__":
    unittest.main()
