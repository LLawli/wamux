#!/usr/bin/env python3
"""#237: no job restores the Rust cache under a key the runner image moves.

Usage: scripts/check-rust-cache.py [--root DIR]   (DIR defaults to the repo)

Why: rust-cache hashes every toolchain `rustup toolchain list` reports into its
key, and the hosted images ship a preinstalled stable whose version changes
between images. The same commit then restores on one image and builds cold on
the next. The cache step runs inside the composite action, after the toolchain
is installed, so only a step placed before the action can take the stable out of
the hash. Rule, checked line by line (no YAML parser, stdlib only):

  every cached Rust setup has, earlier in the same job, a step whose `run` holds
  both `rustup toolchain list` and `rustup toolchain uninstall`
  ("caches before the runner's toolchains are dropped"). A cached setup is
  `actions-rust-lang/setup-rust-toolchain` unless its `cache:` is false (absent
  means true, the action's default) or `Swatinem/rust-cache` called directly.
  Uninstalling one toolchain by name does not count: a different preinstalled
  one on a future image would end up in the hash again.

Floor: at least MIN_CACHED_STEPS cached setups in total, so swapping the action
cannot make the check pass by scanning nothing ("scanned nothing").
"""
import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MIN_CACHED_STEPS = 1
DROP_MARKERS = ("rustup toolchain list", "rustup toolchain uninstall")

SETUP_ACTION = re.compile(r"\buses:\s*actions-rust-lang/setup-rust-toolchain@")
RUST_CACHE_ACTION = re.compile(r"\buses:\s*Swatinem/rust-cache@")
CACHE_OFF = re.compile(r"^\s+cache:\s*[\"']?false[\"']?\s*(#.*)?$")
USES_NAME = re.compile(r"\buses:\s*([^@\s]+)")
JOB_HEADER = re.compile(r"^  ([A-Za-z0-9_-]+):\s*$")
STEP_START = re.compile(r"^      - ")

Finding = tuple[str, int, str]
Step = tuple[int, list[str]]


def workflow_files(root: Path) -> list[Path]:
    """Every workflow under .github/workflows, sorted."""
    folder = root / ".github" / "workflows"
    return sorted([*folder.glob("*.yml"), *folder.glob("*.yaml")])


def job_steps(lines: list[str]) -> list[list[Step]]:
    """Per job, its steps as (line number of the step's first line, step lines)."""
    jobs: list[list[Step]] = []
    in_jobs = False
    for number, line in enumerate(lines, start=1):
        if line.startswith("jobs:"):
            in_jobs = True
        elif in_jobs and JOB_HEADER.match(line):
            jobs.append([])
        elif in_jobs and jobs and STEP_START.match(line):
            jobs[-1].append((number, [line]))
        elif in_jobs and jobs and jobs[-1]:
            jobs[-1][-1][1].append(line)
    return jobs


def is_cached_setup(step: list[str]) -> bool:
    """True for a setup-rust-toolchain with cache on (the default) or a direct rust-cache."""
    if any(RUST_CACHE_ACTION.search(line) for line in step):
        return True
    if not any(SETUP_ACTION.search(line) for line in step):
        return False
    return not any(CACHE_OFF.match(line) for line in step)


def is_toolchain_drop(step: list[str]) -> bool:
    """True when the step lists and uninstalls toolchains (comments do not count)."""
    code = "\n".join(line for line in step if not line.lstrip().startswith("#"))
    return all(marker in code for marker in DROP_MARKERS)


def uses_line(number: int, step: list[str]) -> tuple[int, str]:
    """Line number and action name of the step's `uses:`."""
    for offset, line in enumerate(step):
        found = USES_NAME.search(line)
        if found:
            return number + offset, found.group(1)
    return number, "cached setup"


def scan_job(where: str, steps: list[Step]) -> tuple[int, list[Finding]]:
    """(cached steps seen, findings) for one job, in step order."""
    dropped = False
    cached = 0
    found: list[Finding] = []
    for number, step in steps:
        if is_toolchain_drop(step):
            dropped = True
        elif is_cached_setup(step):
            cached += 1
            if not dropped:
                line, action = uses_line(number, step)
                found.append((where, line, f"{action} caches before the runner's toolchains are dropped (#237)"))
    return cached, found


def scan(root: Path) -> list[Finding]:
    """Every cached Rust setup with no toolchain drop before it, plus the floor."""
    cached = 0
    found: list[Finding] = []
    for path in workflow_files(root):
        where = path.relative_to(root).as_posix()
        lines = path.read_text(encoding="utf-8").splitlines()
        for steps in job_steps(lines):
            count, findings = scan_job(where, steps)
            cached += count
            found += findings
    if cached < MIN_CACHED_STEPS:
        found.append((".", 0, "scanned nothing: no cached Rust setup found"))
    return found


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description="rust cache key check (#237)")
    parser.add_argument("--root", type=Path, default=REPO)
    root = parser.parse_args(argv).root
    findings = scan(Path(root))
    for where, line, reason in findings:
        print(f"{where}:{line}: {reason}")
    if findings:
        print(f"rust cache FAILED: {len(findings)} violations")
        return 1
    print("rust cache ok: every cached Rust setup runs after the runner's toolchains are dropped")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
