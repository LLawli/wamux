#!/usr/bin/env python3
"""#280: every Rust file is under 500 lines.

Usage: scripts/check-file-size.py [--root DIR]   (DIR defaults to the repo)

Why: CONTRIBUTING.md asks for files under 500 lines, one cohesive unit per file, so
an agent can read a whole file at once and predict where a thing lives. The
rule had no check and rotted (event_mapping.rs reached 754 lines). A file that
grows past the limit is split into submodules (`foo.rs` plus `foo/bar.rs`, or a
`#[path = "foo_bar.rs"]` sibling, as the tree already does).

Scanned: every `*.rs` under the root, test files included (`*_tests.rs`,
`tests/`): a 1,100-line test file costs a reader the same as a 1,100-line
source file, and test files are the cheapest to split, since their cases are
independent. Directories named `target` and dot-directories are skipped (build
output, the VCS). At least MIN_SCANNED_FILES files must be scanned, so a walk
that silently finds nothing cannot pass.

EXCEPTIONS names a file allowed past the limit, with the reason a split would
hurt it. An exception that is no longer needed (the file shrank under the
limit, or is gone) fails too, so the table only shrinks.
"""
import argparse
import os
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
LIMIT = 500
MIN_SCANNED_FILES = 100
SKIPPED_DIRS = {"target"}
# path (relative to the root) -> why splitting it would hurt more than its size.
EXCEPTIONS: dict[str, str] = {}


def rust_files(root: Path) -> list[Path]:
    """Every `*.rs` under root, outside build output and dot-directories, sorted."""
    found: list[Path] = []
    for directory, subdirs, files in os.walk(root):
        subdirs[:] = [d for d in subdirs if d not in SKIPPED_DIRS and not d.startswith(".")]
        found.extend(Path(directory) / name for name in files if name.endswith(".rs"))
    return sorted(found)


def line_count(path: Path) -> int:
    return len(path.read_text(encoding="utf-8").splitlines())


def size_problems(counts: dict[str, int], exceptions: dict[str, str]) -> list[str]:
    """One message per oversized file without an exception and per stale exception."""
    problems = [
        f"{rel}: {lines} lines, must be under {LIMIT}: split it into submodules"
        for rel, lines in sorted(counts.items())
        if lines >= LIMIT and rel not in exceptions
    ]
    for rel in sorted(exceptions):
        if rel not in counts:
            problems.append(f"{rel}: in EXCEPTIONS but not found: remove the entry")
        elif counts[rel] < LIMIT:
            problems.append(
                f"{rel}: {counts[rel]} lines, under {LIMIT}: remove it from EXCEPTIONS"
            )
    return problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Rust file size check (#280)")
    parser.add_argument("--root", type=Path, default=REPO)
    root = parser.parse_args(argv).root
    files = rust_files(root)
    counts = {path.relative_to(root).as_posix(): line_count(path) for path in files}
    problems = size_problems(counts, EXCEPTIONS)
    if len(files) < MIN_SCANNED_FILES:
        problems.insert(
            0,
            f"only {len(files)} Rust files scanned, expected at least "
            f"{MIN_SCANNED_FILES}: the layout or the walk probably broke",
        )
    if problems:
        print("file size FAILED:")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    print(
        f"file size ok: {len(files)} Rust files, all under {LIMIT} lines "
        f"({len(EXCEPTIONS)} excepted)"
    )
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main(sys.argv[1:]))
