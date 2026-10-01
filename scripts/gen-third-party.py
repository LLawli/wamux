#!/usr/bin/env python3
"""Generate THIRD-PARTY-LICENSES.md from the dependency graph.

Why this exists: nearly every dependency is MIT or Apache-2.0, and both require
that their copyright notice and license text travel with any distribution of a
binary that includes them. Publishing a tarball or a Docker image without this
file is a license violation, however permissive the licenses are.

Only what is actually shipped is listed: dev-dependencies are excluded (they
never enter the binary) and the graph is resolved for one target platform, so
crates for other operating systems do not inflate the file.

Reads license texts from the local cargo registry, so it needs no network and no
extra tooling. Texts are deduplicated by content: two crates under an identical
MIT text share one entry, while a crate whose notice differs keeps its own.
"""
import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

TARGET = "x86_64-unknown-linux-gnu"
LICENSE_FILENAMES = ("LICENSE", "LICENSE.md", "LICENSE.txt", "LICENSE-MIT",
                     "LICENSE-APACHE", "LICENCE", "COPYING", "UNLICENSE",
                     "LICENSE-MIT.md", "LICENSE-APACHE.md")


SHIPPED_ROOT = "wamux"
REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUTPUT = REPO_ROOT / "THIRD-PARTY-LICENSES.md"


def cargo_metadata():
    """`cargo metadata --locked` for TARGET, run at the repository root (#83).

    --locked: a stale Cargo.lock must fail here instead of being rewritten
    quietly, or the file would describe a graph nobody committed."""
    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--filter-platform", TARGET,
         "--locked"],
        capture_output=True, text=True, check=True, cwd=REPO_ROOT)
    return json.loads(result.stdout)


def find_root_id(meta):
    """Id of the workspace member named SHIPPED_ROOT.

    resolve.root is null on a virtual manifest (#62), so the root is looked up
    among the workspace members instead."""
    members = set(meta.get("workspace_members", []))
    for package in meta.get("packages", []):
        if package["id"] in members and package["name"] == SHIPPED_ROOT:
            return package["id"]
    raise SystemExit(
        f"gen-third-party: workspace member {SHIPPED_ROOT!r} not found in cargo metadata; "
        "refusing to write a license file for an empty graph")


def reachable_ids(meta, root):
    """Ids reachable from root through normal and build edges (dev edges never ship)."""
    nodes = {n["id"]: n for n in (meta.get("resolve") or {}).get("nodes", [])}
    seen, stack = set(), [root]
    while stack:
        current = stack.pop()
        if current in seen:
            continue
        seen.add(current)
        for dep in nodes.get(current, {}).get("deps", []):
            kinds = {k.get("kind") for k in dep.get("dep_kinds", [])}
            # kind None == a normal dependency; "dev" never ships.
            if kinds and kinds <= {"dev"}:
                continue
            stack.append(dep["pkg"])
    return seen


def shipped_packages(meta):
    """Packages reachable from the shipped daemon (SHIPPED_ROOT) through
    normal/build deps only, without the workspace's own members. Raises
    SystemExit with a message when the root is not in the workspace (#83)."""
    by_id = {p["id"]: p for p in meta["packages"]}
    members = set(meta.get("workspace_members", []))
    ids = reachable_ids(meta, find_root_id(meta)) - members
    return sorted((by_id[i] for i in ids), key=lambda p: (p["name"], p["version"]))


def read_license_files(directory):
    """License files named in LICENSE_FILENAMES found directly in directory."""
    found = []
    for name in LICENSE_FILENAMES:
        path = directory / name
        if not path.is_file():
            continue
        try:
            found.append((name, path.read_text(encoding="utf-8", errors="replace").strip()))
        except OSError:
            pass
    return found


