#!/usr/bin/env python3
"""Render a `tauri-plugin-updater` v2 static update manifest.

This is the single source of truth for the manifest contract. It is a
standalone script so that `publish-update.test.sh` can assert the
contract's *semantics* — not merely that a file exists — on any machine,
without an S3 endpoint or the AWS CLI.

The manifest shape is dictated by `tauri-plugin-updater`:

    {
      "version": "1.2.3",
      "pub_date": "2026-11-03T12:00:00Z",
      "notes": "…",
      "platforms": {
        "<os>-<arch>": { "url": "https://…", "signature": "<base64>" }
      }
    }

Two rules that are easy to get wrong and that this script enforces:

* `platforms` is keyed by the *updater target* (`linux-x86_64`,
  `darwin-aarch64`, `windows-x86_64`, optionally suffixed with the
  bundle type). The plugin looks up `{os}-{arch}-{bundle_type}` and
  then `{os}-{arch}` — a Rust target triple such as
  `x86_64-unknown-linux-gnu` is never found.
* `signature` holds the *content* of the `.sig` file (the base64
  minisign signature), not a URL to it. The plugin base64-decodes this
  field and verifies the downloaded archive against it.

Usage:

    render-manifest.py --version 1.2.3 --pub-date 2026-11-03T12:00:00Z \\
        --notes "…" --url https://…/app.tar.gz \\
        --signature-file app.tar.gz.sig --targets linux-x86_64 \\
        [--existing old-manifest.json] [--out manifest.json]

With no `--out`, the manifest is written to stdout.
"""

from __future__ import annotations

import argparse
import base64
import binascii
import json
import os
import re
import sys
from datetime import datetime
from typing import NoReturn

# `tauri-plugin-updater` derives the target from the running OS and
# architecture: `updater_os()` yields linux | darwin | windows and
# `updater_arch()` yields x86_64 | aarch64 | i686 | armv7 | riscv64.
# The optional third segment is the bundle type.
TARGET_RE = re.compile(
    r"^(linux|darwin|windows)"
    r"-(x86_64|aarch64|i686|armv7|riscv64)"
    r"(-(appimage|deb|rpm|app|msi|nsis))?$"
)

SEMVER_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+([+-][A-Za-z0-9.-]+)?$")


class ContractError(Exception):
    """A violation of the updater manifest contract."""


def fail(message: str) -> NoReturn:
    raise ContractError(message)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="render-manifest.py",
        description="Render a tauri-plugin-updater static update manifest.",
    )
    parser.add_argument("--version", required=True, help="semver version, e.g. 1.2.3")
    parser.add_argument("--pub-date", required=True, help="RFC 3339 publication timestamp")
    parser.add_argument("--notes", default="", help="release notes")
    parser.add_argument("--url", required=True, help="absolute URL of the update artefact")
    parser.add_argument(
        "--signature-file",
        required=True,
        help="path to the .sig file whose CONTENT becomes the manifest signature",
    )
    parser.add_argument(
        "--targets",
        required=True,
        help="comma-separated updater target keys, e.g. linux-x86_64",
    )
    parser.add_argument(
        "--existing",
        default=None,
        help="path to the manifest currently published for this channel (optional)",
    )
    parser.add_argument(
        "--repoint",
        action="store_true",
        help=(
            "allow a target's URL to change for a version that is already "
            "published. Off by default: repointing a live version silently "
            "would serve different bytes to clients that already cached the "
            "manifest. Use it ONLY to repair a publication that was broken "
            "and therefore never reachable (e.g. a manifest published "
            "against the wrong hostname)."
        ),
    )
    parser.add_argument("--out", default=None, help="write the manifest here (default: stdout)")
    return parser.parse_args(argv)


def validate_version(version: str) -> None:
    if not SEMVER_RE.match(version):
        fail(f"version '{version}' is not a plain semver string")
    if version.startswith("v"):
        fail(f"version '{version}' must not carry a 'v' prefix")


