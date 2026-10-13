"""#78: scripts/check-release-pins.py fails CI when the release pipeline
builds from a moving input or can publish what did not pass its gates.

The rules: every remote action is pinned by a full commit SHA with its tag as
a trailing comment; no job runs on a `-latest` runner; every Dockerfile `FROM`
carries a digest; every `cargo build`/`cargo install` in the Dockerfile, the
tarball script and the workflows is `--locked`; the image job needs the
release job; both publishing jobs attest provenance with `id-token: write`
and `attestations: write`; Dependabot covers github-actions, docker and cargo.

Run: python3 -m unittest discover -s scripts/tests -v
Needs nothing but the checkout; no cargo, no network.
"""
import subprocess
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent
CHECK = REPO / "scripts" / "check-release-pins.py"

SHA = "3d3c42e5aac5ba805825da76410c181273ba90b1"
DIGEST = "sha256:" + "ab" * 32

CI = f"""name: CI
on: [push]
jobs:
  quick:
    runs-on: ubuntu-26.04
    steps:
      - uses: actions/checkout@{SHA} # v7.0.1
      - uses: ./.github/actions/local
      - run: scripts/ci.sh
"""

RELEASE = f"""name: Release
on:
  push:
    tags: ["v*.*.*"]
permissions: {{}}
jobs:
  release:
    runs-on: ubuntu-26.04
    permissions:
      contents: write
      id-token: write
      attestations: write
    steps:
      - uses: actions/checkout@{SHA} # v7.0.1
      - run: scripts/release-tarball.sh 1.0.0
      - uses: actions/attest-build-provenance@{SHA} # v4.2.2
        with:
          subject-path: dist/*.tar.gz
  image:
    needs: release
    runs-on: ubuntu-26.04
    permissions:
      contents: read
      packages: write
      id-token: write
      attestations: write
    steps:
      - uses: docker/build-push-action@{SHA} # v7.4.0
        id: push
      - uses: actions/attest-build-provenance@{SHA} # v4.2.2
        with:
          subject-name: ghcr.io/example/wamux
          subject-digest: ${{{{ steps.push.outputs.digest }}}}
          push-to-registry: true
"""

DOCKERFILE = f"""# syntax=docker/dockerfile:1
FROM rust:bookworm@{DIGEST} AS build
RUN --mount=type=cache,target=/usr/local/cargo/registry \\
    cargo build --release --locked -p wamux --bin wamux && \\
    cp target/release/wamux /usr/local/bin/wamux

FROM debian:bookworm-slim@{DIGEST} AS runtime
COPY --from=build /usr/local/bin/wamux /usr/local/bin/wamux
"""

TARBALL = """#!/usr/bin/env bash
set -euo pipefail
cargo build --release --locked -p wamux --bin wamux
"""

DEPENDABOT = """version: 2
updates:
  - package-ecosystem: github-actions
    directory: /
    schedule:
      interval: weekly
  - package-ecosystem: docker
    directory: /
    schedule:
      interval: weekly
  - package-ecosystem: cargo
    directory: /
    schedule:
      interval: monthly
"""

FILES = {
    ".github/workflows/ci.yml": CI,
    ".github/workflows/release.yml": RELEASE,
    ".github/dependabot.yml": DEPENDABOT,
    "Dockerfile": DOCKERFILE,
    "scripts/release-tarball.sh": TARBALL,
}


def run_check(root):
    return subprocess.run(
        [str(CHECK), "--root", str(root)],
        capture_output=True,
        text=True,
        check=False,
    )


def write_tree(root, overrides=None, omit=()):
    """The clean tree, with `overrides` {path: body} replacing files and the
    paths in `omit` left out."""
    for rel, body in {**FILES, **(overrides or {})}.items():
        if rel in omit:
            continue
        path = Path(root) / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(body)


def line_of(body, needle):
    """1-based line of the first line containing `needle`."""
    for number, line in enumerate(body.splitlines(), start=1):
        if needle in line:
            return number
    raise AssertionError(f"fixture has no line with {needle!r}")


