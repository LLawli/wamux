#!/usr/bin/env python3
"""#67: no test synchronizes on a fixed sleep.

Usage: scripts/check-test-sleeps.py [--root DIR]   (DIR defaults to the repo)

Why: fixed sleeps as synchronization (a 150 ms wait for "attached", a 300 ms
wait before asserting "nothing was sent") pass on a fast machine and flake on
a loaded one, or worse, pass without proving anything. The fix is a bounded
wait on the condition (`common::poll_until`) or a paused clock. A sleep that
remains must say why it is not a sync point: every `sleep(` needs a comment
`not a sync point: <reason>` on the line immediately above it.

Scanned: crates/*/tests/**/*.rs and, under crates/*/src, only the files named
`tests.rs` or `*_tests.rs`. Limit: tests written inline (`#[cfg(test)] mod`)
inside a production source file are NOT scanned; move them to a `tests.rs`
to get them checked. At least MIN_SCANNED_FILES files must be scanned, so a
glob that silently finds nothing cannot pass.
"""
import argparse
import re
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MARK = "not a sync point:"
MIN_SCANNED_FILES = 20
SLEEP_CALL = re.compile(r"\bsleep\s*\(")


def test_files(root: Path) -> list[Path]:
    """Every file the rule applies to, sorted."""
    crates = root / "crates"
    found = set(crates.glob("*/tests/**/*.rs"))
    found.update(crates.glob("*/src/**/tests.rs"))
    found.update(crates.glob("*/src/**/*_tests.rs"))
    return sorted(found)


def is_marked(previous_line: str) -> bool:
    """True when the line above carries the mark followed by a real reason."""
    text = previous_line.strip()
    if not text.startswith("//") or MARK not in text:
        return False
    return bool(text.split(MARK, 1)[1].strip())


def unmarked_sleeps(path: Path) -> list[int]:
    """1-based line numbers of sleeps with no mark on the line above."""
    lines = path.read_text(encoding="utf-8").splitlines()
    offenders: list[int] = []
    for index, line in enumerate(lines):
        if line.lstrip().startswith("//") or not SLEEP_CALL.search(line):
            continue
        if index == 0 or not is_marked(lines[index - 1]):
            offenders.append(index + 1)
    return offenders


def scan(root: Path) -> tuple[int, list[str]]:
    """(files scanned, `path:line` for each unmarked sleep)."""
    root = Path(root)
    files = test_files(root)
    offenders = [
        f"{path.relative_to(root)}:{line}"
        for path in files
        for line in unmarked_sleeps(path)
    ]
    return len(files), offenders


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="test sleep check (#67)")
    parser.add_argument("--root", type=Path, default=REPO)
    scanned, offenders = scan(parser.parse_args(argv).root)
    problems: list[str] = []
    if scanned < MIN_SCANNED_FILES:
        problems.append(
            f"only {scanned} test files scanned, expected at least "
            f"{MIN_SCANNED_FILES}: the layout or the globs probably broke"
        )
    problems.extend(f"{where}: sleep without `{MARK} <reason>` above it" for where in offenders)
    if problems:
        print("test sleeps FAILED:")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    print(f"test sleeps ok: {scanned} files scanned, every sleep is marked")
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main(sys.argv[1:]))
