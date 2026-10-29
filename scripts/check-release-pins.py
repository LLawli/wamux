#!/usr/bin/env python3
"""#78: the release pipeline builds from fixed inputs and publishes only what passed.

Usage: scripts/check-release-pins.py [--root DIR]   (DIR defaults to the repo)

Why: a tag like `@v4` or `rust:bookworm` moves under us, so the artifact we
ship is not the one we tested; a `cargo build` without `--locked` may resolve
a different dependency tree than the lockfile CI ran on; an image job that
does not wait for the release job can publish what failed its gates; and
without a provenance attestation nobody can tie a binary back to this repo.
Rules, checked line by line (no YAML parser, stdlib only):

  R1  a remote `uses:` is pinned by a 40-hex commit SHA, with `# v...` after it
      ("not pinned by a commit SHA" / "no version comment"); `./` is exempt.
  R2  no `runs-on:` ending in `-latest` ("runs-on a moving label").
  R3  every `FROM` of every root `Dockerfile*` carries `@sha256:<64 hex>`
      ("FROM without a digest"); `scratch` is exempt.
  R4  `cargo build`/`cargo install` is `--locked` in every root `Dockerfile*`,
      scripts/release-tarball.sh, scripts/release-pkg.sh and the workflows, `\\` continuations joined
      ("cargo without --locked").
  R5  release.yml: job `image` has `needs: release` ("image does not need release").
  R6  release.yml: jobs `linux`, `macos` and `image` (the ones that produce an
      artifact) use actions/attest-build-provenance and grant `id-token: write`
      and `attestations: write`. `release` only publishes what they built.
  R7  .github/dependabot.yml covers github-actions, docker and cargo.

#218, one static musl binary shared by the tarball and the image:

  R8  release.yml, job `image`: no `cargo` ("image job runs cargo"); takes the
      release binary through `build-contexts` with `binary=` ("image job does
      not take the release binary"); runs scripts/check-image-binary.sh ("image
      job does not compare the image binary").
  R9  release.yml, job `linux`: runs scripts/check-static-binary.py ("linux
      job does not check the binary is static") and uses actions/upload-artifact
      ("linux job does not upload the binary").
  R10 the default Dockerfile has a `binary` stage ("no binary stage") and
      its last `FROM` is scratch ("final stage is not FROM scratch"), and the
      final stage copies from no stage that runs `cargo build`: the release
      replaces only `binary`, so such a COPY would compile in the image job
      ("final stage depends on the compile stage").
  R11 scripts/release-tarball.sh names x86_64-unknown-linux-musl and its cargo
      build passes `--target` ("does not build x86_64-unknown-linux-musl").

#231, a universal macOS PKG next to the musl tarball:

  R12 release.yml, job `release`: `needs:` holds `linux` and `macos` ("release
      does not need linux" / "release does not need macos"), so nothing is
      published unless both builds passed.
  R13 release.yml, job `macos`: runs `scripts/ci.sh --no-postgres` ("macos job
      does not run the gates"), scripts/release-pkg.sh ("macos job does not
      build the pkg") and scripts/check-macos-pkg.sh ("macos job does not check
      the pkg").
  R14 scripts/release-pkg.sh names both Apple targets and its cargo build
      passes `--target` ("does not build aarch64-apple-darwin" / "does not
      build x86_64-apple-darwin").
  R15 ci.yml: a job on a `macos-*` runner runs `scripts/ci.sh --no-postgres`
      ("no macOS CI job"), and a job runs both scripts/release-pkg.sh and
      scripts/check-macos-pkg.sh ("CI does not build the pkg"), so the PKG path
      is exercised on every PR and not only at a tag.

Floor: at least MIN_WORKFLOWS workflows and one `FROM`, so a layout change
cannot make the check pass by scanning nothing ("scanned nothing").
"""
import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MIN_WORKFLOWS = 2
ECOSYSTEMS = ("github-actions", "docker", "cargo")
ATTEST_JOBS = ("linux", "macos", "image")
MUSL_TARGET = "x86_64-unknown-linux-musl"
APPLE_TARGETS = ("aarch64-apple-darwin", "x86_64-apple-darwin")
GATES_COMMAND = "scripts/ci.sh --no-postgres"
PKG_BUILD_SCRIPT = "scripts/release-pkg.sh"
PKG_CHECK_SCRIPT = "scripts/check-macos-pkg.sh"
DEFAULT_DOCKERFILE = "Dockerfile"
PERMISSIONS = ("id-token: write", "attestations: write")
ATTEST_ACTION = "actions/attest-build-provenance"

