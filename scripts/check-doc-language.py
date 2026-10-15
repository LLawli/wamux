#!/usr/bin/env python3
"""#82: docs and comments are written in English.

Usage: scripts/check-doc-language.py [--root DIR]   (DIR defaults to the repo)

Why: contributors and edge authors read English; without a check, a new
Portuguese paragraph or comment would only be caught by a reader. Two kinds of
text are read:
- every Markdown file, outside fenced code blocks and inline code spans;
- the comments (line, doc, block) of every .rs and .proto file under crates/.

String literals are data, not prose: tests carry real WhatsApp content (poll
options, captions) in Portuguese on purpose, so a small lexer separates
comments from strings, raw strings, chars and lifetimes. Text in double quotes
inside a comment is quoted data for the same reason. THIRD-PARTY-LICENSES.md is
generated from other people's license texts and is skipped. Hidden directories
and `target` are pruned and never followed. MIN_MARKDOWN_FILES and
MIN_SOURCE_FILES keep a walk that silently finds nothing from passing.
"""
import argparse
import os
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MIN_MARKDOWN_FILES = 10
MIN_SOURCE_FILES = 20
SKIPPED_MARKDOWN = ("THIRD-PARTY-LICENSES.md",)
SOURCE_SUFFIXES = (".rs", ".proto")

# Measured: each word appears in the Portuguese docs and zero times in the
# repo's English prose and comments. `com` is left out (it matches `.com`),
# and so are `para`, `como` and `esta` (a Portuguese test string uses them).
PORTUGUESE_WORDS = (
    "não são também está então você já isso uma porque quando depois só até "
    "pelo pela que sem agora foi"
).split()
# \w is Unicode-aware, so `deque` and `agorafobia` do not match `que`/`agora`.
WORD = re.compile(
    r"(?<!\w)(" + "|".join(PORTUGUESE_WORDS) + r")(?!\w)", re.IGNORECASE
)
FENCE = re.compile(r"^ {0,3}(```|~~~)")
CODE_SPAN = re.compile(r"(`+)(?:(?!\1).)+?\1")
QUOTED = re.compile(r'"[^"\n]*"')

Hit = tuple[int, str, str]  # (line, word, stripped line text)


def walk_files(root: Path) -> list[Path]:
    """Every file under root, hidden directories and `target` pruned, sorted."""
    found: list[Path] = []
    for here, dirs, names in os.walk(root, followlinks=False):
        dirs[:] = [d for d in dirs if not d.startswith(".") and d != "target"]
        found.extend(Path(here) / name for name in names)
    return sorted(found)


def is_markdown(root: Path, path: Path) -> bool:
    rel = path.relative_to(root).as_posix()
    return path.suffix == ".md" and rel not in SKIPPED_MARKDOWN


def is_source(root: Path, path: Path) -> bool:
    rel = path.relative_to(root).parts
    return path.suffix in SOURCE_SUFFIXES and rel[0] == "crates"


def read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8", errors="replace")


def blank_spans(line: str) -> str:
    return CODE_SPAN.sub(lambda m: " " * len(m.group(0)), line)


def markdown_prose(text: str) -> list[tuple[int, str]]:
    """(1-based line, text) outside fenced code; inline code spans blanked."""
    prose: list[tuple[int, str]] = []
    marker = None
    for number, line in enumerate(text.splitlines(), start=1):
        fence = FENCE.match(line)
        if marker is None and fence:
            marker = fence.group(1)
        elif marker is not None and fence and fence.group(1) == marker:
            marker = None
        elif marker is None:
            prose.append((number, blank_spans(line)))
    return prose


def skip_block_comment(text: str, i: int, nested: bool) -> int:
    """Index after the block comment opening at i."""
    depth = 1
    i += 2
    while i < len(text) and depth:
        if nested and text.startswith("/*", i):
            depth += 1
            i += 2
        elif text.startswith("*/", i):
            depth -= 1
            i += 2
        else:
            i += 1
    return i


def read_quoted(text: str, i: int, quote: str) -> int:
    """Index after the quoted literal opening at i; end of text if unclosed."""
    j = i + 1
    while j < len(text):
        if text[j] == "\\":
            j += 2
        elif text[j] == quote:
            return j + 1
        else:
            j += 1
    return len(text)


