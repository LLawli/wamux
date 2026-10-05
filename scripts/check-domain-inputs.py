#!/usr/bin/env python3
"""#63 / #115 / #116: no generated `pb::*` input struct and no `String`
identifier reaches `domain/` or `state/`.

Usage: scripts/check-domain-inputs.py [--root DIR]   (DIR defaults to the repo)

Why: the wire schema leaked into every layer. 19 of the 24 domain files took
generated `pb::*Request` structs, so a proto change rippled through business
logic, and a jid or a message id was a bare string to the compiler. The
services now convert once, at the boundary, into the types in `wamux-types`;
this check fails when a function in `crates/wamux/src/domain` or
`crates/wamux/src/state` takes one again (a parameter typed `pb::XRequest`,
`&pb::XHeader`, `&[pb::Mention]`, `Option<&pb::QuoteContext>`,
`Vec<pb::PollVote>`, ...).

#116 adds the other half of #63: an identifier travels as a named type, not a
string. A parameter or field named for one (`group`, `jid`, `jids`,
`participant(s)`, `chat`, `sender`, `creator`, `voter`, `recipient(s)`,
`query`) typed `&str`, `String`, `&[String]` or `Vec<String>` fails too.

Not flagged: building a response or an event (`-> pb::SendResult`,
`pb::SendResult { .. }`), which the proto issues (#72-#74) reach. Test files
(`*_tests.rs`, `tests.rs`, anything under a `tests/` directory) are skipped:
tests build requests to feed the conversions.

PENDING names the files that still take one, each with the issue that removes
it. A pending file with no hit fails too, so the list only shrinks. The scan must read at least MIN_SCANNED files, so a scan that
reads nothing cannot pass.
"""
import argparse
import re
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SCANNED = ("crates/wamux/src/domain", "crates/wamux/src/state")
MIN_SCANNED = 20
PENDING = {
    "crates/wamux/src/domain/event_mapping.rs": "#74 (event construction: map_sent's wire key, chat and sender)",
}
INPUT = re.compile(
    r":\s*(?:(?:Option|Vec)<|&|\[|mut\s+)*pb::[A-Za-z0-9_:]*?"
    r"(?:Request|Header|MessageKey|Mention|QuoteContext|LinkPreview|MediaDescriptor|PollVote|Reply)\b"
)

STRING_ID = re.compile(
    r"\b(?:group|jid|jids|participant|participants|chat|sender|creator|voter|recipient|recipients|query)"
    r"\s*:\s*(?:&\[String\]|Vec<String>|&str|String)\s*[,)]"
)


def is_test_file(path: Path) -> bool:
    return path.name == "tests.rs" or path.name.endswith("_tests.rs") or "tests" in path.parts


def inputs(path: Path) -> list[tuple[int, str]]:
    """(line, text) of each `pb::*` input or `String` identifier outside a `//` comment."""
    hits = []
    for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        code = line.split("//", 1)[0]
        if INPUT.search(code) or STRING_ID.search(code):
            hits.append((number, line.strip()))
    return hits


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="no pb:: inputs or String ids in domain/ and state/ (#115, #116)")
    parser.add_argument("--root", type=Path, default=REPO)
    args = parser.parse_args(argv)
    problems: list[str] = []
    scanned = 0
    for directory in SCANNED:
        for path in sorted((args.root / directory).rglob("*.rs")):
            rel = path.relative_to(args.root).as_posix()
            if is_test_file(path.relative_to(args.root)):
                continue
            scanned += 1
            hits = inputs(path)
            if rel in PENDING:
                if not hits:
                    problems.append(f"{rel} takes no pb:: input any more: remove it from PENDING")
                continue
            problems += [f"{rel}:{n}: {text}" for n, text in hits]
    if scanned < MIN_SCANNED:
        problems.append(
            f"scanned {scanned} files under {', '.join(SCANNED)}, expected at least "
            f"{MIN_SCANNED}: the layout moved or the scan broke"
        )
    if problems:
        print("domain input check FAILED (convert at the service into a wamux-types type):")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    print(f"no pb:: input or String id in {scanned} domain/state files ({len(PENDING)} pending: {', '.join(sorted(set(PENDING.values())))})")
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main(sys.argv[1:]))
