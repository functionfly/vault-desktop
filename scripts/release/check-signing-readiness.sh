#!/usr/bin/env bash
# check-signing-readiness.sh — reports whether this build can produce a
# code-signed, notarised artefact, and fails closed when signing is
# required but the material is missing.
#
# WHY THIS EXISTS
# ---------------
# Code signing is optional in the release pipeline today: the secrets
# table in `README.md` marks `APPLE_*` and `WINDOWS_CERTIFICATE*` as
# "not required". That is honest, but it means a release can be cut with
# no signing at all and nothing says so. The user then meets a Gatekeeper
# quarantine warning on macOS and a SmartScreen "Windows protected your
# PC" dialog on Windows — on a secrets manager, which is exactly where
# that warning does the most damage.
#
# This script makes the state of signing visible at the point where it
# matters: immediately before the release is built. It never signs
# anything itself; it only checks whether the inputs are present.
#
# MODES
# -----
#   (default)    Report the state. Exit 0 whether or not signing is
#                configured, so it can be informational.
#   --require    Exit non-zero when the platform's signing material is
#                missing. Use this when a signed artefact is a launch
#                requirement rather than a nice-to-have.
#
# USAGE
# -----
#   ./scripts/release/check-signing-readiness.sh [--platform macos|windows|linux] [--require]
#
# Exit codes:
#   0  signing is configured, or the platform does not need it
#   1  signing is required but the material is missing
#   2  configuration error

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

PLATFORM=""
REQUIRE=0

while [ $# -gt 0 ]; do
  case "$1" in
    --platform) PLATFORM="${2:-}"; shift 2 ;;
    --platform=*) PLATFORM="${1#*=}"; shift ;;
    --require) REQUIRE=1; shift ;;
    -h|--help)
      sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

if [ -z "$PLATFORM" ]; then
  case "$(uname -s)" in
    Darwin) PLATFORM="macos" ;;
    MINGW*|MSYS*|CYGWIN*) PLATFORM="windows" ;;
    *) PLATFORM="linux" ;;
  esac
fi

step() { printf "\n\033[1;34m▶ %s\033[0m\n" "$*"; }
ok()   { printf "  \033[1;32m✓\033[0m %s\n" "$*"; }
warn() { printf "  \033[1;33m!\033[0m %s\n" "$*"; }
bad()  { printf "  \033[1;31m✗\033[0m %s\n" "$*"; }
note() { printf "  · %s\n" "$*"; }

MISSING=()

require_env() {
  local name="$1" why="$2"
  if [ -n "${!name:-}" ]; then
    ok "${name} is set"
  else
    bad "${name} is not set — ${why}"
    MISSING+=("$name")
  fi
}

# ── macOS ─────────────────────────────────────────────────────────────────
check_macos() {
  step "macOS code signing + notarisation"

  require_env APPLE_CERTIFICATE "the Developer ID Application certificate (base64 .p12)"
  require_env APPLE_CERTIFICATE_PASSWORD "the certificate password"
  require_env APPLE_SIGNING_IDENTITY "the signing identity, e.g. 'Developer ID Application: FunctionFly (TEAMID)'"
  require_env APPLE_ID "the Apple ID used for notarisation"
  require_env APPLE_PASSWORD "the Apple-specific app password"
  require_env APPLE_TEAM_ID "the Apple Developer Team ID"

  # The hardened runtime needs entitlements, and Apple rejects a
  # notarisation upload whose binary lacks the JIT entitlement.
  step "macOS entitlements"
  ENTITLEMENTS="$(python3 - <<'PY'
import json, sys
try:
    conf = json.load(open("src-tauri/tauri.conf.json"))
except Exception:
    print("")
    raise SystemExit(0)
print((conf.get("bundle", {}).get("macOS", {}) or {}).get("entitlements", ""))
PY
)"

  if [ -z "$ENTITLEMENTS" ]; then
    bad "tauri.conf.json does not set bundle.macOS.entitlements"
    MISSING+=("bundle.macOS.entitlements")
  else
    ENT_PATH="src-tauri/${ENTITLEMENTS}"
    if [ ! -f "$ENT_PATH" ]; then
      bad "the entitlements file '${ENT_PATH}' does not exist"
      MISSING+=("$ENT_PATH")
    else
      ok "the entitlements file '${ENT_PATH}' exists"
      # Parse it and check the hardened-runtime requirements are present.
      if ! python3 - "$ENT_PATH" <<'PY'
