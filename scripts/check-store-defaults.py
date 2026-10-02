#!/usr/bin/env python3
"""#93: every wacore store trait default is classified in docs/store-trait-defaults.md.

Usage: scripts/check-store-defaults.py [--root DIR] [--traits PATH]
       (DIR defaults to the repo; PATH defaults to wacore's traits.rs, resolved
       through `cargo metadata --offline`)

Why: wacore gives some store trait methods a default body, and a backend that
says nothing silently inherits it. Four of those methods were wrong for a
real backend (`get_sent_message` errored, `delete_expired_base_keys` never
pruned, the two tc-token writers were not atomic) and nothing flagged them. This
check makes the choice explicit: every default the pinned wacore declares needs
a row in the doc (override or default, with a reason), every row must name a
default that still exists, and `override` means both engines implement the
method while `default` means neither does. A whatsapp-rust bump that adds a
default fails here until someone classifies it.

Textual check, like check-store-coverage.py: it parses traits.rs and the engine
sources, it does not prove an override behaves; the parity tests do that.
"""
import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ENGINES = ("postgres", "sqlite")
MIN_TRAIT_DEFAULTS = 36
DOC = Path("docs") / "store-trait-defaults.md"
ASYNC_FN = re.compile(r"\basync\s+fn\s+(\w+)")
PUB_TRAIT = re.compile(r"\bpub\s+trait\s+(\w+)")
DOC_ROW = re.compile(r"^\|\s*`(\w+)`\s*\|([^|]*)\|([^|]*)\|(.*)\|\s*$")
STRING_LITERAL = re.compile(r'"(?:\\.|[^"\\])*"')


class CheckError(Exception):
    """A reportable failure; main prints it and exits 1, never a traceback."""


def strip_noise(text: str) -> str:
    """Drop line comments and string literals so their braces and `;` do not count."""
    lines = [STRING_LITERAL.sub('""', line.split("//", 1)[0]) for line in text.splitlines()]
    return "\n".join(lines)


def skip_balanced(text: str, start: int, opener: str, closer: str) -> int:
    """Index just past the closer matching the opener at `start`."""
    depth = 0
    for i in range(start, len(text)):
        depth += (text[i] == opener) - (text[i] == closer)
        if depth == 0:
            return i + 1
    return len(text)


def header_end(text: str, start: int) -> tuple[int, str]:
    """Index and char (`{` default, `;` required) ending a fn header."""
    nesting = 0
    for i in range(start, len(text)):
        ch = text[i]
        nesting += (ch in "([") - (ch in ")]")
        if nesting == 0 and ch in "{;":
            return i, ch
    return len(text), ";"


def trait_defaults(body: str) -> list[str]:
    """Names of the `async fn`s at the trait's top level that carry a body."""
    names: list[str] = []
    pos = 0
    while True:
        found = ASYNC_FN.search(body, pos)
        if not found:
            return names
        end, ch = header_end(body, found.end())
        if ch == "{":
            names.append(found.group(1))
            end = skip_balanced(body, end, "{", "}")
        pos = end + 1 if ch == ";" else end


def declared_defaults(traits_text: str) -> dict[str, str]:
    """method -> trait, for every default in every `pub trait` of the file."""
    text = strip_noise(traits_text)
    out: dict[str, str] = {}
    for trait in PUB_TRAIT.finditer(text):
        open_at = text.find("{", trait.end())
        if open_at < 0:
            continue
        close = skip_balanced(text, open_at, "{", "}")
        for name in trait_defaults(text[open_at + 1 : close - 1]):
            out[name] = trait.group(1)
    return out


def doc_rows(doc_text: str) -> dict[str, tuple[str, str, str]]:
    """method -> (trait, decision, reason) from the markdown table."""
    rows: dict[str, tuple[str, str, str]] = {}
    for line in doc_text.splitlines():
        match = DOC_ROW.match(line.strip())
        if match:
            name, trait, decision, why = match.groups()
            rows[name] = (trait.strip(), decision.strip(), why.strip())
    return rows


