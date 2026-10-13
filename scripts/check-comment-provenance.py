#!/usr/bin/env python3
"""#85: no source names the review that produced it.

Usage: scripts/check-comment-provenance.py [--root DIR]   (DIR defaults to the repo)

Why: a comment states the reason the code is this way. "code-review 2026-06-11"
or "edge-review-insights.md achado #3" says who looked at it and when, which is
history: it belongs in the commit message, not in the file. Unlike `used to` or
`no longer`, this pattern has no legitimate use, even inside a string, so the
check is per line and has no allowlist.

Scanned: crates/*/src/**/*.rs and crates/*/proto/**/*.proto. Integration tests
(crates/*/tests), build scripts and docs are out of scope. At least
MIN_SCANNED_FILES files must be scanned, so a glob that silently finds nothing
cannot pass.
"""
import argparse
import re
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MIN_SCANNED_FILES = 20
PROVENANCE = re.compile(r"code-review 20\d\d|edge-review")


def source_files(root: Path) -> list[Path]:
    """Every file the rule applies to, sorted."""
    crates = root / "crates"
    found = set(crates.glob("*/src/**/*.rs"))
    found.update(crates.glob("*/proto/**/*.proto"))
    return sorted(found)


def provenance_hits(path: Path) -> list[tuple[int, str]]:
    """(1-based line number, stripped text) of each line naming a review."""
    lines = path.read_text(encoding="utf-8").splitlines()
    return [
        (number, line.strip())
        for number, line in enumerate(lines, start=1)
        if PROVENANCE.search(line)
    ]


def scan(root: Path) -> tuple[int, list[str]]:
    """(files scanned, `path:line: text` for each hit)."""
    root = Path(root)
    files = source_files(root)
    hits = [
        f"{path.relative_to(root)}:{number}: {text}"
        for path in files
        for number, text in provenance_hits(path)
    ]
    return len(files), hits


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="comment provenance check (#85)")
    parser.add_argument("--root", type=Path, default=REPO)
    scanned, hits = scan(parser.parse_args(argv).root)
    problems: list[str] = []
    if scanned < MIN_SCANNED_FILES:
        problems.append(
            f"only {scanned} source files scanned, expected at least "
            f"{MIN_SCANNED_FILES}: the layout or the globs probably broke"
        )
    problems.extend(f"{hit} (names a review, not a reason)" for hit in hits)
    if problems:
        print("comment provenance FAILED:")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    print(f"comment provenance ok: {scanned} files, no review provenance")
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main(sys.argv[1:]))
