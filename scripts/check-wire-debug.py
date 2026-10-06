#!/usr/bin/env python3
"""#73 / #126: no Rust `Debug` output reaches the wire from the event mapping.

Usage: scripts/check-wire-debug.py [--root DIR]   (DIR defaults to the repo)

Why: three event fields were `format!("{:?}", ..)` of a library value (the
logout reason, the whole temporary ban, the undecryptable reason), so a rename
in whatsapp-rust silently changed the contract. They are typed enums now.

A `{:?}` / `{name:?}` / `{:#?}` format spec in `domain/event_mapping.rs` fails,
except inside `fn variant_name`: the catch-all `RawEvent.kind` is the one place
allowed to name a library event wamux does not know, and only by its `Debug`.
Comments are not read. The check also fails when the file or `variant_name` is
missing, so a move or a rename cannot make it pass by reading nothing.
"""
import argparse
import re
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MAPPING = "crates/wamux/src/domain/event_mapping.rs"
ALLOWED_FN = "variant_name"
DEBUG_SPEC = re.compile(r"\{[A-Za-z0-9_.]*:#?\?\}")
FN_START = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+(\w+)")
# String and char literals, removed before braces are counted: `variant_name`
# itself splits on '{', which would otherwise open a block that never closes.
LITERAL = re.compile(r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])\'')


def debug_lines(text: str) -> tuple[list[int], bool]:
    """(line numbers with a Debug spec outside ALLOWED_FN, whether ALLOWED_FN exists)."""
    offenders: list[int] = []
    found_allowed = False
    depth = 0
    allowed_depth: int | None = None
    for number, line in enumerate(text.splitlines(), 1):
        code = line.split("//", 1)[0]
        start = FN_START.match(code)
        if start and start.group(1) == ALLOWED_FN:
            found_allowed = True
            allowed_depth = depth
        inside = allowed_depth is not None
        if DEBUG_SPEC.search(code) and not inside:
            offenders.append(number)
        braces = LITERAL.sub("", code)
        depth += braces.count("{") - braces.count("}")
        if inside and depth <= allowed_depth and "}" in braces:
            allowed_depth = None
    return offenders, found_allowed


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="no Debug output on the wire (#126)")
    parser.add_argument("--root", type=Path, default=REPO)
    args = parser.parse_args(argv)
    path = args.root / MAPPING
    if not path.is_file():
        print(f"wire debug check FAILED: {MAPPING} not found: the layout moved or the scan broke")
        return 1
    offenders, found_allowed = debug_lines(path.read_text(encoding="utf-8"))
    problems = [f"{MAPPING}:{n}: a Debug format spec outside {ALLOWED_FN}" for n in offenders]
    if not found_allowed:
        problems.append(f"{MAPPING}: fn {ALLOWED_FN} not found: the allowed site moved or was renamed")
    if problems:
        print("wire debug check FAILED (Debug output is not a contract, #126):")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    print(f"no Debug format spec in {MAPPING} outside {ALLOWED_FN}")
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main(sys.argv[1:]))
