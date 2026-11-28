#!/usr/bin/env python3
"""#280: no Rust item is named `handle`, `process`, `data`, `util` or `manager`.

Usage: scripts/check-banned-names.py [--root DIR]   (DIR defaults to the repo)

Why: CONTRIBUTING.md bans these names because names are the agent's navigation API.
A name that says nothing returns hundreds of grep hits and explains nothing at
the call site: `handle()` could build, look up or run anything. The rule had no
check and rotted (a test helper was named `handle`).

Checked: the names a reader navigates by, the ones declared with `fn` (free
functions, methods, tests, helpers), `mod` and `macro_rules!`. A file name is a
module name, so `util.rs` is caught by its `mod util;`. Exactly the banned
word only: `handle_receipt` or `AccountHandle` are specific and pass.

Not checked, on purpose: local bindings, parameters and fields. Their scope is
one function or one type, the type beside them names what they hold, and they
are not what anyone greps for. `handle: &AccountHandle` is the codebase's
idiom for an account, used at hundreds of sites; renaming it would buy nothing
for navigation.

Comments and simple string literals are not read. Directories named `target`
and dot-directories are skipped. At least MIN_SCANNED_FILES files must be
scanned, so a walk that silently finds nothing cannot pass.

EXCEPTIONS names a declaration allowed to keep a banned name, as
`path:name` -> reason. An exception that no longer matches a declaration
fails too, so the table only shrinks.
"""
import argparse
import os
import re
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BANNED = ("handle", "process", "data", "util", "manager")
MIN_SCANNED_FILES = 100
SKIPPED_DIRS = {"target"}
# `path:name` (path relative to the root) -> why this declaration keeps the name.
EXCEPTIONS: dict[str, str] = {}
_WORDS = "|".join(BANNED)
DECLARATION = re.compile(rf"(?:\bfn|\bmod|\bmacro_rules!)\s+(?:r#)?({_WORDS})\b")
# String and char literals, removed so text inside one is never read as code.
LITERAL = re.compile(r'"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])\'')
BLOCK_COMMENT = re.compile(r"/\*.*?\*/", re.DOTALL)


def rust_files(root: Path) -> list[Path]:
    """Every `*.rs` under root, outside build output and dot-directories, sorted."""
    found: list[Path] = []
    for directory, subdirs, files in os.walk(root):
        subdirs[:] = [d for d in subdirs if d not in SKIPPED_DIRS and not d.startswith(".")]
        found.extend(Path(directory) / name for name in files if name.endswith(".rs"))
    return sorted(found)


def code_lines(text: str) -> list[str]:
    """The lines of a file with block comments blanked (line count kept),
    literals removed and `//` comments cut."""
    text = BLOCK_COMMENT.sub(lambda m: "\n" * m.group(0).count("\n"), text)
    return [LITERAL.sub('""', line).split("//", 1)[0] for line in text.splitlines()]


def banned_declarations(text: str) -> list[tuple[int, str]]:
    """(1-based line, name) of each declaration that uses a banned name."""
    return [
        (number, match.group(1))
        for number, line in enumerate(code_lines(text), start=1)
        for match in DECLARATION.finditer(line)
    ]


def name_problems(hits: list[tuple[str, int, str]], exceptions: dict[str, str]) -> list[str]:
    """One message per hit without an exception and per stale exception."""
    problems = [
        f"{rel}:{line}: `{name}` is a banned name: name what it does"
        for rel, line, name in hits
        if f"{rel}:{name}" not in exceptions
    ]
    matched = {f"{rel}:{name}" for rel, _, name in hits}
    problems.extend(
        f"{key}: in EXCEPTIONS but no such declaration: remove the entry"
        for key in sorted(exceptions)
        if key not in matched
    )
    return problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="banned Rust names check (#280)")
    parser.add_argument("--root", type=Path, default=REPO)
    root = parser.parse_args(argv).root
    files = rust_files(root)
    hits = [
        (path.relative_to(root).as_posix(), line, name)
        for path in files
        for line, name in banned_declarations(path.read_text(encoding="utf-8"))
    ]
    problems = name_problems(hits, EXCEPTIONS)
    if len(files) < MIN_SCANNED_FILES:
        problems.insert(
            0,
            f"only {len(files)} Rust files scanned, expected at least "
            f"{MIN_SCANNED_FILES}: the layout or the walk probably broke",
        )
    if problems:
        print("banned names FAILED:")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    print(
        f"banned names ok: {len(files)} Rust files, no fn, mod or macro named "
        f"{', '.join(BANNED)} ({len(EXCEPTIONS)} excepted)"
    )
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main(sys.argv[1:]))
