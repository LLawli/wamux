"""#277: scripts/build-deb.py packages a binary that is already built into a
.deb, for Debian (the static musl binary) or for Termux (the Android one).

Compile once, package many: the script never runs cargo, it takes the binary
the release job built and checked, so the .deb carries the same bytes as the
tarball. It is python stdlib (ar + tar.xz) so it builds the same on the release
runner and on a machine with no dpkg-deb; scripts/check-deb.sh then reads the
result with dpkg-deb on the runner. The .deb read back here is parsed by hand:
an ar archive of debian-binary, control.tar.xz and data.tar.xz, in that order.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no network, no dpkg.
"""
import hashlib
import io
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
BUILD = REPO / "scripts" / "build-deb.py"
VERSION = "0.2.4"
BINARY = b"\x7fELF fake wamux binary for the packaging test\n" * 64
TERMUX = "data/data/com.termux/files/usr"
MAINTAINER = "LLawli <LLawli@users.noreply.github.com>"


def read_ar(data):
    """[(name, bytes)] of an ar archive, in archive order."""
    if not data.startswith(b"!<arch>\n"):
        raise AssertionError("not an ar archive")
    members, at = [], 8
    while at < len(data):
        header = data[at : at + 60]
        if header[58:60] != b"`\n":
            raise AssertionError(f"bad ar header at {at}")
        name = header[:16].decode().strip().rstrip("/")
        size = int(header[48:58].decode().strip())
        members.append((name, data[at + 60 : at + 60 + size]))
        at += 60 + size + (size % 2)
    return members


class Deb:
    """A built package, read back."""

    def __init__(self, path):
        self.path = path
        self.members = read_ar(path.read_bytes())
        blobs = dict(self.members)
        self.control_tar = tarfile.open(fileobj=io.BytesIO(blobs["control.tar.xz"]), mode="r:xz")
        self.data_tar = tarfile.open(fileobj=io.BytesIO(blobs["data.tar.xz"]), mode="r:xz")

    def control_file(self, name):
        member = next(m for m in self.control_tar.getmembers() if m.name.lstrip("./") == name)
        return self.control_tar.extractfile(member).read().decode()

    def control(self):
        fields = {}
        for line in self.control_file("control").splitlines():
            if line and not line.startswith(" "):
                key, _, value = line.partition(":")
                fields[key] = value.strip()
        return fields

    def entries(self):
        """{path without ./: TarInfo} for every non-directory data entry."""
        return {
            m.name.removeprefix("./"): m
            for m in self.data_tar.getmembers()
            if not m.isdir()
        }

    def read(self, path):
        member = self.entries()[path]
        return self.data_tar.extractfile(member).read()


def build(flavor, outdir, *extra, binary=BINARY):
    root = Path(outdir)
    exe = root / "input-wamux"
    exe.write_bytes(binary)
    exe.chmod(0o755)
    out = root / "out"
    out.mkdir(exist_ok=True)
    result = subprocess.run(
        [str(BUILD), flavor, VERSION, str(exe), str(out), *extra],
        capture_output=True,
        text=True,
        check=False,
        cwd=REPO,
    )
    return result, out


