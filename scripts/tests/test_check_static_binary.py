"""#218: scripts/check-static-binary.py fails the release when the binary it is
about to ship needs a dynamic loader or a shared library.

A static binary has no `PT_INTERP` and no `DT_NEEDED`. A static-pie (what the
musl target produces) still has a `PT_DYNAMIC` for its own relocations, but
names no library, so a check that only looked for `PT_DYNAMIC` would reject
the very binary the release ships. The ELFs here are built by hand, little
endian ELF64, with one `PT_LOAD` mapping the whole file at vaddr 0, so a
virtual address in `.dynamic` equals its file offset.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no network.
"""
import struct
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-static-binary.py"

EHDR_SIZE = 64
PHDR_SIZE = 56
PT_LOAD, PT_DYNAMIC, PT_INTERP = 1, 2, 3
DT_NULL, DT_NEEDED, DT_STRTAB = 0, 1, 5
ET_DYN = 3
EM_X86_64 = 0x3E
INTERPRETER = b"/lib64/ld-linux-x86-64.so.2\0"


def elf(interp=False, dynamic=None):
    """An ELF64 with a `PT_LOAD`, plus a `PT_INTERP` when `interp`, plus a
    `PT_DYNAMIC` when `dynamic` is a list: the names it lists as `DT_NEEDED`
    (an empty list is a static-pie's dynamic section)."""
    phnum = 1 + bool(interp) + (dynamic is not None)
    body_at = EHDR_SIZE + phnum * PHDR_SIZE
    body = b""
    headers = []
    if interp:
        headers.append((PT_INTERP, body_at + len(body), len(INTERPRETER)))
        body += INTERPRETER
    if dynamic is not None:
        strtab = b"\0" + b"".join(name.encode() + b"\0" for name in dynamic)
        strtab_at = body_at + len(body)
        body += strtab
        entries = []
        offset = 1
        for name in dynamic:
            entries.append((DT_NEEDED, offset))
            offset += len(name) + 1
        entries += [(DT_STRTAB, strtab_at), (DT_NULL, 0)]
        table = b"".join(struct.pack("<qQ", tag, value) for tag, value in entries)
        headers.append((PT_DYNAMIC, body_at + len(body), len(table)))
        body += table
    size = body_at + len(body)
    phdrs = struct.pack("<IIQQQQQQ", PT_LOAD, 5, 0, 0, 0, size, size, 0x1000)
    for kind, offset, length in headers:
        phdrs += struct.pack("<IIQQQQQQ", kind, 4, offset, offset, offset, length, length, 8)
    ident = b"\x7fELF" + bytes([2, 1, 1, 0]) + bytes(8)
    ehdr = ident + struct.pack(
        "<HHIQQQIHHHHHH",
        ET_DYN, EM_X86_64, 1, 0, EHDR_SIZE, 0, 0,
        EHDR_SIZE, PHDR_SIZE, phnum, 64, 0, 0,
    )
    return ehdr + phdrs + body


def run_check(path):
    return subprocess.run(
        [str(CHECK), str(path)],
        capture_output=True,
        text=True,
        check=False,
    )


class CheckStaticBinary(unittest.TestCase):
    def check_bytes(self, data):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "wamux"
            path.write_bytes(data)
            return run_check(path)

    def assert_rejected(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        for needle in needles:
            self.assertIn(needle, output)

    def assert_accepted(self, result):
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 0, output)
        self.assertIn("static ok", output)

    def test_static_elf_passes(self):
        self.assert_accepted(self.check_bytes(elf()))

    def test_static_pie_with_dynamic_but_no_needed_passes(self):
        self.assert_accepted(self.check_bytes(elf(dynamic=[])))

    def test_interpreter_fails(self):
        self.assert_rejected(self.check_bytes(elf(interp=True)), "has an interpreter")

    def test_needed_library_fails(self):
        self.assert_rejected(
            self.check_bytes(elf(dynamic=["libc.so.6"])),
            "needs libc.so.6",
        )

    def test_real_dynamic_binary_fails(self):
        # The interpreter running this test is a dynamically linked program.
        self.assert_rejected(
            run_check(Path(sys.executable).resolve()),
            "has an interpreter",
            "needs ",
        )

    def test_not_an_elf_fails(self):
        self.assert_rejected(self.check_bytes(b"#!/bin/sh\necho hi\n"), "not an ELF")


if __name__ == "__main__":
    unittest.main()