USES = re.compile(r"^\s*-?\s*uses:\s*(\S+)(.*)$")
SHA_PIN = re.compile(r"@[0-9a-f]{40}$")
VERSION_COMMENT = re.compile(r"^\s*#\s*v\d")
RUNS_ON = re.compile(r"^\s*runs-on:\s*(\S+)")
FROM_LINE = re.compile(r"^FROM\s+(?:--\S+\s+)*(\S+)", re.IGNORECASE)
DIGEST = re.compile(r"@sha256:[0-9a-f]{64}$")
STAGE_NAME = re.compile(r"^FROM\s+\S+\s+AS\s+(\S+)$", re.IGNORECASE)
COPY_FROM = re.compile(r"^COPY\s+(?:--\S+\s+)*--from=(\S+)", re.IGNORECASE)
CARGO_WORD = re.compile(r"\bcargo\b")
BINARY_CONTEXT = re.compile(r"^\s*binary=\S")
BINARY_STAGE = re.compile(r"^FROM\s+.*\s+AS\s+binary\s*$", re.IGNORECASE)
CARGO_CALL = re.compile(r"\bcargo\s+(build|install)\b")
JOB_HEADER = re.compile(r"^  ([A-Za-z0-9_-]+):\s*$")
NEEDS = re.compile(r"^    needs:\s*(.*)$")
ECOSYSTEM = re.compile(r"package-ecosystem:\s*[\"']?([A-Za-z0-9_-]+)")

Finding = tuple[str, int, str]


def read_lines(path: Path) -> list[str]:
    """Lines of `path`, or none when it does not exist."""
    if not path.is_file():
        return []
    return path.read_text(encoding="utf-8").splitlines()


def workflow_files(root: Path) -> list[Path]:
    """Every workflow under .github/workflows, sorted."""
    folder = root / ".github" / "workflows"
    return sorted([*folder.glob("*.yml"), *folder.glob("*.yaml")])


def rel(root: Path, path: Path) -> str:
    return path.relative_to(root).as_posix()


def check_uses_and_runners(where: str, lines: list[str]) -> list[Finding]:
    """R1 and R2 over one workflow."""
    found: list[Finding] = []
    for number, line in enumerate(lines, start=1):
        uses = USES.match(line)
        if uses and not uses.group(1).startswith("./"):
            if not SHA_PIN.search(uses.group(1)):
                found.append((where, number, f"{uses.group(1)} is not pinned by a commit SHA"))
            elif not VERSION_COMMENT.match(uses.group(2)):
                found.append((where, number, f"{uses.group(1)} has no version comment (# vX.Y.Z)"))
        runner = RUNS_ON.match(line)
        if runner and runner.group(1).endswith("-latest"):
            found.append((where, number, f"{runner.group(1)}: runs-on a moving label"))
    return found


def check_from_digests(where: str, lines: list[str]) -> list[Finding]:
    """R3 over the Dockerfile."""
    found: list[Finding] = []
    for number, line in enumerate(lines, start=1):
        base = FROM_LINE.match(line)
        if base and base.group(1) != "scratch" and not DIGEST.search(base.group(1)):
            found.append((where, number, f"FROM without a digest: {base.group(1)}"))
    return found


def joined_commands(lines: list[str]) -> list[tuple[int, str]]:
    """(first line number, text) of each command, `\\` continuations joined and
    full-line comments dropped."""
    commands: list[tuple[int, str]] = []
    start, text = 0, ""
    for number, line in enumerate(lines, start=1):
        if not text and line.lstrip().startswith("#"):
            continue
        start = start or number
        text += " " + line.strip().rstrip("\\")
        if not line.rstrip().endswith("\\"):
            commands.append((start, text))
            start, text = 0, ""
    return commands


def check_locked(where: str, lines: list[str]) -> list[Finding]:
    """R4 over one file."""
    return [
        (where, number, "cargo without --locked")
        for number, command in joined_commands(lines)
        if CARGO_CALL.search(command) and "--locked" not in command
    ]


def job_blocks(lines: list[str]) -> dict[str, tuple[int, list[str]]]:
    """job name -> (header line number, the job's lines), from `jobs:` on."""
    blocks: dict[str, tuple[int, list[str]]] = {}
    current = ""
    in_jobs = False
    for number, line in enumerate(lines, start=1):
        if line.startswith("jobs:"):
            in_jobs = True
        elif in_jobs and (header := JOB_HEADER.match(line)):
            current = header.group(1)
            blocks[current] = (number, [])
        elif in_jobs and current:
            blocks[current][1].append(line)
    return blocks


def needs_job(block: list[str], job: str) -> bool:
    """True when the job's `needs:` is `job` or a list (flow or block) that holds it."""
    for index, line in enumerate(block):
        match = NEEDS.match(line)
        if not match:
            continue
        value = match.group(1).split("#")[0]
        items = [entry.strip() for entry in block[index + 1 :] if entry.strip().startswith("- ")]
        listed = [item[2:].strip() for item in items] if not value.strip() else []
        return job in re.split(r"[\s,\[\]]+", value) or job in listed
    return False


