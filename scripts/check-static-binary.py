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
import struct
import sys
from pathlib import Path

ELF_MAGIC = b"\x7fELF"
ELFCLASS64, ELFDATA2LSB = 2, 1
EHDR_SIZE = 64
PHDR = struct.Struct("<IIQQQQQQ")  # type, flags, offset, vaddr, paddr, filesz, memsz, align
DYN = struct.Struct("<qQ")  # d_tag, d_val
PT_LOAD, PT_DYNAMIC, PT_INTERP = 1, 2, 3
DT_NULL, DT_NEEDED, DT_STRTAB = 0, 1, 5

Segment = tuple[int, int, int, int]  # type, file offset, vaddr, file size


class NotStatic(Exception):
    """The file cannot be proven static; the message says why."""


def read_segments(data: bytes) -> list[Segment]:
    """Program headers of a little-endian ELF64, or `NotStatic` if it is not one."""
    if data[:4] != ELF_MAGIC:
        raise NotStatic("not an ELF file")
    if len(data) < EHDR_SIZE or data[4] != ELFCLASS64 or data[5] != ELFDATA2LSB:
        raise NotStatic("not an ELF file of class 64, little endian")
    phoff, = struct.unpack_from("<Q", data, 32)
    phentsize, phnum = struct.unpack_from("<HH", data, 54)
    if phentsize != PHDR.size or phoff + phnum * phentsize > len(data):
        raise NotStatic("not an ELF file with a readable program header table")
    rows = (PHDR.unpack_from(data, phoff + i * phentsize) for i in range(phnum))
    return [(kind, offset, vaddr, filesz) for kind, _, offset, vaddr, _, filesz, _, _ in rows]


def vaddr_to_offset(segments: list[Segment], vaddr: int) -> int:
    """File offset of `vaddr`, through the PT_LOAD segment that maps it."""
    for kind, offset, base, filesz in segments:
        if kind == PT_LOAD and base <= vaddr < base + filesz:
            return offset + vaddr - base
    raise NotStatic(f"dynamic section points at unmapped address {vaddr:#x}")


def dynamic_entries(data: bytes, segment: Segment) -> list[tuple[int, int]]:
    """(tag, value) pairs of a PT_DYNAMIC segment, up to DT_NULL."""
    _, offset, _, filesz = segment
    entries: list[tuple[int, int]] = []
    for at in range(offset, min(offset + filesz, len(data)) - DYN.size + 1, DYN.size):
        tag, value = DYN.unpack_from(data, at)
        if tag == DT_NULL:
            break
        entries.append((tag, value))
    return entries


def c_string(data: bytes, at: int) -> str:
    end = data.find(b"\0", at)
    return data[at : end if end >= 0 else len(data)].decode("utf-8", "replace")


def needed_libraries(data: bytes, segments: list[Segment]) -> list[str]:
    """Names of the DT_NEEDED entries of every PT_DYNAMIC segment."""
    names: list[str] = []
    for segment in (s for s in segments if s[0] == PT_DYNAMIC):
        entries = dynamic_entries(data, segment)
        wanted = [value for tag, value in entries if tag == DT_NEEDED]
        strtab = next((value for tag, value in entries if tag == DT_STRTAB), None)
        if wanted and strtab is None:
            raise NotStatic("dynamic section has DT_NEEDED but no DT_STRTAB")
        base = vaddr_to_offset(segments, strtab) if wanted else 0
        names += [c_string(data, base + index) for index in wanted]
    return names


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
    except (OSError, NotStatic) as error:
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
