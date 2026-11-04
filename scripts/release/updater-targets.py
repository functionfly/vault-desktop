#!/usr/bin/env python3
"""Map a Tauri updater artefact filename to the manifest target keys.

`tauri-plugin-updater` looks up the `platforms` map of the update
manifest with two candidate keys, in order:

    {os}-{arch}-{bundle_type}
    {os}-{arch}

where `os` is `linux` | `darwin` | `windows`, `arch` is `x86_64` |
`aarch64` | `i686` | `armv7` | `riscv64`, and `bundle_type` is the
installer the user actually installed (`appimage`, `deb`, `rpm`, `app`,
`msi`, `nsis`). A Rust target triple such as `x86_64-unknown-linux-gnu`
is never looked up.

Because the bundle type is part of the key, one manifest entry per
artefact is required: a `.deb` user must be offered the `.deb` archive,
not the AppImage that happens to share its architecture. The second,
bundle-type-less key is published alongside the platform's *primary*
artefact as a fallback for users running an unpackaged binary, where
`bundle_type()` resolves to `None`.

Usage:

    updater-targets.py functionfly-vault-desktop_0.1.0_amd64.AppImage
    # -> linux-x86_64-appimage,linux-x86_64

Exits 1 with an explanation when the filename cannot be mapped, so a
release fails loudly instead of publishing an entry no client can find.
"""

from __future__ import annotations

import os
import re
import sys

# Artefact suffix -> (os, bundle type).
#
# THESE ARE THE NAMES THE TAURI v2 BUNDLER ACTUALLY PRODUCES. Getting
# them wrong is not a theoretical problem: the first version of this
# table used the Tauri *v1* names (`.appimage.tar.gz`, `.deb.tar.gz`,
# `.rpm.tar.gz`), which the v2 bundler never produces. Every real Linux
# artefact was therefore rejected with "not a recognised updater
# artefact", and the Linux updater channel could never have been
# published.
#
# From `tauri-bundler/src/bundle.rs`:
#
#   // Targets the v2 updater plugin installs directly, no wrapping needed.
#   let has_self_contained_target = package_types.iter().any(|package_type| {
#     matches!(package_type,
#       PackageType::AppImage | PackageType::Nsis | PackageType::WindowsMsi
#       | PackageType::Deb | PackageType::Rpm)
#   });
#
#   // the macOS app bundle is always archived, the other v1 targets only
#   // for the legacy updater
#   if package_types.contains(&PackageType::MacOsBundle) || has_v1_target {
#
# So the AppImage, deb and rpm are signed in place; the macOS .app is
# always archived; the Windows installers are zipped. The tar.gz form
# only exists for the legacy v1 updater, which this project does not
# use.
BUNDLES: list[tuple[str, str, str]] = [
    (".appimage", "linux", "appimage"),
    (".deb", "linux", "deb"),
    (".rpm", "linux", "rpm"),
    (".app.tar.gz", "darwin", "app"),
    (".msi.zip", "windows", "msi"),
    (".nsis.zip", "windows", "nsis"),
]

# The bundle type whose artefact also owns the generic `{os}-{arch}`
# fallback key for its platform.
PRIMARY_BUNDLE = {
    "linux": "appimage",
    "darwin": "app",
    "windows": "nsis",
}

ARCH_TOKENS: list[tuple[str, str]] = [
    ("aarch64", "aarch64"),
    ("arm64", "aarch64"),
    ("x86_64", "x86_64"),
    ("amd64", "x86_64"),
    ("x64", "x86_64"),
    ("i686", "i686"),
    ("i386", "i686"),
]


def arch_tokens(name: str) -> list[str]:
    """Every architecture the artefact can serve, most specific first."""
    if "universal" in name:
        return ["aarch64", "x86_64"]
    found = [arch for token, arch in ARCH_TOKENS if token in name]
    # Preserve order but drop duplicates (e.g. "x86_64" and "x64" both
    # present in a Windows name).
    seen: list[str] = []
    for arch in found:
        if arch not in seen:
            seen.append(arch)
    return seen


def targets_for(filename: str) -> list[str]:
    """The manifest target keys this artefact should be published under."""
    name = os.path.basename(filename).lower()

    matched = None
    for suffix, os_name, bundle in BUNDLES:
        if name.endswith(suffix):
            matched = (os_name, bundle)
            break
    if matched is None:
        raise ValueError(
            f"'{filename}' is not a recognised updater artefact; expected one of "
            + ", ".join(suffix for suffix, _, _ in BUNDLES)
        )

    os_name, bundle = matched
    arches = arch_tokens(name)
    if not arches:
        raise ValueError(
            f"could not determine the architecture of '{filename}'; the name must "
            "contain one of: " + ", ".join(token for token, _ in ARCH_TOKENS)
        )

    keys: list[str] = []
    for arch in arches:
        keys.append(f"{os_name}-{arch}-{bundle}")
        if bundle == PRIMARY_BUNDLE[os_name]:
            # The bundle-type-less fallback, for an unpackaged binary.
            keys.append(f"{os_name}-{arch}")
    return keys


def main(argv: list[str]) -> int:
    if len(argv) != 1:
        print("usage: updater-targets.py <artefact-filename>", file=sys.stderr)
        return 64
    try:
        keys = targets_for(argv[0])
    except ValueError as exc:
        print(f"updater-targets: {exc}", file=sys.stderr)
        return 1
    print(",".join(keys))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
