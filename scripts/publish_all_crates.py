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
        add_if_present("espeak-ng")

    for name in sorted(names):
        if not include_main and name == "espeak-ng":
            continue
        if name not in ordered:
            ordered.append(name)

    return ordered


def has_patch_crates_io() -> bool:
    text = ROOT_CARGO_TOML.read_text(encoding="utf-8")
    return "[patch.crates-io]" in text


def run_streaming(cmd: list[str]) -> int:
    completed = subprocess.run(cmd, cwd=ROOT, check=False)
    return completed.returncode


def run_preflight_checks() -> int:
    checks: list[list[str]] = [
        ["cargo", "test"],
        # libespeak-ng uses global state; run oracle tests single-threaded to
        # avoid racing even though the Mutex serialises Rust-side calls.
        ["cargo", "test", "--features", "c-oracle,bundled-espeak", "--", "--test-threads=1"],
        [
            "cargo",
            "test",
            "--features",
            "c-oracle,bundled-espeak",
            "--test",
            "oracle_comparison",
            "--",
            "--nocapture",
            "--test-threads=1",
        ],
    ]

    print("\nPreflight checks (required before publish):")
    for cmd in checks:
        print("\n$", " ".join(cmd))
        code = run_streaming(cmd)
        if code != 0:
            print("\nerror: preflight check failed; publish aborted", file=sys.stderr)
            return code
    return 0


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
        help="Do not publish the main crate (espeak-ng).",
    )
    parser.add_argument(
        "--allow-patch-main",
        action="store_true",
        help="Allow publishing espeak-ng even if [patch.crates-io] exists in root Cargo.toml.",
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
    parser.add_argument(
        "--skip-preflight",
        action="store_true",
        help="Skip required preflight test checks before publishing.",
    )

    args = parser.parse_args()
    include_main = not args.no_main

    # --dry-run does not actually publish; skip the heavy preflight suite.
    if args.execute and not args.dry_run and not args.skip_preflight:
        preflight_code = run_preflight_checks()
        if preflight_code != 0:
            return preflight_code

    crate_names = workspace_crate_names()
    ordered = ordered_publish_list(crate_names, include_main=include_main)

    if include_main and "espeak-ng" in ordered and has_patch_crates_io() and not args.allow_patch_main:
        # --dry-run never pushes to crates.io, so the local [patch] section
        # is harmless; allow it through automatically.
        if args.execute and not args.dry_run:
            print(
                "error: [patch.crates-io] is present in Cargo.toml; "
                "remove it before publishing espeak-ng, or pass --allow-patch-main",
                file=sys.stderr,
            )
            return 2
        elif not args.execute:
            print(
                "note: [patch.crates-io] is present; execute mode would block publishing espeak-ng "
                "unless --allow-patch-main is set",
                file=sys.stderr,
            )
        else:
            # --execute --dry-run: nothing real is published, so warn only.
            print(
                "note: [patch.crates-io] is present; this would block a real publish "
                "(ignored because --dry-run is active)",
                file=sys.stderr,
            )

    print("Publish order:")
    for idx, crate in enumerate(ordered, start=1):
        print(f"  {idx:>3}. {crate}")

    for crate in ordered:
        if args.execute and args.dry_run:
            # Use `cargo package` for dry-run: it is fully offline and does the
            # same packaging + verification without contacting crates.io.
            # `cargo publish --dry-run` hits the registry API even in dry-run
            # mode and will hang or fail without credentials.
            #
            # Exception: `espeak-ng` with [patch.crates-io] present cannot be
            # packaged until its data-crate dependencies are live on crates.io.
            # Use `cargo check` instead, which respects the local patches.
            if crate == "espeak-ng" and has_patch_crates_io() and not args.allow_patch_main:
                cmd = ["cargo", "check", "-p", crate]
            else:
                cmd = ["cargo", "package", "-p", crate]
                if args.allow_dirty:
                    cmd.append("--allow-dirty")
        else:
            cmd = ["cargo", "publish", "-p", crate]
            if args.allow_dirty:
                cmd.append("--allow-dirty")

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
        if args.dry_run:
            print("\nDry-run complete (cargo package only; nothing uploaded to crates.io).")
        else:
            print("\nDone.")
    else:
        print("\nDry-run mode (command preview only). Use --execute to publish.")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
