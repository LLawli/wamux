#!/usr/bin/env python3
"""#63 / #114: a `tonic::Status` is constructed in one place only.

Usage: scripts/check-status-sites.py [--root DIR]   (DIR defaults to the repo)

Why: the same failure was built two ways. `services/mod.rs` wrote
`Status::failed_precondition("account is not connected")` by hand while
`WamuxError::NotConnected` existed, so the two could drift in code or wording,
and an edge matches on both. Every error now goes through `WamuxError`, and
its `From<WamuxError> for tonic::Status` in `crates/wamux-types/src/error.rs`
is the only place a `Status` is built. This check fails on any constructor
call (`Status::invalid_argument(..)`, `Status::new(..)`, ...) anywhere else in
the daemon or in wamux-types.

Not a construction, so not flagged: `Status::from(err)` (the conversion) and a
`Status` that tonic itself hands back (`stream.message().await?`). Test files
(`*_tests.rs`, `tests.rs`, anything under a `tests/` directory) are skipped:
tests build statuses to assert against. The allowed file must hold at least
MIN_ALLOWED constructions, so a scan that reads nothing cannot pass.
"""
import argparse
import re
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ALLOWED = "crates/wamux-types/src/error.rs"
SCANNED = ("crates/wamux/src", "crates/wamux-types/src")
MIN_ALLOWED = 7
CONSTRUCTORS = (
    "new|ok|cancelled|unknown|invalid_argument|deadline_exceeded|not_found|"
    "already_exists|permission_denied|resource_exhausted|failed_precondition|"
    "aborted|out_of_range|unimplemented|internal|unavailable|data_loss|"
    "unauthenticated|with_details|with_metadata|with_details_and_metadata"
)
CONSTRUCTION = re.compile(rf"\bStatus::(?:{CONSTRUCTORS})\s*\(")


def is_test_file(path: Path) -> bool:
    return path.name == "tests.rs" or path.name.endswith("_tests.rs") or "tests" in path.parts


def constructions(path: Path) -> list[tuple[int, str]]:
    """(line, text) of each constructor call outside a `//` comment."""
    hits = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        code = line.split("//", 1)[0]
        if CONSTRUCTION.search(code):
            hits.append((number, line.strip()))
    return hits


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="Status constructed in one place (#114)")
    parser.add_argument("--root", type=Path, default=REPO)
    args = parser.parse_args(argv)
    problems: list[str] = []
    allowed_count = 0
    for directory in SCANNED:
        for path in sorted((args.root / directory).rglob("*.rs")):
            rel = path.relative_to(args.root).as_posix()
            if is_test_file(path.relative_to(args.root)):
                continue
            hits = constructions(path)
            if rel == ALLOWED:
                allowed_count = len(hits)
                continue
            problems += [f"{rel}:{n}: {text}" for n, text in hits]
    if allowed_count < MIN_ALLOWED:
        problems.append(
            f"{ALLOWED} holds {allowed_count} Status constructions, expected at least "
            f"{MIN_ALLOWED}: the mapping moved or the scan broke"
        )
    if problems:
        print("Status construction check FAILED (build a WamuxError instead):")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    print(f"Status constructed only in {ALLOWED} ({allowed_count} sites)")
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main(sys.argv[1:]))
