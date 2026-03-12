#!/usr/bin/env python3

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import time
from datetime import datetime
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


def get_backoff_delays() -> list[int]:
    """
    Return increasing delays in seconds: 1, 2, 3, 5, 7, 10, 15, 22, 32, 46, 60, ...
    capping at 600 seconds (10 minutes).
    """
    delays = [1, 2, 3, 5, 7, 10]
    while delays[-1] < 600:
        # Add ~1.5x the previous delay, bounded by 600 max
        next_delay = min(int(delays[-1] * 1.5), 600)
        if next_delay == delays[-1]:
            next_delay = delays[-1] + 30
        delays.append(next_delay)
    return delays


def is_rate_limit_error(output: str) -> bool:
    """Check if the output indicates a 429 rate limit error."""
    return "429" in output or "too many" in output.lower() or "try again" in output.lower()


def is_already_published_error(output: str) -> bool:
    """Check if the output indicates the crate version was already published."""
    lower = output.lower()
    return (
        "already exists" in lower
        or "already published" in lower
        or "cannot overwrite" in lower
        or "crate version already" in lower
    )


def extract_retry_after_timestamp(output: str) -> int | None:
    """
    Extract the retry-after timestamp from crates.io 429 error message.
    Format: "Please try again after Thu, 12 Mar 2026 06:59:09 GMT"
    Returns: seconds to wait until that time, or None if not found.
    """
    import re
    
    # Look for the "Please try again after <timestamp>" pattern
    match = re.search(
        r"Please try again after\s+([A-Za-z]{3},?\s+\d{1,2}\s+[A-Za-z]{3}\s+\d{4}\s+\d{2}:\d{2}:\d{2}\s+GMT)",
        output,
    )
    if not match:
        return None
    
    timestamp_str = match.group(1)
    try:
        # Parse the timestamp: "Thu, 12 Mar 2026 06:59:09 GMT"
        # or "Thu 12 Mar 2026 06:59:09 GMT" (in case comma is missing)
        timestamp_str_clean = timestamp_str.replace(",", "")
        retry_time = datetime.strptime(timestamp_str_clean, "%a %d %b %Y %H:%M:%S %Z")
        # Get current time
        now = datetime.utcnow()
        wait_seconds = int((retry_time - now).total_seconds())
        # Add a small buffer (5 seconds) to ensure the limit has actually reset
        return max(wait_seconds + 5, 0)
    except Exception:
        return None


def publish_with_retry(cmd: list[str], crate: str, backoff_delays: list[int]) -> int:
    """
    Publish a crate with exponential backoff retry on rate limit (429) errors.
    Returns the exit code: 0 on success, 1 if already published (skipped), 2 on other failures.
    """
    full_output = ""
    
    for attempt in enumerate(backoff_delays, start=1):
        attempt_num = attempt[0]
        print(f"\n$ {' '.join(cmd)}")
        if attempt_num > 1:
            print(f"(Retry attempt {attempt_num})")
        
        completed = run(cmd, check=False)
        full_output = completed.stdout + completed.stderr
        
        if completed.stdout:
            print(completed.stdout, end="")
        if completed.stderr:
            print(completed.stderr, end="", file=sys.stderr)
        
        # Success: return immediately
        if completed.returncode == 0:
            return 0
        
        # Check if already published (skip gracefully)
        if is_already_published_error(full_output):
            print(f"(Crate {crate} already published; skipping)", file=sys.stderr)
            return 1  # Return 1 to indicate "skipped"
        
        # Check for rate limit error
        if is_rate_limit_error(full_output):
            # Try to extract the retry-after timestamp from the error message
            wait_seconds = extract_retry_after_timestamp(full_output)
            
            if wait_seconds is None:
                # Fallback to exponential backoff if we can't parse the timestamp
                if attempt_num < len(backoff_delays):
                    wait_seconds = backoff_delays[attempt_num - 1]
                else:
                    # Final attempt exhausted
                    print(
                        f"error: Rate limited (429) but no retry-after timestamp found. "
                        f"Max retries exhausted.",
                        file=sys.stderr,
                    )
                    return 2
            
            if attempt_num < len(backoff_delays):
                print(
                    f"\nerror: Rate limited (429). Waiting {wait_seconds}s "
                    f"({wait_seconds // 60}m {wait_seconds % 60}s) before retry...",
                    file=sys.stderr,
                )
                time.sleep(wait_seconds)
                continue
        
        # Non-rate-limit error or final attempt: fail
        print(f"publish failed for crate {crate}", file=sys.stderr)
        return 2  # Return 2 to indicate fatal error
    
    # All retries exhausted
    print(f"publish failed for crate {crate} (max retries exhausted)", file=sys.stderr)
    return 2


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

    # Get all possible retry delays upfront
    backoff_delays = get_backoff_delays()
    
    # Fixed delay between crates to respect rate limits
    inter_crate_delay = 10  # seconds

    for crate_idx, crate in enumerate(ordered):
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
                # Always pass --allow-dirty for dry-run: nothing is uploaded, so
                # requiring a clean git tree is unnecessarily strict.
                cmd.append("--allow-dirty")
            
            print(f"\n$ {' '.join(cmd)}")
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
        else:
            # Real publish with retry logic
            cmd = ["cargo", "publish", "-p", crate]
            if args.allow_dirty:
                cmd.append("--allow-dirty")

            result = publish_with_retry(cmd, crate, backoff_delays)
            if result == 0:
                # Successfully published
                published_count += 1
            elif result == 1:
                # Already published; skip gracefully
                pass
            else:
                # Fatal error (result == 2 or other)
                print(f"fatal error publishing {crate}", file=sys.stderr)
                return result

        # Wait fixed delay between crates
        if args.execute and crate_idx < len(ordered) - 1:  # Don't wait after the last crate
            print(f"Waiting {inter_crate_delay}s before next crate...")
            time.sleep(inter_crate_delay)

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
