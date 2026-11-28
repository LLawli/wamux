#!/usr/bin/env python3
"""#218: fail unless an ELF binary is statically linked.

Usage: scripts/check-static-binary.py <binary>

Why: the release ships one musl binary as the tarball and as the scratch
image's only file. A scratch image has no loader and no libc, so a binary that
quietly picked up a `PT_INTERP` or a `DT_NEEDED` would build, pass every test
on a developer machine and die with "exec format error" in the container.

A static-pie (what the musl target links) still has a `PT_DYNAMIC` for its own
relocations but names no library, so the check looks for `DT_NEEDED` entries
rather than for `PT_DYNAMIC`. Stdlib only: the release runner needs no `file`
or `readelf` to run it.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
from elf_dynamic import PT_INTERP, NotElf, needed_libraries, read_segments  # noqa: E402


def findings_for(data: bytes) -> list[str]:
    """Every reason `data` is not static, empty when it is."""
    segments = read_segments(data)
    found = ["has an interpreter" for s in segments if s[0] == PT_INTERP]
    return found + [f"needs {name}" for name in needed_libraries(data, segments)]


def main(argv: list[str]) -> int:
    if len(argv) != 1:
        print("usage: check-static-binary.py <binary>", file=sys.stderr)
        return 2
    path = Path(argv[0])
    try:
        found = findings_for(path.read_bytes())
    except (OSError, NotElf) as error:
        print(f"{path}: {error}", file=sys.stderr)
        return 1
    for reason in found:
        print(f"{path}: {reason}", file=sys.stderr)
    if found:
        return 1
    print(f"static ok: {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