def validate_pub_date(pub_date: str) -> None:
    # The plugin parses this with `time::Rfc3339`.
    try:
        datetime.fromisoformat(pub_date.replace("Z", "+00:00"))
    except ValueError:
        fail(f"pub_date '{pub_date}' is not an RFC 3339 timestamp")


def validate_url(url: str) -> None:
    if not url.startswith("https://"):
        fail(f"url '{url}' must be an absolute https URL")
    if "{{" in url:
        fail(f"url '{url}' contains an unresolved template placeholder")


def validate_targets(targets: list[str]) -> None:
    if not targets:
        fail("at least one target is required")
    for target in targets:
        if not TARGET_RE.match(target):
            fail(
                f"target '{target}' is not an updater target key; expected "
                "<os>-<arch>[-<installer>] such as linux-x86_64"
            )


def read_signature(path: str) -> str:
    try:
        with open(path, "r", encoding="utf-8") as handle:
            signature = handle.read().strip()
    except OSError as exc:
        fail(f"cannot read the signature file {path}: {exc}")
    if not signature:
        fail(f"the signature file {path} is empty")
    try:
        base64.b64decode(signature, validate=True)
    except (binascii.Error, ValueError) as exc:
        fail(f"the signature file {path} is not valid base64: {exc}")
    return signature


def load_existing(path: str | None) -> dict:
    """Load the manifest currently published for the channel.

    Returns an empty dict when there is nothing to merge. A manifest
    that is present but malformed is a hard error: silently replacing it
    would hide a corrupt or tampered channel index.
    """
    if not path or not os.path.exists(path):
        return {}
    try:
        with open(path, "r", encoding="utf-8") as handle:
            raw = handle.read()
    except OSError as exc:
        fail(f"cannot read the existing manifest {path}: {exc}")
    try:
        existing = json.loads(raw)
    except json.JSONDecodeError as exc:
        fail(f"the existing manifest {path} is not valid JSON: {exc}")
    if not isinstance(existing, dict):
        fail(f"the existing manifest {path} is not a JSON object")
    return existing


def merge_platforms(existing: dict, version: str) -> dict[str, dict[str, str]]:
    """Carry forward the platform entries of the *same* version.

    A manifest describing a different version is discarded wholesale: its
    entries point at an older archive, and carrying them forward would
    let a client on a platform that was not rebuilt install a stale
    release.
    """
    platforms: dict[str, dict[str, str]] = {}
    if str(existing.get("version", "")) != version:
        return platforms

    existing_platforms = existing.get("platforms") or {}
    if not isinstance(existing_platforms, dict):
        fail("the existing manifest's `platforms` is not an object")
    for key, entry in existing_platforms.items():
        if not isinstance(entry, dict) or "url" not in entry or "signature" not in entry:
            fail(f"the existing manifest's platform '{key}' is malformed")
        platforms[key] = {"url": entry["url"], "signature": entry["signature"]}
    return platforms


def build_manifest(args: argparse.Namespace) -> dict:
    version = args.version
    validate_version(version)
    validate_pub_date(args.pub_date)
    validate_url(args.url)

    targets = [t.strip() for t in args.targets.split(",") if t.strip()]
    validate_targets(targets)

    signature = read_signature(args.signature_file)
    existing = load_existing(args.existing)
    platforms = merge_platforms(existing, version)

    for target in targets:
        previous = platforms.get(target)
        if previous is not None and previous["url"] != args.url and not args.repoint:
            fail(
                f"target '{target}' already points at {previous['url']} for version "
                f"{version}; refusing to repoint it at {args.url}"
            )
        platforms[target] = {"url": args.url, "signature": signature}

    return {
        "version": version,
        "pub_date": args.pub_date,
        "notes": args.notes,
        "platforms": platforms,
    }


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    try:
        manifest = build_manifest(args)
    except ContractError as exc:
        print(f"render-manifest: {exc}", file=sys.stderr)
        return 1

    rendered = json.dumps(manifest, indent=2, ensure_ascii=False) + "\n"
    if args.out:
        with open(args.out, "w", encoding="utf-8", newline="\n") as handle:
            handle.write(rendered)
    else:
        sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