import plistlib, sys
with open(sys.argv[1], "rb") as fh:
    plist = plistlib.load(fh)
required = [
    "com.apple.security.cs.allow-jit",
    "com.apple.security.cs.allow-unsigned-executable-memory",
    "com.apple.security.cs.disable-library-validation",
]
missing = [k for k in required if not plist.get(k)]
if missing:
    sys.stderr.write("missing entitlements: " + ", ".join(missing) + "\n")
    raise SystemExit(1)
PY
      then
        bad "the entitlements file is missing a hardened-runtime capability"
        MISSING+=("entitlements contents")
      else
        ok "the entitlements carry the hardened-runtime capabilities"
      fi
    fi
  fi

  note "a macOS release without signing is quarantined by Gatekeeper and"
  note "cannot be notarised, so it never gets a clean first-run experience."
}

# ── Windows ───────────────────────────────────────────────────────────────
check_windows() {
  step "Windows code signing"

  require_env WINDOWS_CERTIFICATE "the signing certificate (.pfx)"
  require_env WINDOWS_CERTIFICATE_PASSWORD "the certificate password"

  step "Windows signing configuration"
  if python3 - <<'PY'
import json, sys
conf = json.load(open("src-tauri/tauri.conf.json"))
win = conf.get("bundle", {}).get("windows", {}) or {}
sys.exit(0 if win.get("digestAlgorithm") == "sha256" else 1)
PY
  then
    ok "bundle.windows.digestAlgorithm is sha256"
  else
    bad "bundle.windows.digestAlgorithm is not sha256"
    MISSING+=("bundle.windows.digestAlgorithm")
  fi

  if python3 - <<'PY'
import json, sys
conf = json.load(open("src-tauri/tauri.conf.json"))
win = conf.get("bundle", {}).get("windows", {}) or {}
sys.exit(0 if win.get("timestampUrl") else 1)
PY
  then
    ok "bundle.windows.timestampUrl is set (the signature survives certificate expiry)"
  else
    bad "bundle.windows.timestampUrl is not set"
    MISSING+=("bundle.windows.timestampUrl")
  fi

  note "an unsigned Windows release triggers the SmartScreen 'unknown publisher'"
  note "dialog. That warning is far more damaging for a secrets manager than"
  note "for an ordinary application."
}

# ── Linux ─────────────────────────────────────────────────────────────────
check_linux() {
  step "Linux packaging"
  ok "Linux packages are not code-signed; AppImage/deb carry no signature"
  note "Distro repositories apply their own signing, and the AppImage is"
  note "distributed from the release page rather than a repo."
}

step "signing readiness — platform: ${PLATFORM}"
case "$PLATFORM" in
  macos)   check_macos ;;
  windows) check_windows ;;
  linux)   check_linux ;;
  *) echo "unknown platform: $PLATFORM" >&2; exit 2 ;;
esac

step "summary"
if [ "${#MISSING[@]}" -eq 0 ]; then
  printf "  platform   %s\n" "$PLATFORM"
  printf "  result     \033[1;32msigning material is present\033[0m\n"
  exit 0
fi

printf "  platform   %s\n" "$PLATFORM"
printf "  missing    %s\n" "${MISSING[*]}"
printf "  result     \033[1;33msigning is NOT configured\033[0m\n"
echo
echo "A release built without this material will:"
echo "  - macOS:   be quarantined by Gatekeeper and cannot be notarised."
echo "  - Windows: show the SmartScreen 'unknown publisher' dialog."
echo
echo "See the 'Secrets the release needs' table in README.md for how to"
echo "provision each value."

if [ "$REQUIRE" -eq 1 ]; then
  exit 1
fi
exit 0
