#!/usr/bin/env python3
"""Render the public version index for one updater channel.

WHY THIS FILE EXISTS
--------------------
The updater manifest (`vault/<channel>/manifest.json`) describes ONE
version: the newest. That is exactly what `tauri-plugin-updater` needs,
and exactly what a client wants — but it is not enough for a DOWNLOADS
PAGE.

A downloads page has to answer "what did you ship, and what is in each
build?". That means every published version, with its release notes and
the per-platform files it was published with.

The manifest cannot carry that: `merge_platforms()` deliberately DISCARDS
a previous version's platform entries when a new version is published,
because a client on a platform that was not rebuilt must not be offered
a stale archive. That is right for the plugin and wrong for a history
listing.

So the channel gets a SECOND document:

    vault/<channel>/versions.json

which accumulates. Every publish adds or updates one version entry and
leaves the older ones alone. The manifest stays exactly as the plugin
expects; the index is what the marketing site reads.

THE DOCUMENT
------------
    {
      "channel": "beta",
      "updated_at": "2026-11-04T12:00:00Z",
      "versions": [
        {
          "version": "0.1.0",
          "pub_date": "2026-11-04T04:35:39Z",
          "notes": "…the release's feature notes…",
          "platforms": {
            "linux-x86_64-appimage": { "url": "https://…", "signature": "<base64>" }
          }
        }
      ]
    }

Versions are ordered newest first. `notes` is the same release-notes
text the manifest carries, so the downloads page can show the features
of each version without inventing any.

Usage:

    render-versions.py --version 1.2.3 --pub-date 2026-11-03T12:00:00Z \\
        --notes "…" --url https://…/app.AppImage \\
        --signature-file app.AppImage.sig --targets linux-x86_64-appimage \\
        [--existing old-versions.json] [--out versions.json]

With no `--out`, the index is written to stdout.
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
from typing import Any, NoReturn

# The plugin's lookup keys. The index reuses them verbatim so a consumer
# can tell WHICH INSTALLER a platform entry describes (`appimage` vs
# `deb` vs `rpm` all share `linux-x86_64`).
TARGET_RE = re.compile(
    r"^(linux|darwin|windows)"
    r"-(x86_64|aarch64|i686|armv7|riscv64)"
    r"(-(appimage|deb|rpm|app|msi|nsis))?$"
)

SEMVER_RE = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+([+-][A-Za-z0-9.-]+)?$")

# How many versions the index keeps. A downloads page shows a history,
# not an archive; and the file is fetched by every visitor, so it must
# stay small. Older versions remain in the bucket and remain installable
# by pinning a manifest; they simply leave the list.
MAX_VERSIONS = 20


class ContractError(Exception):
    """A violation of the version-index contract."""


def fail(message: str) -> NoReturn:
    raise ContractError(message)


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="render-versions.py",
        description="Render the public version index for one updater channel.",
    )
    parser.add_argument("--version", required=True, help="semver version, e.g. 1.2.3")
    parser.add_argument("--pub-date", required=True, help="RFC 3339 publication timestamp")
    parser.add_argument("--notes", default="", help="release notes / feature summary")
    parser.add_argument("--url", required=True, help="absolute URL of the update artefact")
    parser.add_argument(
        "--signature-file",
        required=True,
        help="path to the .sig file whose CONTENT becomes the signature",
    )
    parser.add_argument(
        "--targets",
        required=True,
        help="comma-separated updater target keys, e.g. linux-x86_64-appimage",
    )
    parser.add_argument("--channel", default="", help="channel name recorded in the index")
    parser.add_argument(
        "--existing",
        default=None,
        help="path to the index currently published for this channel (optional)",
    )
    parser.add_argument("--out", default=None, help="write the index here (default: stdout)")
    return parser.parse_args(argv)


def validate_version(version: str) -> None:
    if not SEMVER_RE.match(version):
        fail(f"version '{version}' is not a plain semver string")
    if version.startswith("v"):
        fail(f"version '{version}' must not carry a 'v' prefix")


def validate_pub_date(pub_date: str) -> None:
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


def load_existing(path: str | None) -> dict[str, Any]:
    """Load the index currently published for the channel.

    A malformed index is a hard error, never a silent reset: discarding
    a published history because the file failed to parse would erase the
    very list the downloads page exists to show.
    """
    if not path or not os.path.exists(path):
        return {"versions": []}
    try:
        with open(path, "r", encoding="utf-8") as handle:
            existing = json.load(handle)
    except OSError as exc:
        fail(f"cannot read the existing index {path}: {exc}")
    except json.JSONDecodeError as exc:
        fail(f"the existing index {path} is not valid JSON: {exc}")
    if not isinstance(existing, dict):
        fail(f"the existing index {path} is not a JSON object")
    versions = existing.get("versions")
    if versions is None:
        versions = []
    if not isinstance(versions, list):
        fail("the existing index's `versions` is not an array")
    for entry in versions:
        if not isinstance(entry, dict):
            fail("the existing index has a malformed version entry")
        if not entry.get("version"):
            fail("the existing index has a version entry with no `version`")
    return {"versions": versions}


def build_index(args: argparse.Namespace) -> dict[str, Any]:
    version = args.version
    validate_version(version)
    validate_pub_date(args.pub_date)
    validate_url(args.url)

    targets = [t.strip() for t in args.targets.split(",") if t.strip()]
    validate_targets(targets)

    signature = read_signature(args.signature_file)
    existing = load_existing(args.existing)
    versions: list[dict[str, Any]] = list(existing["versions"])

    entry: dict[str, Any] = {
        "version": version,
        "pub_date": args.pub_date,
        "notes": args.notes,
        "platforms": {
            target: {"url": args.url, "signature": signature} for target in targets
        },
    }

    # A later publish for a version that is ALREADY in the index (the
    # macOS artefact of a release whose Linux artefact shipped first)
    # MERGES into that entry rather than adding a second one. The
    # manifest does the same, and for the same reason: one version, one
    # entry, every platform it was published with.
    merged = False
    for candidate in versions:
        if candidate.get("version") == version:
            platforms = candidate.setdefault("platforms", {})
            if not isinstance(platforms, dict):
                fail(f"the existing index's platforms for {version} is not an object")
            for target in targets:
                platforms[target] = {"url": args.url, "signature": signature}
            # The notes of a re-publish win: the newest text is the
            # accurate one, and the changelog may have been corrected.
            candidate["notes"] = args.notes
            candidate["pub_date"] = args.pub_date
            merged = True
            break

    if not merged:
        versions.insert(0, entry)

    # Newest first, and bounded. Ordering is by the semver-ish version
    # string descending so the list reads the way a changelog does.
    versions.sort(key=lambda v: _version_key(str(v.get("version", ""))), reverse=True)
    versions = versions[:MAX_VERSIONS]

    index: dict[str, Any] = {
        "channel": args.channel,
        "updated_at": args.pub_date,
        "versions": versions,
    }
    return index


def _version_key(version: str) -> tuple:
    """A sort key that orders numeric versions numerically.

    A plain string sort puts `0.10.0` before `0.9.0`, which is wrong.
    Non-numeric components sort last but do not crash.
    """
    parts: list[Any] = []
    for chunk in version.split("."):
        if chunk.isdigit():
            parts.append((1, int(chunk), ""))
        else:
            parts.append((0, 0, chunk))
    return tuple(parts)


def main(argv: list[str]) -> int:
    try:
        args = parse_args(argv)
        index = build_index(args)
    except ContractError as exc:
        print(f"render-versions: {exc}", file=sys.stderr)
        return 1

    rendered = json.dumps(index, indent=2, sort_keys=False) + "\n"

    if args.out:
        try:
            with open(args.out, "w", encoding="utf-8") as handle:
                handle.write(rendered)
        except OSError as exc:
            print(f"render-versions: cannot write {args.out}: {exc}", file=sys.stderr)
            return 1
    else:
        sys.stdout.write(rendered)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
