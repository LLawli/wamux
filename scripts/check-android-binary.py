#!/usr/bin/env python3
"""#277: fail unless an ELF binary is one Android's loader can start.

Usage: scripts/check-android-binary.py <binary>

Why: the Termux artifacts ship the aarch64-linux-android binary. A static musl
binary starts there but cannot resolve names (no /etc/resolv.conf), a glibc one
has no loader on Android, and one that picked up libc++_shared.so dies at exec
because nothing ships that library. Android carries libc.so, libm.so and
libdl.so in /system/lib64 and nothing else a Rust binary needs, so those are the
only DT_NEEDED entries allowed. Stdlib only, like check-static-binary.py.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "lib"))
from elf_dynamic import NotElf, interpreter, machine, needed_libraries, read_segments  # noqa: E402

EM_AARCH64 = 0xB7
ANDROID_LOADER = "/system/bin/linker64"
SYSTEM_LIBRARIES = {"libc.so", "libm.so", "libdl.so"}


def findings_for(data: bytes) -> list[str]:
    """Every reason `data` would not start on Android, empty when it would."""
    segments = read_segments(data)
    found = [] if machine(data) == EM_AARCH64 else ["not aarch64"]
    found += interpreter_findings(interpreter(data, segments))
    return found + [
        f"needs {name}"
        for name in needed_libraries(data, segments)
        if name not in SYSTEM_LIBRARIES
    ]


def interpreter_findings(found: str | None) -> list[str]:
    if found is None:
        return [f"no interpreter (Android needs {ANDROID_LOADER})"]
    if found != ANDROID_LOADER:
        return [f"interpreter is {found}, Android needs {ANDROID_LOADER}"]
    return []


def main(argv: list[str]) -> int:
    if len(argv) != 1:
        print("usage: check-android-binary.py <binary>", file=sys.stderr)
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
    print(f"android ok: {path}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