class BuildDeb(unittest.TestCase):
    def built(self, flavor, *extra):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        result, out = build(flavor, tmp.name, *extra)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        debs = sorted(out.glob("*.deb"))
        self.assertEqual(len(debs), 1, [p.name for p in out.iterdir()])
        return Deb(debs[0])

    def test_debian_deb_is_named_for_amd64(self):
        self.assertEqual(self.built("debian").path.name, f"wamux_{VERSION}_amd64.deb")

    def test_termux_deb_is_named_for_aarch64(self):
        self.assertEqual(self.built("termux").path.name, f"wamux_{VERSION}_aarch64.deb")

    def test_ar_members_are_in_dpkg_order(self):
        for flavor in ("debian", "termux"):
            deb = self.built(flavor)
            names = [name for name, _ in deb.members]
            self.assertEqual(names, ["debian-binary", "control.tar.xz", "data.tar.xz"], flavor)
            self.assertEqual(dict(deb.members)["debian-binary"], b"2.0\n", flavor)

    def test_control_fields(self):
        for flavor, arch in (("debian", "amd64"), ("termux", "aarch64")):
            fields = self.built(flavor).control()
            self.assertEqual(fields["Package"], "wamux", flavor)
            self.assertEqual(fields["Version"], VERSION, flavor)
            self.assertEqual(fields["Architecture"], arch, flavor)
            self.assertEqual(fields["Maintainer"], MAINTAINER, flavor)
            self.assertEqual(fields["Homepage"], "https://github.com/LLawli/wamux", flavor)
            self.assertTrue(fields["Description"], flavor)
            self.assertGreaterEqual(int(fields["Installed-Size"]), len(BINARY) // 1024, flavor)

    def test_only_termux_recommends_termux_services(self):
        self.assertEqual(self.built("termux").control().get("Recommends"), "termux-services")
        self.assertNotIn("termux-services", self.built("debian").control().get("Recommends", ""))

    def test_debian_file_list(self):
        docs = "usr/share/doc/wamux"
        expected = {
            "usr/bin/wamux",
            "usr/lib/systemd/user/wamux.service",
            f"{docs}/README.md",
            f"{docs}/CHANGELOG.md",
            f"{docs}/LICENSE-MIT",
            f"{docs}/LICENSE-APACHE",
            f"{docs}/THIRD-PARTY-LICENSES.md",
            f"{docs}/DEPLOYMENT.md",
            f"{docs}/wamux.toml.example",
        }
        self.assertEqual(set(self.built("debian").entries()), expected)

    def test_debian_unit_starts_the_packaged_binary(self):
        unit = self.built("debian").read("usr/lib/systemd/user/wamux.service").decode()
        self.assertIn("\nExecStart=/usr/bin/wamux\n", unit)
        self.assertNotIn(".local/bin", unit)
        source = (REPO / "contrib" / "wamux.service").read_text()
        self.assertEqual(
            [line for line in unit.splitlines() if not line.startswith("ExecStart=")],
            [line for line in source.splitlines() if not line.startswith("ExecStart=")],
            "only the ExecStart line changes",
        )

    def test_debian_refuses_a_unit_without_execstart(self):
        with tempfile.TemporaryDirectory() as tmp:
            unit = Path(tmp) / "wamux.service"
            unit.write_text("[Service]\nType=simple\n")
            result, out = build("debian", tmp, "--unit", str(unit))
            output = result.stdout + result.stderr
            self.assertEqual(result.returncode, 1, output)
            self.assertNotIn("Traceback", output)
            self.assertIn("ExecStart", output)
            self.assertEqual(list(out.glob("*.deb")), [])

    def test_termux_file_list(self):
        docs = f"{TERMUX}/share/doc/wamux"
        service = f"{TERMUX}/var/service/wamux"
        expected = {
            f"{TERMUX}/bin/wamux",
            f"{service}/run",
            f"{service}/down",
            f"{service}/log/run",
            f"{docs}/README.md",
            f"{docs}/CHANGELOG.md",
            f"{docs}/LICENSE-MIT",
            f"{docs}/LICENSE-APACHE",
            f"{docs}/THIRD-PARTY-LICENSES.md",
            f"{docs}/TERMUX.md",
            f"{docs}/wamux.toml.example",
        }
        self.assertEqual(set(self.built("termux").entries()), expected)

    def test_termux_service_runs_wamux_under_termux_sh(self):
        deb = self.built("termux")
        service = f"{TERMUX}/var/service/wamux"
        run = deb.read(f"{service}/run").decode()
        self.assertTrue(run.startswith("#!/data/data/com.termux/files/usr/bin/sh\n"), run)
        self.assertIn("exec wamux", run)
        entries = deb.entries()
        self.assertEqual(entries[f"{service}/run"].mode & 0o777, 0o755)
        log = entries[f"{service}/log/run"]
        self.assertTrue(log.issym() or log.mode & 0o111, "log/run is runnable")

    def test_binary_is_the_input_bytes_and_executable(self):
        for flavor, path in (("debian", "usr/bin/wamux"), ("termux", f"{TERMUX}/bin/wamux")):
            deb = self.built(flavor)
            self.assertEqual(deb.read(path), BINARY, flavor)
            self.assertEqual(deb.entries()[path].mode & 0o777, 0o755, flavor)

    def test_every_entry_is_owned_by_root(self):
        for flavor in ("debian", "termux"):
            deb = self.built(flavor)
            for tar in (deb.control_tar, deb.data_tar):
                for member in tar.getmembers():
                    self.assertEqual((member.uid, member.gid), (0, 0), f"{flavor} {member.name}")

    def test_md5sums_cover_every_regular_file(self):
        for flavor in ("debian", "termux"):
            deb = self.built(flavor)
            listed = {}
            for line in deb.control_file("md5sums").splitlines():
                digest, _, path = line.partition("  ")
                listed[path] = digest
            files = {path: m for path, m in deb.entries().items() if m.isfile()}
            self.assertEqual(set(listed), set(files), flavor)
            for path in files:
                self.assertEqual(listed[path], hashlib.md5(deb.read(path)).hexdigest(), path)

    def test_build_is_reproducible(self):
        for flavor in ("debian", "termux"):
            first = self.built(flavor).path.read_bytes()
            second = self.built(flavor).path.read_bytes()
            self.assertEqual(first, second, f"{flavor}: same input, same bytes")

    def test_unknown_flavor_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            result, out = build("rpm", tmp)
            output = result.stdout + result.stderr
            self.assertEqual(result.returncode, 2, output)
            self.assertNotIn("Traceback", output)
            self.assertIn("rpm", output)
            self.assertEqual(list(out.glob("*.deb")), [])


if __name__ == "__main__":
    unittest.main()
