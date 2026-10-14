#!/usr/bin/env python3
"""#81: every relative link, anchor and docs/*.md mention points at something
that exists.

Usage: scripts/check-doc-links.py [--root DIR]   (DIR defaults to the repo)

Why: the docs described a daemon that no longer existed, and nothing failed
when a file was deleted or a heading renamed. Three kinds of pointer are checked:
- a relative Markdown link (inline, image or `[id]: dest` definition): the file
  or directory must exist, relative to the linking file;
- its `#anchor`, against the GitHub slugs of the target's headings (a bare
  `#anchor` is checked against the same file);
- a root-relative mention of a `docs/` Markdown file in Rust, proto, scripts and
  Markdown text. CHANGELOG.md and docs/BREAKING-CHANGES-*.md are history: they
  name files as they were, so only their links are checked.

Links with a scheme (https:, mailto:) are not ours to check. Hidden directories
and `target` are pruned and never followed: `target` is a symlink to another
disk in a real checkout. MIN_MARKDOWN_FILES and MIN_LINKS keep a walk that
silently finds nothing from passing.
"""
import argparse
import os
import re
from pathlib import Path
from urllib.parse import unquote

REPO = Path(__file__).resolve().parent.parent
MIN_MARKDOWN_FILES = 10
MIN_LINKS = 5

INLINE_LINK = re.compile(r"!?\[[^\]]*\]\(\s*(<[^>]*>|[^)\s]*)(?:\s+(?:\"[^\"]*\"|'[^']*'))?\s*\)")
REFERENCE_DEF = re.compile(r"^ {0,3}\[[^\]]+\]:\s*(<[^>]*>|\S+)")
SCHEME = re.compile(r"^[A-Za-z][A-Za-z0-9+.-]*:")
FENCE = re.compile(r"^ {0,3}(```|~~~)")
HEADING = re.compile(r"^ {0,3}#{1,6}\s+(.*?)\s*#*\s*$")
CODE_SPAN = re.compile(r"(`+)(?:(?!\1).)+?\1")
MENTION = re.compile(r"(?<![\w./-])docs/[\w./-]*\.md(?![\w-])")
HISTORY_FILES = ("CHANGELOG.md",)
HISTORY_PREFIX = "docs/BREAKING-CHANGES-"
MENTION_SUFFIXES = (".rs", ".proto", ".sh", ".py", ".md")


def walk_files(root: Path) -> list[Path]:
    """Every file under root, hidden directories and `target` pruned, sorted."""
    found: list[Path] = []
    for here, dirs, names in os.walk(root, followlinks=False):
        dirs[:] = [d for d in dirs if not d.startswith(".") and d != "target"]
        found.extend(Path(here) / name for name in names)
    return sorted(found)


def markdown_files(files: list[Path]) -> list[Path]:
    return [f for f in files if f.suffix == ".md"]


def mention_files(root: Path, files: list[Path]) -> list[Path]:
    """Files whose text may name a docs/ page; scripts/tests are fixtures."""
    wanted: list[Path] = []
    for path in files:
        rel = path.relative_to(root).parts
        if path.suffix not in MENTION_SUFFIXES or path.is_symlink():
            continue
        if rel[0] == "crates" and path.suffix in (".rs", ".proto", ".md"):
            wanted.append(path)
        elif path.suffix == ".md" and not (rel[0] == "scripts"):
            wanted.append(path)
        elif rel[0] == "scripts" and len(rel) == 2:
            wanted.append(path)
    return wanted


def is_history(root: Path, path: Path) -> bool:
    rel = path.relative_to(root).as_posix()
    return rel in HISTORY_FILES or rel.startswith(HISTORY_PREFIX)


def prose_lines(text: str, blank_code: bool = True) -> list[tuple[int, str]]:
    """(1-based line, text) outside fenced code; code spans blanked if asked.

    Headings keep their spans: GitHub slugs `foo` as foo, not as nothing."""
    kept: list[tuple[int, str]] = []
    fence = ""
    for number, line in enumerate(text.splitlines(), start=1):
        marker = FENCE.match(line)
        if fence:
            fence = "" if marker and marker.group(1) == fence else fence
        elif marker:
            fence = marker.group(1)
        else:
            blanked = CODE_SPAN.sub(lambda m: " " * len(m.group(0)), line)
            kept.append((number, blanked if blank_code else line))
    return kept


