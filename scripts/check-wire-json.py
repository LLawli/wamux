#!/usr/bin/env python3
"""#74 / #135 / #151: the daemon puts JSON on the wire only in two named places.

Usage: scripts/check-wire-json.py [--root DIR]   (DIR defaults to the repo)

Why: `CallEvent.raw` was `serde_json::to_vec` of the whole library value, so a
rename in whatsapp-rust silently changed the contract. Every event is a typed
message now. Two JSON serializers are left, each in a named function:
- `fn raw_event_of` in `domain/event_mapping.rs`: the catch-all for a library
  event wamux does not know (the library's JSON, on purpose).
- `fn mex_payload_json_of` in `domain/mex_notification.rs` (#151): the
  server's GraphQL body of a mex notification. The library parses it and the
  core writes it back out; its shape is the server's, not the library's.

Any JSON serializer under `crates/wamux/src` fails (`to_vec`, `to_string`,
`to_value`, `to_writer`, their `_pretty` forms, `json!`), except inside one of
those functions in its own file. Reading JSON (`from_str`, `from_slice`) is not
writing it. Comments are not read, nor are test files (`*_tests.rs`,
`tests.rs`, `test_xml.rs`) or `storage/`, whose JSON columns are rows of the
store, not the wire. The check also fails when a file or an allowed function is
missing, so a move or a rename cannot make it pass by reading nothing.
"""
import argparse
import re
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SRC = "crates/wamux/src"
MAPPING = f"{SRC}/domain/event_mapping.rs"
# Each allowed serializer, by the file it must live in.
ALLOWED_SITES = {
    MAPPING: "raw_event_of",
    f"{SRC}/domain/mex_notification.rs": "mex_payload_json_of",
}
SERIALIZER = re.compile(
    r"\bserde_json::(?:to_vec(?:_pretty)?|to_string(?:_pretty)?|to_value|to_writer(?:_pretty)?)\b"
    r"|\b(?:serde_json::)?json!"
)
FN_START = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?fn\s+(\w+)")
# String and char literals, removed before braces are counted so a '{' inside
# one cannot open a block that never closes.
LITERAL = re.compile(r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])\'')
TEST_FILES = {"tests.rs", "test_xml.rs"}


def is_scanned(path: Path, src: Path) -> bool:
    """Production source only: not tests, not the store."""
    if path.name.endswith("_tests.rs") or path.name in TEST_FILES:
        return False
    return "storage" not in path.relative_to(src).parts


def serializer_lines(text: str, allowed_fn: str | None) -> tuple[list[int], bool]:
    """(line numbers with a serializer outside `allowed_fn`, whether it exists)."""
    offenders: list[int] = []
    found_allowed = False
    depth = 0
    allowed_depth: int | None = None
    for number, line in enumerate(text.splitlines(), 1):
        code = line.split("//", 1)[0]
        start = FN_START.match(code)
        if allowed_fn and start and start.group(1) == allowed_fn:
            found_allowed = True
            allowed_depth = depth
        inside = allowed_depth is not None
        if SERIALIZER.search(code) and not inside:
            offenders.append(number)
        braces = LITERAL.sub("", code)
        depth += braces.count("{") - braces.count("}")
        if inside and depth <= allowed_depth and "}" in braces:
            allowed_depth = None
    return offenders, found_allowed


def scan(root: Path) -> tuple[list[str], set[str]]:
    """Every offending `file:line` under the tree, and the allowed sites found."""
    src = root / SRC
    problems: list[str] = []
    found_sites: set[str] = set()
    for path in sorted(src.rglob("*.rs")):
        if not is_scanned(path, src):
            continue
        relative = path.relative_to(root).as_posix()
        allowed_fn = ALLOWED_SITES.get(relative)
        offenders, found = serializer_lines(path.read_text(encoding="utf-8"), allowed_fn)
        if found:
            found_sites.add(relative)
        problems += [f"{relative}:{n}: a JSON serializer outside the allowed sites" for n in offenders]
    return problems, found_sites


def missing_sites(found_sites: set[str]) -> list[str]:
    """A named site that is not where it should be: moved, renamed or deleted."""
    return [
        f"{relative}: fn {name} not found: the allowed site moved or was renamed"
        for relative, name in ALLOWED_SITES.items()
        if relative not in found_sites
    ]


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="no JSON on the wire outside the named sites (#135)")
    parser.add_argument("--root", type=Path, default=REPO)
    args = parser.parse_args(argv)
    if not (args.root / MAPPING).is_file():
        print(f"wire json check FAILED: {MAPPING} not found: the layout moved or the scan broke")
        return 1
    problems, found_sites = scan(args.root)
    problems += missing_sites(found_sites)
    if problems:
        print("wire json check FAILED (typed messages, not the library's JSON, #135):")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    print(f"no JSON serializer under {SRC} outside " + ", ".join(ALLOWED_SITES.values()))
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main(sys.argv[1:]))