def raw_string_end(text: str, i: int) -> int:
    """Index after the raw string whose `r` is at i; -1 if it is not one."""
    j = i + 1
    while j < len(text) and text[j] == "#":
        j += 1
    if j >= len(text) or text[j] != '"':
        return -1
    closer = '"' + "#" * (j - i - 1)
    end = text.find(closer, j + 1)
    return len(text) if end < 0 else end + len(closer)


def is_identifier_char(char: str) -> bool:
    return char.isalnum() or char == "_"


def starts_raw_prefix(text: str, i: int) -> bool:
    """An `r` at i opens a raw string unless it ends an identifier (`br`/`cr` ok)."""
    prev = text[i - 1] if i else ""
    if not is_identifier_char(prev):
        return True
    return prev in "bc" and (i < 2 or not is_identifier_char(text[i - 2]))


def char_or_lifetime(text: str, i: int) -> int:
    """Index after the char literal at i, or after the tick of a lifetime."""
    if i + 1 < len(text) and text[i + 1] == "\\":
        return read_quoted(text, i, "'")
    if i + 2 < len(text) and text[i + 2] == "'":
        return i + 3
    return i + 1


def source_comments(text: str, is_rust: bool) -> list[tuple[int, str]]:
    """(1-based line, comment text) for every comment line; strings skipped."""
    comments: list[tuple[int, str]] = []
    i = 0
    while i < len(text):
        i = next_token(text, i, is_rust, comments)
    return comments


def next_token(text: str, i: int, is_rust: bool, comments: list) -> int:
    """Consume one token at i, appending any comment it holds; return the new i."""
    char = text[i]
    if text.startswith("//", i):
        end = text.find("\n", i)
        end = len(text) if end < 0 else end
        add_comment(comments, text, i, end)
        return end
    if text.startswith("/*", i):
        end = skip_block_comment(text, i, nested=is_rust)
        add_comment(comments, text, i, end)
        return end
    if char == '"' or (char == "'" and not is_rust):
        return read_quoted(text, i, char)
    if is_rust and char == "'":
        return char_or_lifetime(text, i)
    if is_rust and char == "r" and starts_raw_prefix(text, i):
        end = raw_string_end(text, i)
        return end if end > 0 else i + 1
    return i + 1


def add_comment(comments: list, text: str, start: int, end: int) -> None:
    """Split the comment text into one entry per physical line."""
    first = text.count("\n", 0, start) + 1
    for offset, line in enumerate(text[start:end].split("\n")):
        comments.append((first + offset, line))


def hits_in(lines: list[tuple[int, str]]) -> list[Hit]:
    found: list[Hit] = []
    for number, line in lines:
        match = WORD.search(line)
        if match:
            found.append((number, match.group(1), line.strip()))
    return found


def drop_quoted(lines: list[tuple[int, str]]) -> list[tuple[int, str]]:
    return [(number, QUOTED.sub(" ", line)) for number, line in lines]


def check_file(root: Path, path: Path, markdown: bool) -> list[str]:
    text = read_text(path)
    if markdown:
        lines = markdown_prose(text)
    else:
        lines = drop_quoted(source_comments(text, path.suffix == ".rs"))
    rel = path.relative_to(root).as_posix()
    return [f"{rel}:{n}: {word}: {shown}" for n, word, shown in hits_in(lines)]


def check_tree(root: Path) -> tuple[list[str], int, int]:
    files = [f for f in walk_files(root) if not f.is_symlink()]
    markdown = [f for f in files if is_markdown(root, f)]
    sources = [f for f in files if is_source(root, f)]
    problems: list[str] = []
    for path in markdown:
        problems += check_file(root, path, markdown=True)
    for path in sources:
        problems += check_file(root, path, markdown=False)
    return problems, len(markdown), len(sources)


def floor_problems(markdown: int, sources: int) -> list[str]:
    problems: list[str] = []
    if markdown < MIN_MARKDOWN_FILES:
        problems.append(f"only {markdown} markdown files found, expected at least {MIN_MARKDOWN_FILES}")
    if sources < MIN_SOURCE_FILES:
        problems.append(f"only {sources} sources found, expected at least {MIN_SOURCE_FILES}")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", type=Path, default=REPO)
    root = parser.parse_args().root.resolve()
    problems, markdown, sources = check_tree(root)
    problems += floor_problems(markdown, sources)
    if problems:
        print("doc language FAILED")
        print("\n".join(problems))
        return 1
    print(f"doc language ok: {markdown} markdown files, {sources} sources")
    return 0


if __name__ == "__main__":
    sys.exit(main())
