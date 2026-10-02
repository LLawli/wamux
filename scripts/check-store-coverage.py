#!/usr/bin/env python3
"""#60: every method the store engines implement is called by a test.

Usage: scripts/check-store-coverage.py [--root DIR]   (DIR defaults to the repo)

Why: 41 of 59 store methods had no test calling them, so an engine could
diverge from the other (or from the trait) with CI green. The storage rewrite in
#65 would cross that gap with no net. This check pins the surface: both
engines declare the same methods, there are at least MIN_STORE_METHODS of
them (a parser that silently finds nothing must not pass), and each name
appears as a `.name(` call somewhere under crates/wamux/tests/.

It is a textual check, not a proof of behavior: the parity tests assert
that. It only stops a method from existing with no caller at all.
"""
import argparse
import re
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ENGINES = ("postgres", "sqlite")
MIN_STORE_METHODS = 59
ASYNC_FN = re.compile(r"\basync\s+fn\s+(\w+)")
CALL = re.compile(r"\.\s*(\w+)\s*\(")


def store_methods(engine_dir: Path) -> set[str]:
    """Names of every `async fn` in the engine's `*_store.rs` files only."""
    names: set[str] = set()
    for path in sorted(engine_dir.glob("*_store.rs")):
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


def engine_problems(by_engine: dict[str, set[str]]) -> list[str]:
    """Divergence between engines, and a too-small surface."""
    problems: list[str] = []
    first, second = ENGINES
    for here, there in ((first, second), (second, first)):
        only = sorted(by_engine[here] - by_engine[there])
        if only:
            problems.append(f"only in {here}, missing in {there}: {', '.join(only)}")
    for engine, names in by_engine.items():
        if len(names) < MIN_STORE_METHODS:
            problems.append(
                f"{engine} declares {len(names)} store methods, expected at least "
                f"{MIN_STORE_METHODS}: the parser or the layout probably broke"
            )
    return problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="store method coverage check (#60)")
    parser.add_argument("--root", type=Path, default=REPO)
    root: Path = parser.parse_args(argv).root
    storage = root / "crates" / "wamux" / "src" / "storage"
    by_engine = {e: store_methods(storage / e) for e in ENGINES}
    problems = engine_problems(by_engine)
    union = set().union(*by_engine.values())
    uncalled = sorted(union - called_methods(root / "crates" / "wamux" / "tests", union))
    if uncalled:
        problems.append(f"store methods never called under crates/wamux/tests/: {', '.join(uncalled)}")
    if problems:
        print("store coverage FAILED:")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    print(
        f"store coverage ok: {len(by_engine[ENGINES[0]])} methods per engine, "
        "each called under crates/wamux/tests/"
    )
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main(sys.argv[1:]))
