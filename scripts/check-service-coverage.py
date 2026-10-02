#!/usr/bin/env python3
"""#68: every RPC of a gRPC service is called by its socket test suite.

Usage: scripts/check-service-coverage.py SERVICE TESTS EXPECTED [--root DIR]
  SERVICE   the service name in crates/wamux-proto/proto (e.g. GroupService)
  TESTS     the test file, or a directory of them, relative to the root
  EXPECTED  how many RPCs the service is known to have

Why: GroupService had 21 RPCs and no test through the socket, so a handler
could break (or a new RPC land) with CI green. This pins the surface: each RPC
in the proto appears as a `.name(` call under TESTS, and the proto holds exactly
EXPECTED of them, so adding an RPC fails here until someone decides how to test
it. A parser that silently finds nothing cannot pass either: an unknown service
fails, and EXPECTED is an absolute number. Reused by #69-#71 for the other
services.

It is a textual check, not a proof of behavior: the tests assert that. It only
stops an RPC from existing with no caller at all.
"""
import argparse
import re
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
RPC = re.compile(r"\brpc\s+(\w+)\s*\(")
CALL = re.compile(r"\.\s*(\w+)\s*\(")
WORD_EDGE = re.compile(r"(?<=[a-z0-9])(?=[A-Z])")


def service_block(text: str, service: str) -> str | None:
    """The body of `service NAME { ... }`, or None when it is not declared."""
    code = "\n".join(line.split("//", 1)[0] for line in text.splitlines())
    start = re.search(rf"\bservice\s+{re.escape(service)}\s*\{{", code)
    if start is None:
        return None
    # Option blocks nest one level (`{ option ...; }`), so track the depth.
    depth, end = 1, start.end()
    while depth and end < len(code):
        depth += {"{": 1, "}": -1}.get(code[end], 0)
        end += 1
    return code[start.end() : end]


def service_rpcs(proto_dir: Path, service: str) -> list[str]:
    """RPC names of `service`, from every .proto under proto_dir (recursive)."""
    for path in sorted(proto_dir.rglob("*.proto")):
        block = service_block(path.read_text(encoding="utf-8"), service)
        if block is not None:
            return RPC.findall(block)
    raise LookupError(f"service {service} is not declared under {proto_dir}")


def snake_case(rpc: str) -> str:
    """`GetInviteLink` -> `get_invite_link`, the name tonic gives the client method."""
    return WORD_EDGE.sub("_", rpc).lower()


def called_rpcs(tests: Path, names: set[str]) -> set[str]:
    """Which of `names` appear as `.name(` in a .rs file at/under `tests`."""
    files = [tests] if tests.is_file() else sorted(tests.rglob("*.rs"))
    found: set[str] = set()
    for path in files:
        for line in path.read_text(encoding="utf-8").splitlines():
            code = line.split("//", 1)[0]
            found.update(n for n in CALL.findall(code) if n in names)
    return found


def problems_for(root: Path, service: str, tests: str, expected: int) -> list[str]:
    """Everything wrong with the service's coverage; empty means ok."""
    try:
        rpcs = service_rpcs(root / "crates" / "wamux-proto" / "proto", service)
    except LookupError as err:
        return [str(err)]
    problems: list[str] = []
    if len(rpcs) != expected:
        problems.append(f"{service} declares {len(rpcs)} RPCs, expected {expected}")
    names = {snake_case(r) for r in rpcs}
    uncalled = sorted(names - called_rpcs(root / tests, names))
    if uncalled:
        problems.append(f"RPCs never called under {tests}: {', '.join(uncalled)}")
    return problems


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="service RPC coverage check (#68)")
    parser.add_argument("service")
    parser.add_argument("tests")
    parser.add_argument("expected", type=int)
    parser.add_argument("--root", type=Path, default=REPO)
    args = parser.parse_args(argv)
    problems = problems_for(args.root, args.service, args.tests, args.expected)
    if problems:
        print("service coverage FAILED:")
        print("\n".join(f"  - {p}" for p in problems))
        return 1
    print(f"service coverage ok: {args.service}, {args.expected} RPCs, each called in {args.tests}")
    return 0


if __name__ == "__main__":
    import sys

    sys.exit(main(sys.argv[1:]))
