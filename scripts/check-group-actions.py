#!/usr/bin/env python3
"""Every variant of wacore's GroupNotificationAction has its own arm in action_of.

Usage: scripts/check-group-actions.py [--root DIR] [--groups PATH]
       (DIR defaults to the repo; PATH defaults to wacore's stanza/groups.rs,
       resolved the way check-store-defaults.py resolves traits.rs)

Why: `domain/group_update.rs::action_of` maps each library group action to its
own wire case. The library marks the enum `#[non_exhaustive]`, so the match
needs a fallback arm, and a variant added upstream compiles straight into it:
it reaches the edge as `GroupUnknownAction` carrying its tag, typed fields
dropped, and no test notices. The compiler used to catch that at the bump;
this check is what catches it now. A whatsapp-rust bump that adds a variant
fails here until action_of maps it.

Textual check: it reads the enum's top-level variant names and the `Lib::Name`
patterns in action_of. It does not prove a mapping is right; the
group_update tests do that.
"""
import argparse
import importlib.util
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MAPPER = Path("crates") / "wamux" / "src" / "domain" / "group_update.rs"
ENUM_HEAD = re.compile(r"\bpub\s+enum\s+GroupNotificationAction\s*\{")
MAPPER_HEAD = re.compile(r"\bfn\s+action_of\s*\([^)]*\)[^{]*\{")
TOKEN = re.compile(r"[{}]|\b([A-Z]\w*)\b")
LIB_ARM = re.compile(r"\bLib::(\w+)")
# 44 variants on the pin that added this check; far fewer means the parser lost
# the enum, not that upstream deleted half of it.
MIN_VARIANTS = 40


class CheckError(Exception):
    """A reportable failure; main prints it and exits 1, never a traceback."""


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as err:
        raise CheckError(f"cannot read {path}: {err}") from err


def braced_body(text: str, head: re.Pattern, what: str) -> str:
    """The text between the `{` that ends `head` and its matching `}`."""
    found = head.search(text)
    if not found:
        raise CheckError(f"{what} not found")
    depth, start = 1, found.end()
    for i in range(start, len(text)):
        depth += (text[i] == "{") - (text[i] == "}")
        if depth == 0:
            return text[start:i]
    raise CheckError(f"{what} has no closing brace")


def without_comments(text: str) -> str:
    """Line comments hold names (`<add>`, `Add`) and braces that are not code."""
    return "\n".join(line.split("//", 1)[0] for line in text.splitlines())


def enum_variants(groups_text: str) -> list[str]:
    """Capitalized names at the enum's top level: the variants. Field types sit
    one brace deeper; attributes (`#[wire = "add"]`) hold no capitalized word."""
    body = without_comments(braced_body(groups_text, ENUM_HEAD, "pub enum GroupNotificationAction"))
    depth, names = 0, []
    for token in TOKEN.finditer(body):
        depth += (token.group(0) == "{") - (token.group(0) == "}")
        if token.group(1) and depth == 0:
            names.append(token.group(1))
    return names


def mapped_variants(mapper_text: str) -> set[str]:
    body = without_comments(braced_body(mapper_text, MAPPER_HEAD, "fn action_of in group_update.rs"))
    return set(LIB_ARM.findall(body))


def resolve_groups_path(root: Path) -> Path:
    """wacore's stanza/groups.rs, next to the traits.rs the defaults check finds."""
    spec = importlib.util.spec_from_file_location("store_defaults", REPO / "scripts" / "check-store-defaults.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    try:
        traits = module.resolve_traits_path(root)
    except module.CheckError as err:
        raise CheckError(str(err)) from err
    return traits.parent.parent / "stanza" / "groups.rs"


def mapping_problems(variants: list[str], mapped: set[str]) -> list[str]:
    problems: list[str] = []
    if len(variants) < MIN_VARIANTS:
        problems.append(
            f"found {len(variants)} GroupNotificationAction variants, expected at least {MIN_VARIANTS}: "
            "the parser or the pinned wacore probably changed"
        )
    problems += [
        f"unmapped variant: {v} (it would reach the edge as GroupUnknownAction)"
        for v in variants
        if v not in mapped
    ]
    problems += [f"stale arm, not a variant: Lib::{m}" for m in sorted(mapped - set(variants))]
    return problems


def run(root: Path, groups: Path | None) -> int:
    groups_path = groups if groups is not None else resolve_groups_path(root)
    variants = enum_variants(read_text(groups_path))
    problems = mapping_problems(variants, mapped_variants(read_text(root / MAPPER)))
    if problems:
        print("group actions FAILED:")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    print(f"group actions ok: {len(variants)} variants, each mapped in action_of")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="group action mapping check")
    parser.add_argument("--root", type=Path, default=REPO)
    parser.add_argument("--groups", type=Path, default=None)
    args = parser.parse_args(argv)
    try:
        return run(args.root, args.groups)
    except CheckError as err:
        print(f"group actions FAILED:\n  - {err}")
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
