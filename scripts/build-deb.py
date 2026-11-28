#!/usr/bin/env python3
"""#277: package an already built wamux binary as a .deb.

Usage: scripts/build-deb.py <debian|termux> <version> <binary> <outdir> [--unit PATH]

Why: the release compiles once per platform and packages many times, so the
.deb carries the same bytes as the tarball the release job checked. This script
never runs cargo. It is stdlib only (ar + tar.xz + md5), so it builds the same
on the release runner and on a machine with no dpkg-deb; scripts/check-deb.sh
reads the result back with dpkg-deb.

The output is reproducible: every timestamp is SOURCE_DATE_EPOCH (0 when unset),
owners are root, entries are sorted, and the tar has no PAX time headers.
"""
import argparse
import hashlib
import io
import lzma
import os
import sys
import tarfile
from dataclasses import dataclass
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TERMUX_ROOT = "data/data/com.termux/files/usr"
HOMEPAGE = "https://github.com/LLawli/wamux"
MAINTAINER = "LLawli <LLawli@users.noreply.github.com>"
SUMMARY = "WhatsApp multiplexer core (gRPC over a Unix socket)"
DETAIL = (
    " A relay that keeps many WhatsApp accounts connected in one process and"
    " exposes\n them over a local gRPC API on a Unix domain socket. It has no"
    " auth of its own:\n the socket's filesystem permissions are the boundary.\n"
)
LICENSES = ["LICENSE-MIT", "LICENSE-APACHE", "THIRD-PARTY-LICENSES.md"]


class PackagingError(Exception):
    """The inputs cannot make a package; the message says what is wrong."""


@dataclass(frozen=True)
class Flavor:
    architecture: str
    root: str  # install prefix inside the package, no leading slash
    docs: list[str]
    recommends: str | None


FLAVORS = {
    "debian": Flavor(
        "amd64", "usr",
        ["README.md", "CHANGELOG.md", *LICENSES, "docs/DEPLOYMENT.md", "wamux.toml.example"],
        None,
    ),
    "termux": Flavor(
        "aarch64", TERMUX_ROOT,
        ["README.md", "CHANGELOG.md", *LICENSES, "docs/TERMUX.md", "wamux.toml.example"],
        "termux-services",
    ),
}

File = tuple[str, bytes, int]  # path without ./, content, mode


def debian_unit(unit: Path) -> bytes:
    """The systemd unit with only its ExecStart= line pointed at /usr/bin/wamux."""
    lines = unit.read_text().splitlines()
    hits = [i for i, line in enumerate(lines) if line.startswith("ExecStart=")]
    if not hits:
        raise PackagingError(f"{unit}: no ExecStart= line to point at /usr/bin/wamux")
    for i in hits:
        lines[i] = "ExecStart=/usr/bin/wamux"
    return ("\n".join(lines) + "\n").encode()


def payload(path: Path) -> bytes:
    try:
        return path.read_bytes()
    except OSError as error:
        raise PackagingError(f"cannot read {path}: {error}") from error


def data_files(flavor: str, binary: Path, unit: Path) -> list[File]:
    spec = FLAVORS[flavor]
    docs = f"{spec.root}/share/doc/wamux"
    files: list[File] = [(f"{spec.root}/bin/wamux", payload(binary), 0o755)]
    files += [(f"{docs}/{Path(name).name}", payload(REPO / name), 0o644) for name in spec.docs]
    if flavor == "debian":
        files.append(("usr/lib/systemd/user/wamux.service", debian_unit(unit), 0o644))
        return files
    service = f"{spec.root}/var/service/wamux"
    contrib = REPO / "contrib" / "termux"
    files.append((f"{service}/run", payload(contrib / "run"), 0o755))
    files.append((f"{service}/down", b"", 0o644))
    files.append((f"{service}/log/run", payload(contrib / "log" / "run"), 0o755))
    return files