def destinations(line: str) -> list[tuple[str, int, int]]:
    """(destination, start, end) of each link destination on a prose line."""
    found: list[tuple[str, int, int]] = []
    for match in INLINE_LINK.finditer(line):
        found.append((match.group(1), match.start(1), match.end(1)))
    ref = REFERENCE_DEF.match(line)
    if ref:
        found.append((ref.group(1), ref.start(1), ref.end(1)))
    return [(d.strip("<>"), s, e) for d, s, e in found]


def github_slug(heading: str) -> str:
    """GitHub's anchor for a heading, before the -N duplicate suffix."""
    text = re.sub(r"!?\[([^\]]*)\]\([^)]*\)", r"\1", heading).lower()
    kept = [c for c in text if c.isalnum() or c in " -_"]
    return "".join(kept).replace(" ", "-")


def heading_slugs(text: str) -> set[str]:
    seen: dict[str, int] = {}
    slugs: set[str] = set()
    for _, line in prose_lines(text, blank_code=False):
        match = HEADING.match(line)
        if not match:
            continue
        base = github_slug(match.group(1))
        count = seen.get(base, 0)
        seen[base] = count + 1
        slugs.add(base if count == 0 else f"{base}-{count}")
    return slugs


class Checker:
    def __init__(self, root: Path) -> None:
        self.root = root
        self.problems: list[str] = []
        self.links = 0
        self.mentions = 0
        self.slug_cache: dict[Path, set[str]] = {}

    def problem(self, path: Path, line: int, message: str) -> None:
        rel = path.relative_to(self.root).as_posix()
        self.problems.append(f"  - {rel}:{line}: {message}")

    def slugs_of(self, path: Path) -> set[str]:
        if path not in self.slug_cache:
            text = path.read_text(encoding="utf-8", errors="replace")
            self.slug_cache[path] = heading_slugs(text)
        return self.slug_cache[path]

    def check_destination(self, source: Path, line: int, dest: str) -> None:
        if not dest or SCHEME.match(dest):
            return
        self.links += 1
        raw_path, _, fragment = dest.partition("#")
        raw_path = raw_path.split("?", 1)[0]
        target = (source.parent / unquote(raw_path)) if raw_path else source
        if not target.exists():
            self.problem(source, line, f"link target not found: {dest}")
        elif fragment and target.is_file() and target.suffix == ".md":
            if unquote(fragment).lower() not in self.slugs_of(target):
                self.problem(source, line, f"anchor not found: {dest}")

    def check_markdown_links(self, path: Path, text: str) -> list[tuple[int, int, int]]:
        """Check the links; return (line, start, end) of each destination."""
        spans: list[tuple[int, int, int]] = []
        for number, line in prose_lines(text):
            for dest, start, end in destinations(line):
                spans.append((number, start, end))
                self.check_destination(path, number, dest)
        return spans

    def check_mentions(self, path: Path, text: str, skip: list[tuple[int, int, int]]) -> None:
        for number, line in enumerate(text.splitlines(), start=1):
            for match in MENTION.finditer(line):
                if any(n == number and s <= match.start() < e for n, s, e in skip):
                    continue
                self.mentions += 1
                if not (self.root / match.group(0)).is_file():
                    self.problem(path, number, f"mentioned file not found: {match.group(0)}")


def run_checks(root: Path) -> tuple[Checker, int]:
    files = walk_files(root)
    docs = markdown_files(files)
    checker = Checker(root)
    spans: dict[Path, list[tuple[int, int, int]]] = {}
    for path in docs:
        text = path.read_text(encoding="utf-8", errors="replace")
        spans[path] = checker.check_markdown_links(path, text)
    for path in mention_files(root, files):
        if is_history(root, path):
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        checker.check_mentions(path, text, spans.get(path, []))
    return checker, len(docs)


def floor_problems(markdown_count: int, links: int) -> list[str]:
    found: list[str] = []
    if markdown_count < MIN_MARKDOWN_FILES:
        found.append(f"  - only {markdown_count} markdown files found, expected at least {MIN_MARKDOWN_FILES}")
    if links < MIN_LINKS:
        found.append(f"  - only {links} links checked, expected at least {MIN_LINKS}")
    return found


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="doc link check (#81)")
    parser.add_argument("--root", type=Path, default=REPO)
    root = parser.parse_args(argv).root.resolve()
    checker, markdown_count = run_checks(root)
    problems = floor_problems(markdown_count, checker.links) + checker.problems
    if problems:
        print("doc links FAILED:")
        print("\n".join(problems))
        return 1
    print(
        f"doc links ok: {markdown_count} markdown files, {checker.links} links, "
        f"{checker.mentions} mentions"
    )
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main(sys.argv[1:]))