class CheckReleasePins(unittest.TestCase):
    def check_tree(self, overrides=None, omit=()):
        with tempfile.TemporaryDirectory() as root:
            write_tree(root, overrides, omit)
            return run_check(root)

    def assert_reported(self, result, *needles):
        """Exit 1 by verdict, not by crash: an uncaught exception also exits 1."""
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, 1, output)
        self.assertNotIn("Traceback", output)
        for needle in needles:
            self.assertIn(needle, output)

    def assert_clean(self, result):
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    # --- the baseline ---

    def test_a_clean_tree_passes(self):
        self.assert_clean(self.check_tree())

    def test_the_repository_passes(self):
        self.assert_clean(run_check(REPO))

    def test_an_empty_tree_fails(self):
        with tempfile.TemporaryDirectory() as root:
            self.assert_reported(run_check(root), "scanned nothing")

    # --- R1: actions pinned by commit SHA, tag as a comment ---

    def test_a_tag_pin_is_reported(self):
        body = CI.replace(f"actions/checkout@{SHA} # v7.0.1", "actions/checkout@v4")
        line = line_of(body, "actions/checkout@v4")
        self.assert_reported(
            self.check_tree({".github/workflows/ci.yml": body}),
            f".github/workflows/ci.yml:{line}:",
            "not pinned by a commit SHA",
        )

    def test_a_sha_without_version_comment_is_reported(self):
        body = CI.replace(" # v7.0.1", "")
        line = line_of(body, "actions/checkout@")
        self.assert_reported(
            self.check_tree({".github/workflows/ci.yml": body}),
            f".github/workflows/ci.yml:{line}:",
            "no version comment",
        )

    def test_a_local_action_is_exempt(self):
        # The clean CI fixture already holds `uses: ./.github/actions/local`.
        self.assertIn("uses: ./", CI)
        self.assert_clean(self.check_tree())

    # --- R2: no moving runner label ---

    def test_a_latest_runner_is_reported(self):
        body = RELEASE.replace("runs-on: ubuntu-26.04", "runs-on: ubuntu-latest", 1)
        line = line_of(body, "ubuntu-latest")
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            f".github/workflows/release.yml:{line}:",
            "runs-on a moving label",
        )

    # --- R3: base images by digest ---

    def test_a_from_without_digest_is_reported(self):
        body = DOCKERFILE.replace(f"debian:bookworm-slim@{DIGEST}", "debian:bookworm-slim")
        line = line_of(body, "FROM debian:bookworm-slim")
        self.assert_reported(
            self.check_tree({"Dockerfile": body}),
            f"Dockerfile:{line}:",
            "FROM without a digest",
        )

    # --- R4: the lockfile the tests ran on is the one the release builds ---

    def test_cargo_build_without_locked_is_reported(self):
        # The cargo call sits on a continuation line of a multi-line RUN.
        body = DOCKERFILE.replace("--release --locked", "--release")
        self.assert_reported(
            self.check_tree({"Dockerfile": body}),
            "Dockerfile:",
            "cargo without --locked",
        )

    def test_the_tarball_script_without_locked_is_reported(self):
        body = TARBALL.replace(" --locked", "")
        line = line_of(body, "cargo build")
        self.assert_reported(
            self.check_tree({"scripts/release-tarball.sh": body}),
            f"scripts/release-tarball.sh:{line}:",
            "cargo without --locked",
        )

    # --- R5: no image unless the gates passed ---

    def test_an_image_job_without_needs_is_reported(self):
        body = RELEASE.replace("    needs: release\n", "")
        line = line_of(body, "  image:")
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            f".github/workflows/release.yml:{line}:",
            "image does not need release",
        )

    # --- R6: provenance for both artifacts ---

    def test_a_job_without_attestation_is_reported(self):
        tarball_attest = (
            f"      - uses: actions/attest-build-provenance@{SHA} # v4.2.2\n"
            "        with:\n"
            "          subject-path: dist/*.tar.gz\n"
        )
        self.assertIn(tarball_attest, RELEASE)
        body = RELEASE.replace(tarball_attest, "")
        line = line_of(body, "  release:")
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            f".github/workflows/release.yml:{line}:",
            "no provenance attestation",
        )

    def test_a_job_without_id_token_is_reported(self):
        image_at = RELEASE.index("  image:")
        image = RELEASE[image_at:].replace("      id-token: write\n", "", 1)
        body = RELEASE[:image_at] + image
        line = line_of(body, "  image:")
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            f".github/workflows/release.yml:{line}:",
            "lacks id-token: write",
        )

    # --- R7: pins get update PRs instead of going stale ---

    def test_a_missing_dependabot_ecosystem_is_reported(self):
        docker = (
            "  - package-ecosystem: docker\n"
            "    directory: /\n"
            "    schedule:\n"
            "      interval: weekly\n"
        )
        self.assertIn(docker, DEPENDABOT)
        self.assert_reported(
            self.check_tree({".github/dependabot.yml": DEPENDABOT.replace(docker, "")}),
            ".github/dependabot.yml:",
            "dependabot does not cover docker",
        )

    def test_no_dependabot_config_is_reported(self):
        self.assert_reported(
            self.check_tree(omit=(".github/dependabot.yml",)),
            "no dependabot config",
        )


if __name__ == "__main__":
    unittest.main()
