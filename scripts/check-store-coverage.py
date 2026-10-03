#!/usr/bin/env python3
"""#60: every method the store engines implement is called by a test.

Usage: scripts/check-store-coverage.py [--root DIR] [--families A,B]
       (DIR defaults to the repo, the families to FAMILIES)

Why: 41 of 59 store methods had no test calling them, so an engine could
diverge from the other (or from the trait) with CI green. The storage rewrite in
#65 would cross that gap with no net. This check pins the surface: both
engines declare the same methods, there are at least MIN_STORE_METHODS of
them (a parser that silently finds nothing must not pass), and each name
appears as a `.name(` call somewhere under crates/wamux/tests/.

Since #65 the unit is the engine FAMILY, one directory under storage/ with
one impl of each trait (`sql` covers Postgres and SQLite). Every family must
declare the same methods, and every `*_store.rs` under storage/ must belong to
a listed family, so a new engine (#106) or a stray copy cannot sit outside the
check.

It is a textual check, not a proof of behavior: the parity tests assert
that. It only stops a method from existing with no caller at all.
"""
import argparse
import re
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
FAMILIES = ("sql",)
MIN_STORE_METHODS = 63
ASYNC_FN = re.compile(r"\basync\s+fn\s+(\w+)")
CALL = re.compile(r"\.\s*(\w+)\s*\(")


def store_methods(family_dir: Path) -> set[str]:
    """Names of every `async fn` in the family's `*_store.rs` files only."""
    names: set[str] = set()
    for path in sorted(family_dir.glob("*_store.rs")):
        names.update(ASYNC_FN.findall(path.read_text(encoding="utf-8")))
    return names


def called_methods(tests_dir: Path, names: set[str]) -> set[str]:
    """Which of `names` appear as `.name(` in any .rs file under tests_dir."""
    found: set[str] = set()
    for path in sorted(tests_dir.rglob("*.rs")):
        for line in path.read_text(encoding="utf-8").splitlines():
            # Cutting at `//` also cuts inside `postgres://` strings; harmless,
            # no store call follows a URL on the same line.
            code = line.split("//", 1)[0]
            found.update(n for n in CALL.findall(code) if n in names)
    return found


def family_problems(by_family: dict[str, set[str]]) -> list[str]:
    """Divergence between families (each against the first), and a too-small surface."""
    problems: list[str] = []
    first, *rest = by_family
    for family in rest:
        for here, there in ((family, first), (first, family)):
            only = sorted(by_family[here] - by_family[there])
            if only:
                problems.append(f"only in {here}, missing in {there}: {', '.join(only)}")
    for family, names in by_family.items():
        if len(names) < MIN_STORE_METHODS:
            problems.append(
                f"{family} declares {len(names)} store methods, expected at least "
                f"{MIN_STORE_METHODS}: the parser or the layout probably broke"
            )
    return problems


def layout_problems(storage: Path, families: tuple[str, ...]) -> list[str]:
    """A listed family with no directory, or a store file outside every family."""
    problems = [f"family directory missing: storage/{f}" for f in families if not (storage / f).is_dir()]
    for path in sorted(storage.rglob("*_store.rs")):
        if path.parent.relative_to(storage).as_posix() not in families:
            problems.append(f"store file outside the families {', '.join(families)}: {path.relative_to(storage)}")
    return problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="store method coverage check (#60)")
    parser.add_argument("--root", type=Path, default=REPO)
    parser.add_argument("--families", default=",".join(FAMILIES))
    args = parser.parse_args(argv)
    families = tuple(f for f in args.families.split(",") if f)
    storage = args.root / "crates" / "wamux" / "src" / "storage"
    by_family = {f: store_methods(storage / f) for f in families}
    problems = layout_problems(storage, families) + family_problems(by_family)
    union = set().union(*by_family.values())
    uncalled = sorted(union - called_methods(args.root / "crates" / "wamux" / "tests", union))
    if uncalled:
        problems.append(f"store methods never called under crates/wamux/tests/: {', '.join(uncalled)}")
    if problems:
        print("store coverage FAILED:")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    print(
        f"store coverage ok: {len(by_family[families[0]])} methods per family "
        f"({', '.join(families)}), each called under crates/wamux/tests/"
    )
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main(sys.argv[1:]))