def checkout_license_texts(start):
    """Walk up from start to the checkout root (the nearest ancestor holding a
    `.git` entry, inclusive) and return the license files of the first level
    that has any. The root is found BEFORE reading anything, so with no `.git`
    above start it returns nothing instead of adopting an unrelated LICENSE from
    some ancestor such as $HOME."""
    levels = (start, *start.parents)
    root_index = next((i for i, d in enumerate(levels) if (d / ".git").exists()), None)
    if root_index is None:
        return []
    for directory in levels[:root_index + 1]:
        found = read_license_files(directory)
        if found:
            return found
    return []


def license_texts(package):
    """Every license file shipped inside the crate, as (filename, text). A git
    dependency without its own file uses the one at its checkout root (#83).

    Git workspaces keep one LICENSE at the repo root while the crates live in
    subdirectories; registry packages are self-contained, so they never walk up."""
    manifest = Path(package["manifest_path"]).parent
    own = read_license_files(manifest)
    if own or not (package.get("source") or "").startswith("git+"):
        return own
    return checkout_license_texts(manifest.parent)


def render(packages, texts, missing):
    """The whole THIRD-PARTY-LICENSES.md text."""
    out = [
        "# Third-party licenses",
        "",
        "`wamux` is MIT OR Apache-2.0. It links the crates below, whose licenses",
        "require their notices to accompany any distribution of a binary built",
        f"from this source. Generated by `scripts/gen-third-party.py` for `{TARGET}`;",
        "dev-dependencies are excluded because they never reach the binary.",
        "",
        f"**{len(packages)} crates**, {len(texts)} distinct license texts.",
        "",
        "## Crates",
        "",
        "| crate | version | license |",
        "| --- | --- | --- |",
    ]
    for package in packages:
        out.append(f"| {package['name']} | {package['version']} | {package.get('license') or '(see file)'} |")

    out += ["", "## License texts", ""]
    # Ties on crate count fall back to file name then digest, so the order never
    # depends on dict insertion order.
    ordered = sorted(texts.items(), key=lambda kv: (-len(kv[1]["crates"]), kv[1]["file"], kv[0]))
    for _, entry in ordered:
        out.append(f"### {entry['file']}: {len(entry['crates'])} crate(s)")
        out.append("")
        out.append("<details><summary>" + ", ".join(entry["crates"][:6])
                   + (", ..." if len(entry["crates"]) > 6 else "") + "</summary>")
        out += ["", "```", entry["text"], "```", "", "</details>", ""]

    if missing:
        out += ["## Crates with no license file in the published package", "",
                "Their SPDX identifier above is the authoritative statement:", ""]
        out += [f"- {p['name']} {p['version']}: {p.get('license') or 'NO LICENSE FIELD'}" for p in missing]
        out.append("")
    return "\n".join(out)


def collect_texts(packages):
    """Deduplicate license texts by content; also returns packages with none."""
    texts, missing = {}, []
    for package in packages:
        entries = license_texts(package)
        if not entries:
            missing.append(package)
            continue
        for name, text in entries:
            digest = hashlib.sha256(text.encode()).hexdigest()
            texts.setdefault(digest, {"text": text, "file": name, "crates": []})
            texts[digest]["crates"].append(f"{package['name']} {package['version']}")
    return texts, missing


def main(argv=None):
    """Write the file (or `--output PATH`); non-zero with nothing written when
    the graph comes back empty. Returns the exit code."""
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--output", type=Path, default=DEFAULT_OUTPUT)
    args = parser.parse_args(argv)

    packages = shipped_packages(cargo_metadata())
    if not packages:
        print("gen-third-party: the shipped graph is empty; writing nothing", file=sys.stderr)
        return 1
    texts, missing = collect_texts(packages)
    args.output.write_text(render(packages, texts, missing))
    print(f"{args.output}: {len(packages)} crates, {len(texts)} textos, {len(missing)} sem arquivo")
    for package in missing:
        if not package.get("license"):
            print(f"  !! {package['name']} {package['version']}: sem campo license E sem arquivo", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
