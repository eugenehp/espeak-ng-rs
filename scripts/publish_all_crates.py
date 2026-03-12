#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent
ROOT_CARGO_TOML = ROOT / "Cargo.toml"


def run(cmd: list[str], *, check: bool = True) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=ROOT,
        check=check,
        text=True,
        capture_output=True,
    )


def workspace_crate_names() -> list[str]:
    result = run(["cargo", "metadata", "--format-version", "1", "--no-deps"])
    metadata = json.loads(result.stdout)
    packages = metadata.get("packages", [])
    workspace_members = set(metadata.get("workspace_members", []))

    id_to_name = {pkg["id"]: pkg["name"] for pkg in packages}
    names = [id_to_name[pkg_id] for pkg_id in workspace_members if pkg_id in id_to_name]
    return sorted(names)


def ordered_publish_list(crate_names: list[str], include_main: bool) -> list[str]:
    names = set(crate_names)
    ordered: list[str] = []

    def add_if_present(name: str) -> None:
        if name in names and name not in ordered:
            ordered.append(name)

    add_if_present("espeak-ng-data-phonemes")

    dict_crates = sorted(
        name
        for name in names
        if name.startswith("espeak-ng-data-dict-")
    )
    if "espeak-ng-data-dict-ru" in dict_crates:
        ordered.append("espeak-ng-data-dict-ru")
    ordered.extend(name for name in dict_crates if name != "espeak-ng-data-dict-ru")

    add_if_present("espeak-ng-data-dicts")

    if include_main:
        add_if_present("espeak-ng-rs")

    for name in sorted(names):
        if not include_main and name == "espeak-ng-rs":
            continue
        if name not in ordered:
            ordered.append(name)

    return ordered


def has_patch_crates_io() -> bool:
    text = ROOT_CARGO_TOML.read_text(encoding="utf-8")
    return "[patch.crates-io]" in text


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Publish all workspace crates in dependency-safe order."
    )
    parser.add_argument(
        "--execute",
        action="store_true",
        help="Actually publish crates. Without this flag, commands are printed only.",
    )
    parser.add_argument(
        "--no-main",
        action="store_true",
        help="Do not publish the main crate (espeak-ng-rs).",
    )
    parser.add_argument(
        "--allow-patch-main",
        action="store_true",
        help="Allow publishing espeak-ng-rs even if [patch.crates-io] exists in root Cargo.toml.",
    )
    parser.add_argument(
        "--allow-dirty",
        action="store_true",
        help="Pass --allow-dirty to cargo publish.",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="Pass --dry-run to cargo publish (ignored unless --execute is set).",
    )

    args = parser.parse_args()
    include_main = not args.no_main

    crate_names = workspace_crate_names()
    ordered = ordered_publish_list(crate_names, include_main=include_main)

    if include_main and "espeak-ng-rs" in ordered and has_patch_crates_io() and not args.allow_patch_main:
        if args.execute:
            print(
                "error: [patch.crates-io] is present in Cargo.toml; "
                "remove it before publishing espeak-ng-rs, or pass --allow-patch-main",
                file=sys.stderr,
            )
            return 2
        else:
            print(
                "note: [patch.crates-io] is present; execute mode would block publishing espeak-ng-rs "
                "unless --allow-patch-main is set",
                file=sys.stderr,
            )

    print("Publish order:")
    for idx, crate in enumerate(ordered, start=1):
        print(f"  {idx:>3}. {crate}")

    for crate in ordered:
        cmd = ["cargo", "publish", "-p", crate]
        if args.allow_dirty:
            cmd.append("--allow-dirty")
        if args.execute and args.dry_run:
            cmd.append("--dry-run")

        print("\n$", " ".join(cmd))
        if not args.execute:
            continue

        completed = run(cmd, check=False)
        if completed.stdout:
            print(completed.stdout, end="")
        if completed.stderr:
            print(completed.stderr, end="", file=sys.stderr)

        if completed.returncode != 0:
            print(f"publish failed for crate {crate}", file=sys.stderr)
            return completed.returncode

    if args.execute:
        print("\nDone.")
    else:
        print("\nDry-run mode (command preview only). Use --execute to publish.")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