def check_release_jobs(where: str, lines: list[str]) -> list[Finding]:
    """R5, R6, R8, R9, R12 and R13 over release.yml."""
    blocks = job_blocks(lines)
    found: list[Finding] = []
    if "image" in blocks and not needs_job(blocks["image"][1], "release"):
        found.append((where, blocks["image"][0], "image does not need release"))
    for name, check in JOB_CHECKS.items():
        if name in blocks:
            found += check(where, *blocks[name])
    for name in ATTEST_JOBS:
        if name in blocks:
            found += check_attestation(where, name, *blocks[name])
    return found


def check_attestation(where: str, name: str, number: int, block: list[str]) -> list[Finding]:
    """R6 over one artifact-producing job."""
    text = "\n".join(block)
    found: list[Finding] = []
    if ATTEST_ACTION not in text:
        found.append((where, number, f"job {name}: no provenance attestation"))
    for permission in PERMISSIONS:
        if not re.search(rf"^\s+{re.escape(permission)}\s*$", text, re.MULTILINE):
            found.append((where, number, f"job {name} lacks {permission}"))
    return found


def run_text(block: list[str]) -> str:
    """The job's lines without full-line comments, so prose cannot trip a rule."""
    return "\n".join(line for line in block if not line.lstrip().startswith("#"))


def check_image_job(where: str, number: int, block: list[str]) -> list[Finding]:
    """R8 over the `image` job."""
    text = run_text(block)
    found: list[Finding] = []
    if CARGO_WORD.search(text):
        found.append((where, number, "image job runs cargo"))
    if not any(BINARY_CONTEXT.match(line) for line in text.splitlines()) or "build-contexts" not in text:
        found.append((where, number, "image job does not take the release binary"))
    if "scripts/check-image-binary.sh" not in text:
        found.append((where, number, "image job does not compare the image binary"))
    return found


def check_linux_binary(where: str, number: int, block: list[str]) -> list[Finding]:
    """R9 over the `linux` job."""
    text = run_text(block)
    found: list[Finding] = []
    if "scripts/check-static-binary.py" not in text:
        found.append((where, number, "linux job does not check the binary is static"))
    if "actions/upload-artifact" not in text:
        found.append((where, number, "linux job does not upload the binary"))
    return found


def check_release_needs(where: str, number: int, block: list[str]) -> list[Finding]:
    """R12 over the `release` job."""
    return [
        (where, number, f"release does not need {builder}")
        for builder in ("linux", "macos")
        if not needs_job(block, builder)
    ]


def check_macos_release_job(where: str, number: int, block: list[str]) -> list[Finding]:
    """R13 over the `macos` job."""
    text = run_text(block)
    required = (
        (GATES_COMMAND, "macos job does not run the gates"),
        (PKG_BUILD_SCRIPT, "macos job does not build the pkg"),
        (PKG_CHECK_SCRIPT, "macos job does not check the pkg"),
    )
    return [(where, number, reason) for needle, reason in required if needle not in text]


JOB_CHECKS = {
    "image": check_image_job,
    "linux": check_linux_binary,
    "macos": check_macos_release_job,
    "release": check_release_needs,
}


def check_default_dockerfile(where: str, lines: list[str]) -> list[Finding]:
    """R10 over the default Dockerfile."""
    froms = [(n, m) for n, line in enumerate(lines, start=1) if (m := FROM_LINE.match(line))]
    found: list[Finding] = []
    if froms and froms[-1][1].group(1).lower() != "scratch":
        found.append((where, froms[-1][0], "final stage is not FROM scratch"))
    if froms and not any(BINARY_STAGE.match(line) for line in lines):
        found.append((where, 1, "no `binary` stage"))
    return found + check_final_stage_copies(where, lines)


def compile_stages(lines: list[str]) -> set[str]:
    """Names of the stages whose commands run `cargo build`."""
    names: set[str] = set()
    current = ""
    for _, command in joined_commands(lines):
        stage = STAGE_NAME.match(command.strip())
        if stage:
            current = stage.group(1)
        elif current and re.search(r"\bcargo\s+build\b", command):
            names.add(current)
    return names


def check_final_stage_copies(where: str, lines: list[str]) -> list[Finding]:
    """The final stage copies from no compile stage."""
    last = max((n for n, line in enumerate(lines, start=1) if FROM_LINE.match(line)), default=0)
    compiling = compile_stages(lines)
    found: list[Finding] = []
    for number, line in enumerate(lines[last:], start=last + 1):
        copy = COPY_FROM.match(line)
        if copy and copy.group(1) in compiling:
            found.append((where, number, "final stage depends on the compile stage"))
    return found


