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
      scripts/release-tarball.sh and the workflows, `\\` continuations joined
      ("cargo without --locked").
  R5  release.yml: job `image` has `needs: release` ("image does not need release").
  R6  release.yml: jobs `release` and `image` use actions/attest-build-provenance
      and grant `id-token: write` and `attestations: write`.
  R7  .github/dependabot.yml covers github-actions, docker and cargo.

#218, one static musl binary shared by the tarball and the image:

  R8  release.yml, job `image`: no `cargo` ("image job runs cargo"); takes the
      release binary through `build-contexts` with `binary=` ("image job does
      not take the release binary"); runs scripts/check-image-binary.sh ("image
      job does not compare the image binary").
  R9  release.yml, job `release`: runs scripts/check-static-binary.py ("release
      job does not check the binary is static") and uses actions/upload-artifact
      ("release job does not upload the binary").
  R10 the default Dockerfile has a `binary` stage ("no binary stage") and
      its last `FROM` is scratch ("final stage is not FROM scratch"), and the
      final stage copies from no stage that runs `cargo build`: the release
      replaces only `binary`, so such a COPY would compile in the image job
      ("final stage depends on the compile stage").
  R11 scripts/release-tarball.sh names x86_64-unknown-linux-musl and its cargo
      build passes `--target` ("does not build x86_64-unknown-linux-musl").

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
PUBLISH_JOBS = ("release", "image")
MUSL_TARGET = "x86_64-unknown-linux-musl"
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


def needs_release(block: list[str]) -> bool:
    """True when the job's `needs:` is `release` or a list that holds it."""
    for index, line in enumerate(block):
        match = NEEDS.match(line)
        if not match:
            continue
        value = match.group(1).split("#")[0]
        items = [entry.strip() for entry in block[index + 1 :] if entry.strip().startswith("- ")]
        listed = [item[2:].strip() for item in items] if not value.strip() else []
        return "release" in re.split(r"[\s,\[\]]+", value) or "release" in listed
    return False


def check_release_jobs(where: str, lines: list[str]) -> list[Finding]:
    """R5 and R6 over release.yml."""
    blocks = job_blocks(lines)
    found: list[Finding] = []
    if "image" in blocks and not needs_release(blocks["image"][1]):
        found.append((where, blocks["image"][0], "image does not need release"))
    if "image" in blocks:
        found += check_image_job(where, *blocks["image"])
    if "release" in blocks:
        found += check_release_binary(where, *blocks["release"])
    for name in PUBLISH_JOBS:
        if name not in blocks:
            continue
        number, block = blocks[name]
        text = "\n".join(block)
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


def check_release_binary(where: str, number: int, block: list[str]) -> list[Finding]:
    """R9 over the `release` job."""
    text = run_text(block)
    found: list[Finding] = []
    if "scripts/check-static-binary.py" not in text:
        found.append((where, number, "release job does not check the binary is static"))
    if "actions/upload-artifact" not in text:
        found.append((where, number, "release job does not upload the binary"))
    return found


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


def check_dependabot(root: Path) -> list[Finding]:
    """R7."""
    path = root / ".github" / "dependabot.yml"
    where = rel(root, path)
    if not path.is_file():
        return [(where, 1, "no dependabot config")]
    covered = {m.group(1) for line in read_lines(path) if (m := ECOSYSTEM.search(line))}
    return [(where, 1, f"dependabot does not cover {eco}") for eco in ECOSYSTEMS if eco not in covered]


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
    for path in (p for p in dockerfiles if p.is_file()):
        lines = read_lines(path)
        findings += check_from_digests(path.name, lines)
        findings += check_locked(path.name, lines)
    if default_lines:
        findings += check_default_dockerfile(DEFAULT_DOCKERFILE, default_lines)
    tarball = "scripts/release-tarball.sh"
    tarball_lines = read_lines(root / tarball)
    findings += check_locked(tarball, tarball_lines)
    findings += check_musl_tarball(tarball, tarball_lines)
    return findings + check_dependabot(root)


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
    print("release pins ok: actions by SHA, FROM by digest, cargo --locked, image gated, one static musl binary, provenance attested")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
