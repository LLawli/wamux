"""#78: scripts/check-release-pins.py fails CI when the release pipeline
builds from a moving input or can publish what did not pass its gates.

The rules: every remote action is pinned by a full commit SHA with its tag as
a trailing comment; no job runs on a `-latest` runner; every Dockerfile `FROM`
carries a digest; every `cargo build`/`cargo install` in the Dockerfile, the
tarball script and the workflows is `--locked`; the image job needs the
release job; both publishing jobs attest provenance with `id-token: write`
and `attestations: write`; Dependabot covers github-actions, docker and cargo.

#218 adds: every `Dockerfile*` is held to the digest and `--locked` rules; the
tarball script builds for x86_64-unknown-linux-musl; the release job checks
the binary is static and uploads it; the image job compiles nothing, takes
that binary through the `binary` build context and checks the pushed image
carries the same bytes; the default Dockerfile has a `binary` stage and ends
`FROM scratch`.

#231 adds a universal macOS PKG: the release splits into `linux` and `macos`
builds that run in parallel and a `release` job that publishes only after both;
the `macos` job runs the gates, builds the PKG with scripts/release-pkg.sh (both
Apple targets, `--locked`), checks it with scripts/check-macos-pkg.sh and
attests it; CI runs the macOS gates and the PKG build on every PR. The checks
the `release` job did on the musl binary move to the `linux` job.

#233 adds dev-only rustflags in .cargo/config.toml (`-Zthreads`): every
`cargo build` in the two release scripts runs with an empty RUSTFLAGS, which
cargo reads instead of the config (`unset` does not do it), and no
`Dockerfile*` copies `.cargo` or the whole build context into the image build.

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
  macos:
    runs-on: macos-15
    steps:
      - uses: actions/checkout@{SHA} # v7.0.1
      - run: scripts/ci.sh --no-postgres
  macos-pkg:
    runs-on: macos-15
    steps:
      - uses: actions/checkout@{SHA} # v7.0.1
      - run: scripts/release-pkg.sh 1.0.0
      - run: scripts/check-macos-pkg.sh dist/wamux-1.0.0-macos-universal.pkg
"""

RELEASE = f"""name: Release
on:
  push:
    tags: ["v*.*.*"]
permissions: {{}}
jobs:
  linux:
    runs-on: ubuntu-26.04
    permissions:
      contents: read
      id-token: write
      attestations: write
    steps:
      - uses: actions/checkout@{SHA} # v7.0.1
      - run: scripts/release-tarball.sh 1.0.0
      - run: scripts/check-static-binary.py dist/bin/wamux
      - uses: actions/upload-artifact@{SHA} # v5.0.0
        with:
          name: wamux-binary
          path: dist/bin/wamux
      - uses: actions/attest-build-provenance@{SHA} # v4.2.2
        with:
          subject-path: dist/*.tar.gz
  macos:
    runs-on: macos-15
    permissions:
      contents: read
      id-token: write
      attestations: write
    steps:
      - uses: actions/checkout@{SHA} # v7.0.1
      - run: scripts/ci.sh --no-postgres
      - run: scripts/release-pkg.sh 1.0.0
      - run: scripts/check-macos-pkg.sh dist/wamux-1.0.0-macos-universal.pkg
      - uses: actions/attest-build-provenance@{SHA} # v4.2.2
        with:
          subject-path: dist/*.pkg
      - uses: actions/upload-artifact@{SHA} # v5.0.0
        with:
          name: wamux-pkg
          path: dist/*.pkg*
  release:
    needs: [linux, macos]
    runs-on: ubuntu-26.04
    permissions:
      contents: write
    steps:
      - uses: actions/download-artifact@{SHA} # v6.0.0
        with:
          path: dist
          merge-multiple: true
      - run: gh release create v1.0.0 dist/*.tar.gz dist/*.pkg dist/*.sha256
  image:
    needs: release
    runs-on: ubuntu-26.04
    permissions:
      contents: read
      packages: write
      id-token: write
      attestations: write
    steps:
      - uses: actions/download-artifact@{SHA} # v6.0.0
        with:
          name: wamux-binary
          path: dist/bin
      - uses: docker/build-push-action@{SHA} # v7.4.0
        id: push
        with:
          build-contexts: |
            binary=dist/bin
      - run: scripts/check-image-binary.sh "ghcr.io/example/wamux@${{{{ steps.push.outputs.digest }}}}" dist/bin/wamux
      - uses: actions/attest-build-provenance@{SHA} # v4.2.2
        with:
          subject-name: ghcr.io/example/wamux
          subject-digest: ${{{{ steps.push.outputs.digest }}}}
          push-to-registry: true
"""