def check_musl_tarball(where: str, lines: list[str]) -> list[Finding]:
    """R11 over scripts/release-tarball.sh. The triple may sit in a variable the
    build line expands, so the file must name it and the build must pass `--target`."""
    commands = [text for _, text in joined_commands(lines)]
    names_musl = any(MUSL_TARGET in text for text in commands)
    targeted = any(CARGO_CALL.search(text) and "--target" in text for text in commands)
    if names_musl and targeted:
        return []
    return [(where, 1, f"does not build {MUSL_TARGET}")]


def check_pkg_script(where: str, lines: list[str]) -> list[Finding]:
    """R14 over scripts/release-pkg.sh. The triples may sit in a loop variable the
    build line expands, so the file must name each one and the build pass `--target`."""
    commands = [text for _, text in joined_commands(lines)]
    targeted = any(CARGO_CALL.search(text) and "--target" in text for text in commands)
    return [
        (where, 1, f"does not build {target}")
        for target in APPLE_TARGETS
        if not (targeted and any(target in text for text in commands))
    ]


def runs_on_macos(block: list[str]) -> bool:
    """True when the job's `runs-on:` is a `macos-*` runner."""
    matches = (RUNS_ON.match(line) for line in block)
    return any(m and m.group(1).startswith("macos-") for m in matches)


def check_ci_macos(where: str, lines: list[str]) -> list[Finding]:
    """R15 over ci.yml."""
    texts = [(run_text(block), block) for _, block in job_blocks(lines).values()]
    found: list[Finding] = []
    if not any(GATES_COMMAND in text and runs_on_macos(block) for text, block in texts):
        found.append((where, 1, "no macOS CI job runs the gates"))
    if not any(PKG_BUILD_SCRIPT in text and PKG_CHECK_SCRIPT in text for text, _ in texts):
        found.append((where, 1, "CI does not build the pkg"))
    return found


def check_dependabot(root: Path) -> list[Finding]:
    """R7."""
    path = root / ".github" / "dependabot.yml"
    where = rel(root, path)
    if not path.is_file():
        return [(where, 1, "no dependabot config")]
    covered = {m.group(1) for line in read_lines(path) if (m := ECOSYSTEM.search(line))}
    return [(where, 1, f"dependabot does not cover {eco}") for eco in ECOSYSTEMS if eco not in covered]


def scan_release_scripts(root: Path) -> list[Finding]:
    """R4, R11 and R14 over the two scripts that build the shipped artifacts."""
    tarball = "scripts/release-tarball.sh"
    tarball_lines = read_lines(root / tarball)
    pkg_lines = read_lines(root / PKG_BUILD_SCRIPT)
    return [
        *check_locked(tarball, tarball_lines),
        *check_musl_tarball(tarball, tarball_lines),
        *check_locked(PKG_BUILD_SCRIPT, pkg_lines),
        *check_pkg_script(PKG_BUILD_SCRIPT, pkg_lines),
    ]


def scan(root: Path) -> list[Finding]:
    """Every violation under `root`, plus the floor."""
    findings: list[Finding] = []
    workflows = workflow_files(root)
    dockerfiles = sorted(root.glob("Dockerfile*"))
    default_lines = read_lines(root / DEFAULT_DOCKERFILE)
    has_from = any(FROM_LINE.match(line) for line in default_lines)
    if len(workflows) < MIN_WORKFLOWS or not has_from:
        findings.append((".", 0, f"scanned nothing useful: {len(workflows)} workflows, FROM found: {has_from}"))
    for path in workflows:
        lines = read_lines(path)
        findings += check_uses_and_runners(rel(root, path), lines)
        findings += check_locked(rel(root, path), lines)
        if path.name == "release.yml":
            findings += check_release_jobs(rel(root, path), lines)
        if path.name == "ci.yml":
            findings += check_ci_macos(rel(root, path), lines)
    for path in (p for p in dockerfiles if p.is_file()):
        lines = read_lines(path)
        findings += check_from_digests(path.name, lines)
        findings += check_locked(path.name, lines)
    if default_lines:
        findings += check_default_dockerfile(DEFAULT_DOCKERFILE, default_lines)
    return findings + scan_release_scripts(root) + check_dependabot(root)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="release pins check (#78)")
    parser.add_argument("--root", type=Path, default=REPO)
    root = parser.parse_args(argv).root
    findings = scan(Path(root))
    for where, line, reason in findings:
        print(f"{where}:{line}: {reason}")
    if findings:
        print(f"release pins FAILED: {len(findings)} violations")
        return 1
    print("release pins ok: actions by SHA, FROM by digest, cargo --locked, image gated, one static musl binary, a macOS PKG built and checked, provenance attested")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
