#!/usr/bin/env bash
# Publish every updater artefact in a directory to the R2 updater
# channel, in one pass.
#
# Usage:
#
#   ./scripts/release/publish-updater-assets.sh <channel> <version> <assets-dir>
#
# Example (from the release workflow, after downloading the signed
# release assets):
#
#   ./scripts/release/publish-updater-assets.sh stable 0.1.1 ./sidecars
#
# Each artefact is published under the manifest target keys that
# `tauri-plugin-updater` looks up:
#
#   {os}-{arch}-{bundle_type}   the artefact's own installer format
#   {os}-{arch}                 the platform's primary artefact, as a
#                               fallback for an unpackaged binary
#
# The mapping lives in `updater-targets.py` so that it can be asserted
# by `publish-update.test.sh` without any cloud access.
#
# Side effects: uploads each artefact + its `.sig` to
# `vault/<channel>/<version>/…` and rewrites
# `vault/<channel>/manifest.json` with every platform's entry.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

usage() {
  cat >&2 <<EOF
usage: $0 <channel> <version> <assets-dir>
  channel:    one of: stable | beta
  version:    semver, e.g. 0.1.1
  assets-dir: directory holding the downloaded release artefacts and
              their .sig files
EOF
  exit 64
}

[[ $# -eq 3 ]] || usage

CHANNEL="$1"
VERSION="$2"
ASSETS_DIR="$3"

if [[ "$CHANNEL" != "stable" && "$CHANNEL" != "beta" ]]; then
  echo "publish-updater-assets: channel must be stable or beta, got: $CHANNEL" >&2
  exit 1
fi

if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([+-][A-Za-z0-9.-]+)?$ ]]; then
  echo "publish-updater-assets: version must be semver, got: $VERSION" >&2
  exit 1
fi

if [[ ! -d "$ASSETS_DIR" ]]; then
  echo "publish-updater-assets: not a directory: $ASSETS_DIR" >&2
  exit 1
fi

# The updater artefacts are the archives the updater installs: a
# `.tar.gz` on Linux and macOS, a `.zip` on Windows. The installers
# themselves (`.AppImage`, `.deb`, `.rpm`, `.dmg`, `.msi`, `.exe`) are
# served from the GitHub release and are not updater artefacts.
ARTEFACTS=()
while IFS= read -r -d '' artefact; do
  ARTEFACTS+=("$artefact")
done < <(find "$ASSETS_DIR" -maxdepth 1 -type f \
  \( -name '*.tar.gz' -o -name '*.nsis.zip' -o -name '*.msi.zip' \) \
  ! -name '*.sig' -print0 | sort -z)

if [[ ${#ARTEFACTS[@]} -eq 0 ]]; then
  echo "::error::No updater artefacts found in ${ASSETS_DIR}." >&2
  echo "  Expected *.tar.gz (Linux/macOS) and/or *.nsis.zip / *.msi.zip (Windows)." >&2
  exit 1
fi

published=0
for artefact in "${ARTEFACTS[@]}"; do
  sig="${artefact}.sig"
  if [[ ! -s "$sig" ]]; then
    echo "::error::Missing signature for $(basename "$artefact") (expected ${sig})." >&2
    exit 1
  fi
  echo
  echo "→ publishing $(basename "$artefact")"
  "$SCRIPT_DIR/publish-update.sh" "$CHANNEL" "$VERSION" "$artefact" "$sig"
  published=$((published + 1))
done

echo
echo "── Done ──"
echo "  channel:  $CHANNEL"
echo "  version:  $VERSION"
echo "  artefacts: ${published}"
echo "  manifest: https://updates.functionfly.app/vault/${CHANNEL}/manifest.json"