def engine_methods(storage: Path) -> dict[str, set[str]]:
    """`async fn` names in each engine's `*_store.rs` (same rule as the coverage check)."""
    out: dict[str, set[str]] = {}
    for engine in ENGINES:
        names: set[str] = set()
        for path in sorted((storage / engine).glob("*_store.rs")):
            names.update(ASYNC_FN.findall(read_text(path)))
        out[engine] = names
    return out


def read_text(path: Path) -> str:
    try:
        return path.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as err:
        raise CheckError(f"cannot read {path}: {err}") from err


def resolve_traits_path(root: Path) -> Path:
    """wacore's traits.rs, found through the manifest of the pinned `wacore` package."""
    cmd = ["cargo", "metadata", "--format-version", "1", "--locked", "--offline"]
    try:
        done = subprocess.run(cmd, cwd=root, capture_output=True, text=True, check=False)
        if done.returncode != 0:
            raise CheckError(f"cannot resolve wacore: cargo metadata exited {done.returncode}")
        packages = json.loads(done.stdout)["packages"]
    except (OSError, ValueError, KeyError) as err:
        raise CheckError(f"cannot resolve wacore: {err}") from err
    for package in packages:
        if package.get("name") == "wacore":
            return Path(package["manifest_path"]).parent / "src" / "store" / "traits.rs"
    raise CheckError("cannot resolve wacore: no `wacore` package in cargo metadata")


def load_traits(traits: Path) -> str:
    try:
        return traits.read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError) as err:
        raise CheckError(f"cannot resolve wacore: {traits} unreadable: {err}") from err


def row_problems(name: str, trait: str, row: tuple[str, str, str], impls: dict[str, set[str]]) -> list[str]:
    """Everything wrong with one classified default."""
    row_trait, decision, why = row
    problems: list[str] = []
    if row_trait != trait:
        problems.append(f"trait column says {row_trait}, wacore declares {name} in {trait}")
    if decision not in ("override", "default"):
        problems.append(f"unknown decision '{decision}' for {name}, expected override or default")
        return problems
    if decision == "override":
        problems += [f"marked override but not implemented by {e}: {name}" for e in ENGINES if name not in impls[e]]
        return problems
    problems += [f"marked default but overridden by {e}: {name}" for e in ENGINES if name in impls[e]]
    if not why:
        problems.append(f"default kept without a reason: {name}")
    return problems


def classification_problems(
    defaults: dict[str, str], rows: dict[str, tuple[str, str, str]], impls: dict[str, set[str]]
) -> list[str]:
    """Floor, then unclassified, stale, and per-row disagreements."""
    problems: list[str] = []
    if len(defaults) < MIN_TRAIT_DEFAULTS:
        problems.append(
            f"found {len(defaults)} trait defaults, expected at least {MIN_TRAIT_DEFAULTS}: "
            "the parser or the pinned wacore probably changed"
        )
    problems += [f"unclassified trait default: {n}" for n in sorted(set(defaults) - set(rows))]
    problems += [f"stale row, not a trait default: {n}" for n in sorted(set(rows) - set(defaults))]
    for name in sorted(set(defaults) & set(rows)):
        problems += row_problems(name, defaults[name], rows[name], impls)
    return problems


def run(root: Path, traits: Path | None) -> int:
    traits_path = traits if traits is not None else resolve_traits_path(root)
    defaults = declared_defaults(load_traits(traits_path))
    rows = doc_rows(read_text(root / DOC))
    impls = engine_methods(root / "crates" / "wamux" / "src" / "storage")
    problems = classification_problems(defaults, rows, impls)
    if problems:
        print("store defaults FAILED:")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    over = sum(1 for r in rows.values() if r[1] == "override")
    print(f"store defaults ok: {len(defaults)} trait defaults, {over} overridden, {len(defaults) - over} kept")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="store trait defaults check (#93)")
    parser.add_argument("--root", type=Path, default=REPO)
    parser.add_argument("--traits", type=Path, default=None)
    args = parser.parse_args(argv)
    try:
        return run(args.root, args.traits)
    except CheckError as err:
        print(f"store defaults FAILED:\n  - {err}")
        return 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