def tar_member(name: str, mtime: int, mode: int, content: bytes | None) -> tarfile.TarInfo:
    info = tarfile.TarInfo(name)
    info.mtime, info.mode = mtime, mode
    info.uid = info.gid = 0
    info.uname = info.gname = "root"
    info.type = tarfile.DIRTYPE if content is None else tarfile.REGTYPE
    info.size = 0 if content is None else len(content)
    return info


def build_tar(entries: list[File], mtime: int, with_parents: bool) -> bytes:
    """An xz tar of `entries` under ./, with their parent directories first."""
    dirs = {"."}
    if with_parents:
        dirs |= {f"./{p}" for path, _, _ in entries for p in map(str, Path(path).parents) if p != "."}
    members = [(d, 0o755, None) for d in dirs]
    members += [(f"./{path}", mode, content) for path, content, mode in entries]
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w", format=tarfile.USTAR_FORMAT) as tar:
        for name, mode, content in sorted(members, key=lambda m: m[0]):
            info = tar_member("./" if name == "." else name, mtime, mode, content)
            tar.addfile(info, None if content is None else io.BytesIO(content))
    return lzma.compress(buffer.getvalue(), format=lzma.FORMAT_XZ, preset=9)


def control_text(flavor: str, version: str, files: list[File]) -> bytes:
    spec = FLAVORS[flavor]
    kib = -(-sum(len(content) for _, content, _ in files) // 1024)
    fields = [
        ("Package", "wamux"), ("Version", version), ("Architecture", spec.architecture),
        ("Maintainer", MAINTAINER), ("Installed-Size", str(kib)), ("Homepage", HOMEPAGE),
    ]
    if spec.recommends:
        fields.append(("Recommends", spec.recommends))
    head = "".join(f"{key}: {value}\n" for key, value in fields)
    return f"{head}Description: {SUMMARY}\n{DETAIL}".encode()


def md5sums(files: list[File]) -> bytes:
    rows = sorted((path, hashlib.md5(content).hexdigest()) for path, content, _ in files)
    return "".join(f"{digest}  {path}\n" for path, digest in rows).encode()


def ar_member(name: str, content: bytes, mtime: int) -> bytes:
    header = f"{name + '/':<16}{mtime:<12}{0:<6}{0:<6}{'100644':<8}{len(content):<10}`\n"
    pad = b"\n" if len(content) % 2 else b""
    return header.encode() + content + pad


def build_deb(flavor: str, version: str, binary: Path, unit: Path, mtime: int) -> bytes:
    files = data_files(flavor, binary, unit)
    control = [("control", control_text(flavor, version, files), 0o644),
               ("md5sums", md5sums(files), 0o644)]
    members = [
        ("debian-binary", b"2.0\n"),
        ("control.tar.xz", build_tar(control, mtime, with_parents=False)),
        ("data.tar.xz", build_tar(files, mtime, with_parents=True)),
    ]
    return b"!<arch>\n" + b"".join(ar_member(n, c, mtime) for n, c in members)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(prog="build-deb.py", description=__doc__.split("\n")[0])
    parser.add_argument("flavor", choices=sorted(FLAVORS))
    parser.add_argument("version")
    parser.add_argument("binary", type=Path)
    parser.add_argument("outdir", type=Path)
    parser.add_argument("--unit", type=Path, default=REPO / "contrib" / "wamux.service")
    return parser.parse_args(argv)


def main(argv: list[str]) -> int:
    args = parse_args(argv)  # argparse exits 2 on a bad flavor, naming the value
    mtime = int(os.environ.get("SOURCE_DATE_EPOCH", "0"))
    try:
        deb = build_deb(args.flavor, args.version, args.binary, args.unit, mtime)
    except PackagingError as error:
        print(f"build-deb.py: {error}", file=sys.stderr)
        return 1
    # Written only after the whole package is built, so a refusal leaves no .deb.
    arch = FLAVORS[args.flavor].architecture
    target = args.outdir / f"wamux_{args.version}_{arch}.deb"
    args.outdir.mkdir(parents=True, exist_ok=True)
    target.write_bytes(deb)
    print(f"built {target}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
