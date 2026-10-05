#!/usr/bin/env python3
"""#106: every store statement is written once, in storage/statements/.

Usage: scripts/check-store-sql-shared.py [--root DIR]   (DIR defaults to the repo)

Why: two engine families (`sql`, `turso`) run the same statements. A copy kept
in a family would drift from the other one with every test still green,
because each family keeps reading back what it wrote. And turso binds `$N` by
order of appearance, not by number: a statement hand-written for it would
either repeat that trap or spell `?N`, a second dialect to keep in step. So:

- no SQL string literal in storage/sql/ or storage/turso/. The exceptions are
  the lines with no common form across drivers (`ANY(` array binds,
  `FOR UPDATE`) and turso/migrations.rs, which writes sqlx's own
  `_sqlx_migrations` table, something the sqlx family never spells out;
- no hand-written `?N` anywhere in storage/, except turso/placeholders* (the
  rewrite) and turso/migrations.rs;
- in storage/statements/, every `$` starts a placeholder (`$1`) or a
  `format!` field (`${n}`): the rewrite turns each `$` into `?`;
- storage/statements/ holds at least MIN_STATEMENTS statements, so an empty or
  unparsed directory cannot pass.

Test files (`*_tests.rs`, `tests.rs`) are skipped: they probe the engines with
raw SQL on purpose.
"""
import argparse
import re
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MIN_STATEMENTS = 70
SQL_LITERAL = re.compile(r'"\s*(SELECT|INSERT|UPDATE|DELETE|WITH)\b')
PER_DRIVER = ("ANY(", "FOR UPDATE")
HAND_WRITTEN_QMARK = re.compile(r"\?[0-9]")
STRAY_DOLLAR = re.compile(r"\$(?![0-9{])")
TURSO_ONLY_SQL = "turso/migrations.rs"


def code_lines(path: Path):
    """(line number, text) of every line that is not a `//` comment."""
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.lstrip().startswith("//"):
            yield number, line


def sources(directory: Path):
    """Every non-test `.rs` under `directory`, sorted."""
    for path in sorted(directory.rglob("*.rs")):
        if path.name != "tests.rs" and not path.name.endswith("_tests.rs"):
            yield path


def inline_sql(storage: Path) -> list[str]:
    problems: list[str] = []
    for family in ("sql", "turso"):
        for path in sources(storage / family):
            rel = path.relative_to(storage).as_posix()
            if rel == TURSO_ONLY_SQL:
                continue
            for number, line in code_lines(path):
                if SQL_LITERAL.search(line) and not any(m in line for m in PER_DRIVER):
                    problems.append(f"inline SQL in storage/{rel}:{number}: {line.strip()}")
    return problems


def hand_written_qmarks(storage: Path) -> list[str]:
    problems: list[str] = []
    for path in sources(storage):
        rel = path.relative_to(storage).as_posix()
        if rel == TURSO_ONLY_SQL or rel.startswith("turso/placeholders"):
            continue
        for number, line in code_lines(path):
            if HAND_WRITTEN_QMARK.search(line):
                problems.append(f"hand-written ?N in storage/{rel}:{number}: {line.strip()}")
    return problems


def statement_problems(statements: Path) -> tuple[list[str], int]:
    if not statements.is_dir():
        return ["storage/statements/ is missing"], 0
    problems: list[str] = []
    count = 0
    for path in sources(statements):
        rel = path.relative_to(statements.parent).as_posix()
        for number, line in code_lines(path):
            count += len(SQL_LITERAL.findall(line))
            if STRAY_DOLLAR.search(line):
                problems.append(f"a '$' that is not a placeholder in storage/{rel}:{number}")
    if count < MIN_STATEMENTS:
        problems.append(
            f"storage/statements/ holds {count} statements, expected at least "
            f"{MIN_STATEMENTS}: the parser or the layout probably broke"
        )
    return problems, count


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="store SQL written once (#106)")
    parser.add_argument("--root", type=Path, default=REPO)
    args = parser.parse_args(argv)
    storage = args.root / "crates" / "wamux" / "src" / "storage"
    shared, count = statement_problems(storage / "statements")
    problems = inline_sql(storage) + hand_written_qmarks(storage) + shared
    if problems:
        print("store SQL check FAILED:")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    print(f"store SQL ok: {count} statements in storage/statements/, none inline in the families")
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main(sys.argv[1:]))