DOCKERFILE = f"""# syntax=docker/dockerfile:1
FROM rust:trixie@{DIGEST} AS build
RUN --mount=type=cache,target=/usr/local/cargo/registry \\
    cargo build --release --locked -p wamux --bin wamux \\
      --target x86_64-unknown-linux-musl && \\
    cp target/x86_64-unknown-linux-musl/release/wamux /wamux

FROM scratch AS binary
COPY --from=build /wamux /wamux

FROM scratch
COPY --from=binary /wamux /usr/local/bin/wamux
"""

GLIBC_DOCKERFILE = f"""# syntax=docker/dockerfile:1
FROM rust:trixie@{DIGEST} AS build
RUN --mount=type=cache,target=/usr/local/cargo/registry \\
    cargo build --release --locked -p wamux --bin wamux && \\
    cp target/release/wamux /usr/local/bin/wamux

FROM debian:trixie-slim@{DIGEST} AS runtime
COPY --from=build /usr/local/bin/wamux /usr/local/bin/wamux
"""

TARBALL = """#!/usr/bin/env bash
set -euo pipefail
RUSTFLAGS="" cargo build --release --locked --target x86_64-unknown-linux-musl -p wamux --bin wamux
"""

PKG = """#!/usr/bin/env bash
set -euo pipefail
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  RUSTFLAGS="" cargo build --release --locked --target "$target" -p wamux --bin wamux
done
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
    "Dockerfile.glibc": GLIBC_DOCKERFILE,
    "scripts/release-tarball.sh": TARBALL,
    "scripts/release-pkg.sh": PKG,
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
        body = DOCKERFILE.replace(f"rust:trixie@{DIGEST}", "rust:trixie")
        line = line_of(body, "FROM rust:trixie")
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
        line = line_of(body, "  linux:")
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

    # --- #218: one static musl binary, in the tarball and the image ---

    def test_glibc_dockerfile_from_without_digest_is_flagged(self):
        body = GLIBC_DOCKERFILE.replace(f"debian:trixie-slim@{DIGEST}", "debian:trixie-slim")
        line = line_of(body, "FROM debian:trixie-slim")
        self.assert_reported(
            self.check_tree({"Dockerfile.glibc": body}),
            f"Dockerfile.glibc:{line}:",
            "FROM without a digest",
        )

    def test_glibc_dockerfile_cargo_without_locked_is_flagged(self):
        body = GLIBC_DOCKERFILE.replace("--release --locked", "--release")
        self.assert_reported(
            self.check_tree({"Dockerfile.glibc": body}),
            "Dockerfile.glibc:",
            "cargo without --locked",
        )

    def test_image_job_running_cargo_is_flagged(self):
        image_at = RELEASE.index("  image:")
        image = RELEASE[image_at:].replace(
            "      - uses: docker/build-push-action",
            "      - run: cargo build --release --locked -p wamux\n"
            "      - uses: docker/build-push-action",
            1,
        )
        body = RELEASE[:image_at] + image
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            ".github/workflows/release.yml:",
            "image job runs cargo",
        )

    def test_image_job_without_binary_build_context_is_flagged(self):
        context = "          build-contexts: |\n            binary=dist/bin\n"
        self.assertIn(context, RELEASE)
        body = RELEASE.replace(context, "")
        line = line_of(body, "  image:")
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            f".github/workflows/release.yml:{line}:",
            "image job does not take the release binary",
        )

    def test_image_job_without_image_binary_check_is_flagged(self):
        step = next(l for l in RELEASE.splitlines() if "check-image-binary.sh" in l)
        body = RELEASE.replace(step + "\n", "")
        line = line_of(body, "  image:")
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            f".github/workflows/release.yml:{line}:",
            "image job does not compare the image binary",
        )

    def test_linux_job_without_static_check_is_flagged(self):
        body = RELEASE.replace("      - run: scripts/check-static-binary.py dist/bin/wamux\n", "")
        line = line_of(body, "  linux:")
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            f".github/workflows/release.yml:{line}:",
            "linux job does not check the binary is static",
        )

    def test_linux_job_without_artifact_upload_is_flagged(self):
        upload = (
            f"      - uses: actions/upload-artifact@{SHA} # v5.0.0\n"
            "        with:\n"
            "          name: wamux-binary\n"
            "          path: dist/bin/wamux\n"
        )
        self.assertIn(upload, RELEASE)
        body = RELEASE.replace(upload, "")
        line = line_of(body, "  linux:")
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            f".github/workflows/release.yml:{line}:",
            "linux job does not upload the binary",
        )

    def test_default_dockerfile_not_ending_in_scratch_is_flagged(self):
        last = DOCKERFILE.rindex("FROM scratch")
        body = DOCKERFILE[:last] + f"FROM debian:trixie-slim@{DIGEST}" + DOCKERFILE[last + len("FROM scratch"):]
        self.assert_reported(
            self.check_tree({"Dockerfile": body}),
            "Dockerfile:",
            "final stage is not FROM scratch",
        )

    def test_default_dockerfile_without_binary_stage_is_flagged(self):
        body = DOCKERFILE.replace("FROM scratch AS binary", "FROM scratch AS bin")
        self.assert_reported(
            self.check_tree({"Dockerfile": body}),
            "Dockerfile:",
            "no `binary` stage",
        )

    def test_final_stage_copying_from_the_compile_stage_is_flagged(self):
        # The release replaces only the `binary` stage: a final stage that also
        # copies from the stage running `cargo build` makes BuildKit compile
        # anyway, and the image job is back to building its own binary.
        body = DOCKERFILE + "COPY --from=build /stage/run/wamux /run/wamux\n"
        line = line_of(body, "COPY --from=build /stage")
        self.assert_reported(
            self.check_tree({"Dockerfile": body}),
            f"Dockerfile:{line}:",
            "final stage depends on the compile stage",
        )

    def test_tarball_script_not_building_musl_is_flagged(self):
        body = TARBALL.replace("--target x86_64-unknown-linux-musl ", "")
        self.assert_reported(
            self.check_tree({"scripts/release-tarball.sh": body}),
            "scripts/release-tarball.sh:",
            "does not build x86_64-unknown-linux-musl",
        )

    # --- #231: a universal macOS PKG, built and checked before it is published ---

    def test_release_job_not_needing_linux_is_flagged(self):
        body = RELEASE.replace("    needs: [linux, macos]\n", "    needs: [macos]\n")
        line = line_of(body, "  release:")
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            f".github/workflows/release.yml:{line}:",
            "release does not need linux",
        )

    def test_release_job_not_needing_macos_is_flagged(self):
        body = RELEASE.replace("    needs: [linux, macos]\n", "    needs: linux\n")
        line = line_of(body, "  release:")
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            f".github/workflows/release.yml:{line}:",
            "release does not need macos",
        )

    def macos_job_without(self, needle):
        """RELEASE with the `macos` job's step holding `needle` removed."""
        macos_at = RELEASE.index("  macos:")
        release_at = RELEASE.index("  release:")
        job = RELEASE[macos_at:release_at]
        step = next(l for l in job.splitlines() if needle in l)
        return RELEASE[:macos_at] + job.replace(step + "\n", "", 1) + RELEASE[release_at:]

    def test_macos_job_without_gates_is_flagged(self):
        body = self.macos_job_without("scripts/ci.sh --no-postgres")
        line = line_of(body, "  macos:")
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            f".github/workflows/release.yml:{line}:",
            "macos job does not run the gates",
        )

    def test_macos_job_not_building_the_pkg_is_flagged(self):
        body = self.macos_job_without("scripts/release-pkg.sh")
        line = line_of(body, "  macos:")
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            f".github/workflows/release.yml:{line}:",
            "macos job does not build the pkg",
        )

    def test_macos_job_not_checking_the_pkg_is_flagged(self):
        body = self.macos_job_without("scripts/check-macos-pkg.sh")
        line = line_of(body, "  macos:")
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            f".github/workflows/release.yml:{line}:",
            "macos job does not check the pkg",
        )

    def test_macos_job_without_attestation_is_flagged(self):
        pkg_attest = (
            f"      - uses: actions/attest-build-provenance@{SHA} # v4.2.2\n"
            "        with:\n"
            "          subject-path: dist/*.pkg\n"
        )
        self.assertIn(pkg_attest, RELEASE)
        body = RELEASE.replace(pkg_attest, "")
        line = line_of(body, "  macos:")
        self.assert_reported(
            self.check_tree({".github/workflows/release.yml": body}),
            f".github/workflows/release.yml:{line}:",
            "no provenance attestation",
        )

    def test_pkg_script_without_locked_is_flagged(self):
        body = PKG.replace(" --locked", "")
        line = line_of(body, "cargo build")
        self.assert_reported(
            self.check_tree({"scripts/release-pkg.sh": body}),
            f"scripts/release-pkg.sh:{line}:",
            "cargo without --locked",
        )

    def test_pkg_script_missing_an_apple_target_is_flagged(self):
        body = PKG.replace(" x86_64-apple-darwin", "")
        self.assert_reported(
            self.check_tree({"scripts/release-pkg.sh": body}),
            "scripts/release-pkg.sh:",
            "does not build x86_64-apple-darwin",
        )

    def test_ci_without_a_macos_job_is_flagged(self):
        macos_at = CI.index("  macos:")
        pkg_at = CI.index("  macos-pkg:")
        body = CI[:macos_at] + CI[pkg_at:]
        self.assert_reported(
            self.check_tree({".github/workflows/ci.yml": body}),
            ".github/workflows/ci.yml:",
            "no macOS CI job",
        )

    def test_ci_not_building_the_pkg_is_flagged(self):
        body = CI[: CI.index("  macos-pkg:")]
        self.assert_reported(
            self.check_tree({".github/workflows/ci.yml": body}),
            ".github/workflows/ci.yml:",
            "CI does not build the pkg",
        )


    # --- #233: the release builds without the dev-only rustflags ---

    def test_tarball_cargo_build_without_empty_rustflags_is_flagged(self):
        body = TARBALL.replace('RUSTFLAGS="" ', "")
        line = line_of(body, "cargo build")
        self.assert_reported(
            self.check_tree({"scripts/release-tarball.sh": body}),
            f"scripts/release-tarball.sh:{line}:",
            "does not clear RUSTFLAGS",
        )

    def test_pkg_cargo_build_without_empty_rustflags_is_flagged(self):
        body = PKG.replace('RUSTFLAGS="" ', "")
        line = line_of(body, "cargo build")
        self.assert_reported(
            self.check_tree({"scripts/release-pkg.sh": body}),
            f"scripts/release-pkg.sh:{line}:",
            "does not clear RUSTFLAGS",
        )

    def test_rustflags_with_a_value_is_flagged(self):
        # A non-empty RUSTFLAGS also replaces the config, but with flags of its
        # own: the release must build with none.
        body = TARBALL.replace('RUSTFLAGS=""', 'RUSTFLAGS="-Zthreads=4"')
        line = line_of(body, "cargo build")
        self.assert_reported(
            self.check_tree({"scripts/release-tarball.sh": body}),
            f"scripts/release-tarball.sh:{line}:",
            "does not clear RUSTFLAGS",
        )

    def test_unset_rustflags_is_flagged(self):
        # Without RUSTFLAGS in the environment cargo falls back to
        # `build.rustflags` from .cargo/config.toml.
        body = TARBALL.replace('RUSTFLAGS="" cargo build', "unset RUSTFLAGS\ncargo build")
        line = line_of(body, "cargo build")
        self.assert_reported(
            self.check_tree({"scripts/release-tarball.sh": body}),
            f"scripts/release-tarball.sh:{line}:",
            "does not clear RUSTFLAGS",
        )

    def test_exported_empty_rustflags_is_accepted(self):
        body = PKG.replace('  RUSTFLAGS="" cargo build', "  cargo build").replace(
            "set -euo pipefail\n", 'set -euo pipefail\nexport RUSTFLAGS=""\n', 1
        )
        self.assertIn('export RUSTFLAGS=""', body)
        self.assertNotIn('RUSTFLAGS="" cargo', body)
        self.assert_clean(self.check_tree({"scripts/release-pkg.sh": body}))

    def test_dockerfile_copying_dot_cargo_is_flagged(self):
        stage = f"FROM rust:trixie@{DIGEST} AS build\n"
        body = DOCKERFILE.replace(stage, stage + "COPY .cargo ./.cargo\n", 1)
        line = line_of(body, "COPY .cargo")
        self.assert_reported(
            self.check_tree({"Dockerfile": body}),
            f"Dockerfile:{line}:",
            "copies .cargo",
        )

    def test_dockerfile_copying_the_whole_context_is_flagged(self):
        # `COPY . .` brings .cargo/config.toml along with everything else.
        stage = f"FROM rust:trixie@{DIGEST} AS build\n"
        body = GLIBC_DOCKERFILE.replace(stage, stage + "COPY . .\n", 1)
        line = line_of(body, "COPY . .")
        self.assert_reported(
            self.check_tree({"Dockerfile.glibc": body}),
            f"Dockerfile.glibc:{line}:",
            "copies the whole build context",
        )

if __name__ == "__main__":
    unittest.main()
