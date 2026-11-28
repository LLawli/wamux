"""Read the dynamic-linking facts of an ELF64 little-endian binary: its program
headers, its PT_INTERP and its DT_NEEDED names.

Shared by scripts/check-static-binary.py and scripts/check-android-binary.py so
the release gates parse an ELF one way. Stdlib only: the release runner needs no
`file` or `readelf`.
"""
import struct

ELF_MAGIC = b"\x7fELF"
ELFCLASS64, ELFDATA2LSB = 2, 1
EHDR_SIZE = 64
PHDR = struct.Struct("<IIQQQQQQ")  # type, flags, offset, vaddr, paddr, filesz, memsz, align
DYN = struct.Struct("<qQ")  # d_tag, d_val
PT_LOAD, PT_DYNAMIC, PT_INTERP = 1, 2, 3
DT_NULL, DT_NEEDED, DT_STRTAB = 0, 1, 5

Segment = tuple[int, int, int, int]  # type, file offset, vaddr, file size


class NotElf(Exception):
    """The file is not an ELF this parser can read; the message says why."""


def read_segments(data: bytes) -> list[Segment]:
    """Program headers of a little-endian ELF64, or `NotElf` if it is not one."""
    if data[:4] != ELF_MAGIC:
        raise NotElf("not an ELF file")
    if len(data) < EHDR_SIZE or data[4] != ELFCLASS64 or data[5] != ELFDATA2LSB:
        raise NotElf("not an ELF file of class 64, little endian")
    phoff, = struct.unpack_from("<Q", data, 32)
    phentsize, phnum = struct.unpack_from("<HH", data, 54)
    if phentsize != PHDR.size or phoff + phnum * phentsize > len(data):
        raise NotElf("not an ELF file with a readable program header table")
    rows = (PHDR.unpack_from(data, phoff + i * phentsize) for i in range(phnum))
    return [(kind, offset, vaddr, filesz) for kind, _, offset, vaddr, _, filesz, _, _ in rows]


def vaddr_to_offset(segments: list[Segment], vaddr: int) -> int:
    """File offset of `vaddr`, through the PT_LOAD segment that maps it."""
    for kind, offset, base, filesz in segments:
        if kind == PT_LOAD and base <= vaddr < base + filesz:
            return offset + vaddr - base
    raise NotElf(f"dynamic section points at unmapped address {vaddr:#x}")


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
            raise NotElf("dynamic section has DT_NEEDED but no DT_STRTAB")
        base = vaddr_to_offset(segments, strtab) if wanted else 0
        names += [c_string(data, base + index) for index in wanted]
    return names


def interpreter(data: bytes, segments: list[Segment]) -> str | None:
    """The PT_INTERP path, or None when the binary names no loader."""
    for kind, offset, _, _ in segments:
        if kind == PT_INTERP:
            return c_string(data, offset)
    return None


def machine(data: bytes) -> int:
    """e_machine of an ELF already accepted by `read_segments`."""
    value, = struct.unpack_from("<H", data, 18)
    return value
