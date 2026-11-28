"""#277: scripts/check-android-binary.py fails the release when the Android
binary it is about to ship would not start in Termux.

A binary Android runs is an aarch64 PIE that the system loader,
/system/bin/linker64, maps, and that names only the libraries every Android
since API 24 carries in /system/lib64: libc.so, libm.so and libdl.so. A static
musl binary starts but cannot resolve names (no /etc/resolv.conf, measured on
Android 16), a glibc one has no loader there, and one that picked up
libc++_shared.so dies at exec because nothing ships that library. The ELFs here
are built by hand, little endian ELF64, with one PT_LOAD mapping the whole file
at vaddr 0, so a virtual address in .dynamic equals its file offset.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no network.
"""
import struct
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-android-binary.py"

EHDR_SIZE = 64
PHDR_SIZE = 56
PT_LOAD, PT_DYNAMIC, PT_INTERP = 1, 2, 3
DT_NULL, DT_NEEDED, DT_STRTAB = 0, 1, 5
ET_DYN = 3
EM_X86_64, EM_AARCH64 = 0x3E, 0xB7
BIONIC = "/system/bin/linker64"
GLIBC_AARCH64 = "/lib/ld-linux-aarch64.so.1"
SYSTEM_LIBS = ["libdl.so", "libm.so", "libc.so"]


def elf(machine=EM_AARCH64, interp=BIONIC, needed=SYSTEM_LIBS):
    """An ELF64 for `machine` with a PT_LOAD, a PT_INTERP naming `interp`
    (none when None) and a PT_DYNAMIC listing `needed` (none when None)."""
    phnum = 1 + (interp is not None) + (needed is not None)
    body_at = EHDR_SIZE + phnum * PHDR_SIZE
    body = b""
    headers = []
    if interp is not None:
        path = interp.encode() + b"\0"
        headers.append((PT_INTERP, body_at + len(body), len(path)))
        body += path
    if needed is not None:
        strtab = b"\0" + b"".join(name.encode() + b"\0" for name in needed)
        strtab_at = body_at + len(body)
        body += strtab
        entries = []
        offset = 1
        for name in needed:
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
        ET_DYN, machine, 1, 0, EHDR_SIZE, 0, 0,
        EHDR_SIZE, PHDR_SIZE, phnum, 64, 0, 0,
    )
    return ehdr + phdrs + body


class CheckAndroidBinary(unittest.TestCase):
    def check_bytes(self, data):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "wamux"
            path.write_bytes(data)
            return subprocess.run(
                [str(CHECK), str(path)],
                capture_output=True,
                text=True,
                check=False,
            )

    def assert_rejected(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        self.assertNotIn("android ok", output)
        for needle in needles:
            self.assertIn(needle, output)

    def test_bionic_aarch64_with_system_libraries_passes(self):
        result = self.check_bytes(elf())
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 0, output)
        self.assertIn("android ok", output)

    def test_a_subset_of_the_system_libraries_passes(self):
        result = self.check_bytes(elf(needed=["libc.so"]))
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_x86_64_machine_fails(self):
        self.assert_rejected(self.check_bytes(elf(machine=EM_X86_64)), "not aarch64")

    def test_static_binary_fails(self):
        self.assert_rejected(
            self.check_bytes(elf(interp=None, needed=None)),
            "no interpreter",
        )

    def test_glibc_interpreter_fails(self):
        self.assert_rejected(
            self.check_bytes(elf(interp=GLIBC_AARCH64)),
            GLIBC_AARCH64,
            BIONIC,
        )

    def test_a_library_android_does_not_ship_fails(self):
        self.assert_rejected(
            self.check_bytes(elf(needed=["libc.so", "libc++_shared.so"])),
            "needs libc++_shared.so",
        )

    def test_not_an_elf_fails(self):
        self.assert_rejected(self.check_bytes(b"#!/bin/sh\necho hi\n"), "not an ELF")


if __name__ == "__main__":
    unittest.main()
