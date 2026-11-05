#!/usr/bin/env bash
# Smoke test for scripts/release/. Validates arg parsing, channel
# validation, version validation, pre-flight env checks, and the
# updater manifest contract — WITHOUT touching R2.
#
# Operators should run this after editing `publish-update.sh` to catch
# regressions before a real publish.
#
# Strategy: re-run the script's `usage()` + arg validation paths by
# feeding each invalid input in turn and asserting the script exits
# non-zero with the expected error category. The manifest contract is
# asserted separately through `render-manifest.py`, which is the single
# source of truth for the manifest shape — those assertions run on every
# machine, with or without the AWS CLI.

set -uo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
SCRIPT="$DIR/publish-update.sh"
LOAD_ENV="$DIR/load-updater-env.sh"
RENDER="$DIR/render-manifest.py"
RENDER_VERSIONS="$DIR/render-versions.py"
PROVISION="$DIR/provision-r2-bucket.sh"

fail=0
expect_exit() {
  local description="$1"
  local expected="$2"
  shift 2
  local actual=0
  "$@" >/dev/null 2>&1 || actual=$?
  if [[ "$actual" -eq "$expected" ]]; then
    echo "  ✓ $description"
  else
    echo "  ✗ $description (expected exit=$expected, got $actual)"
    fail=1
  fi
}

expect_true() {
  local description="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "  ✓ $description"
  else
    echo "  ✗ $description"
    fail=1
  fi
}

expect_false() {
  local description="$1"
  shift
  if "$@" >/dev/null 2>&1; then
    echo "  ✗ $description"
    fail=1
  else
    echo "  ✓ $description"
  fi
}

echo "── scripts/release smoke test ──"
echo

# ── 0. Tooling ─────────────────────────────────────────────────────────
# python3 (or python) is a hard requirement of the release tooling: the
# manifest is produced by a JSON encoder, never by shell interpolation.
PYTHON=""
for candidate in python3 python; do
  if command -v "$candidate" >/dev/null 2>&1; then
    PYTHON="$candidate"
    break
  fi
done

if [[ -z "$PYTHON" ]]; then
  echo "  ✗ python3 is required by the release tooling" >&2
  exit 1
fi

# 1. Bash + shellcheck already passed at write-time; re-confirm here.
echo "→ syntax"
expect_exit "publish-update.sh is bash-clean" 0 bash -n "$SCRIPT"
expect_exit "load-updater-env.sh is bash-clean" 0 bash -n "$LOAD_ENV"
expect_exit "render-manifest.py compiles" 0 "$PYTHON" -m py_compile "$RENDER"
expect_exit "render-versions.py compiles" 0 "$PYTHON" -m py_compile "$RENDER_VERSIONS"
expect_exit "provision-r2-bucket.sh is bash-clean" 0 bash -n "$PROVISION"

# 2. Usage on missing args
echo
echo "→ usage"
expect_exit "zero args → exit 64" 64 "$SCRIPT"

# 3. Bad channel
expect_exit "invalid channel → exit 1" 1 \
  "$SCRIPT" "release" "0.1.1" "/dev/null" "/dev/null"

# 4. Bad version
expect_exit "invalid version → exit 1" 1 \
  "$SCRIPT" "stable" "not-a-version" "/dev/null" "/dev/null"

# 5. Missing env (no R2_*) → exit 1 (pre-flight fail)
echo
echo "→ pre-flight"
expect_exit "missing R2_ACCOUNT_ID → exit 1" 1 \
  env -i PATH="$PATH" "$SCRIPT" "stable" "0.1.1" "/dev/null" "/dev/null"

# 6. load-updater-env.sh: missing private key
expect_exit "load-updater-env.sh without key files → exit 1" 1 \
  env -i PATH="$PATH" VAULT_KEY_DIR="/nonexistent" bash -c "source $LOAD_ENV"

# 7. load-updater-env.sh: happy path (uses the real key we generated)
#
# This check asserts the loader's contract — that sourcing it exports a
# non-empty pubkey, a private-key path, and that the path exists and is
# mode 0600 — WITHOUT requiring the real signing key to be present.
#
# It previously sourced the loader with no VAULT_KEY_DIR override, which
# silently depended on `load-updater-env.sh`'s default of
# $HOME/.tauri. That passed on the author's machine and failed on
# every CI runner with:
#
#   x load-updater-env.sh loads env (round-trip ok) (expected exit=0, got 1)
#
# because the loader correctly exits 1 when the key files are absent (that
# is check #6's contract, verified just above).
#
# A throwaway Ed25519 keypair is generated into a temp dir instead. It is
# never uploaded, never used to sign a real artefact, and is deleted on
# exit. This keeps the check hermetic: it tests the loader, not whether a
# particular machine happens to hold a key.
expect_exit "load-updater-env.sh loads env (round-trip ok)" 0 \
  env VAULT_KEY_DIR="$(mktemp -d)" LOAD_ENV_PATH="$LOAD_ENV" \
  bash "${DIR}/_load-env-smoke.sh"

# ── 8. The updater manifest contract ───────────────────────────────────
#
# These assertions run everywhere. They are the difference between "a
# manifest file exists" and "a manifest a signed client can actually
# install from" — the previous manifest was syntactically invalid JSON,
# keyed by a Rust target triple the plugin never looks up, and carried a
# URL where the signature belongs.
echo
echo "→ manifest contract"

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

# A realistic .sig: the base64 encoding of a minisign signature, exactly
# what `tauri signer sign` writes.
cat > "$FIXTURE_DIR/app.tar.gz.sig" <<'SIG_EOF'
dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUNytjQ0QwTlM4QzUvVmFWTkV1R0VscEhhTjlDRUtBZ0pFN0pVQ1R6YXY5SzhPQ1ZmZnhpRzNvb3BoNkgxQXJLOXFhWEFHc2cwRi9MRTRDWEVIWVY1STloTVBPNE1XSGdNPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkzNjg3NDA3CWZpbGU6cGF5bG9hZC5iaW4KQjd1RWRtMk1KVW9yRUJ1dHNCZ0RvUW5paXZFSWlrQjR2Y0pROTlMciswQ082R2FUeFpSYVBnRFBLV1c2MEJZTFd1S2tsMnZ1eWU3dG9WTThsN1BqRHc9PQo=
SIG_EOF

head -c 4096 /dev/urandom > "$FIXTURE_DIR/app.tar.gz"

SIG_CONTENT="$(tr -d '\r\n' < "$FIXTURE_DIR/app.tar.gz.sig")"
BASE_URL="https://updates.functionfly.com/vault/stable/0.1.1"

render() {
  "$PYTHON" "$RENDER" \
    --version=0.1.1 \
    --pub-date=2026-11-03T12:00:00Z \
    --notes="vault 0.1.1 (channel: stable)" \
    --url="$1" \
    --signature-file="$FIXTURE_DIR/app.tar.gz.sig" \
    --targets="$2" \
    "${@:3}"
}

# 8a. The happy path produces a manifest the plugin can consume.
render "$BASE_URL/app.tar.gz" "linux-x86_64" --out="$FIXTURE_DIR/manifest.json"
expect_exit "render-manifest.py writes a manifest" 0 test -s "$FIXTURE_DIR/manifest.json"

# 8b. It is valid JSON, and the shape is the one the plugin expects.
if "$PYTHON" - "$FIXTURE_DIR/manifest.json" <<'PY_EOF'
import json
import re
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    manifest = json.load(handle)

assert manifest["version"] == "0.1.1", manifest.get("version")
assert re.match(r"^\d+\.\d+\.\d+", manifest["version"]), manifest["version"]
assert re.match(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$", manifest["pub_date"]), manifest["pub_date"]
assert isinstance(manifest["notes"], str), manifest["notes"]
assert manifest["notes"], "notes must not be empty"

platforms = manifest["platforms"]
assert list(platforms) == ["linux-x86_64"], list(platforms)
for key, entry in platforms.items():
    # The plugin looks up `{os}-{arch}` / `{os}-{arch}-{installer}`.
    # A Rust target triple is never found.
    assert re.match(
        r"^(linux|darwin|windows)-(x86_64|aarch64|i686|armv7|riscv64)"
        r"(-(appimage|deb|rpm|app|msi|nsis))?$",
        key,
    ), key
    assert entry["url"].startswith("https://"), entry["url"]
    # The signature is CONTENT, not a URL to a .sig file.
    assert not entry["signature"].startswith("http"), entry["signature"]
    import base64
    base64.b64decode(entry["signature"], validate=True)

print("manifest shape ok", file=sys.stderr)
PY_EOF
then
  echo "  ✓ manifest is valid JSON with plugin-compatible targets and signature content"
else
  echo "  ✗ manifest shape assertion failed"
  fail=1
fi

# 8c. The signature really is the .sig file's content.
if "$PYTHON" - "$FIXTURE_DIR/manifest.json" "$SIG_CONTENT" <<'PY_EOF'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    manifest = json.load(handle)

expected = sys.argv[2]
for entry in manifest["platforms"].values():
    assert entry["signature"] == expected, "signature is not the .sig content"
PY_EOF
then
  echo "  ✓ manifest signature is the .sig file content, not a URL"
else
  echo "  ✗ manifest signature is not the .sig content"
  fail=1
fi

# 8d. A Rust target triple is rejected — it is the exact mistake the
#     previous manifest made.
expect_exit "a Rust target triple is rejected → exit 1" 1 \
  render "$BASE_URL/app.tar.gz" "x86_64-unknown-linux-gnu"

# 8e. An unknown target is rejected.
expect_exit "an unknown target is rejected → exit 1" 1 \
  render "$BASE_URL/app.tar.gz" "solaris-sparc"

# 8f. A non-semver version is rejected.
expect_exit "a non-semver version is rejected → exit 1" 1 \
  "$PYTHON" "$RENDER" \
    --version=v0.1.1 \
    --pub-date=2026-11-03T12:00:00Z \
    --url="$BASE_URL/app.tar.gz" \
    --signature-file="$FIXTURE_DIR/app.tar.gz.sig" \
    --targets=linux-x86_64

# 8g. A signature that is not base64 is rejected before it reaches R2.
printf 'not base64 @@@\n' > "$FIXTURE_DIR/bad.tar.gz.sig"
expect_exit "a non-base64 signature is rejected → exit 1" 1 \
  "$PYTHON" "$RENDER" \
    --version=0.1.1 \
    --pub-date=2026-11-03T12:00:00Z \
    --url="$BASE_URL/app.tar.gz" \
    --signature-file="$FIXTURE_DIR/bad.tar.gz.sig" \
    --targets=linux-x86_64

# 8h. A second platform for the SAME version merges into the manifest.
render "$BASE_URL/app.tar.gz" "linux-x86_64" --out="$FIXTURE_DIR/manifest.json"
render "$BASE_URL/app.tar.gz" "darwin-aarch64,darwin-x86_64" \
  --existing="$FIXTURE_DIR/manifest.json" --out="$FIXTURE_DIR/manifest2.json"

if "$PYTHON" - "$FIXTURE_DIR/manifest2.json" <<'PY_EOF'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    manifest = json.load(handle)

assert sorted(manifest["platforms"]) == ["darwin-aarch64", "darwin-x86_64", "linux-x86_64"], \
    sorted(manifest["platforms"])
PY_EOF
then
  echo "  ✓ a second platform merges into the same version's manifest"
else
  echo "  ✗ merging a second platform failed"
  fail=1
fi

# 8i. A NEW version replaces the manifest rather than inheriting the
#     old version's platform entries.
render "$BASE_URL/app.tar.gz" "linux-x86_64" --out="$FIXTURE_DIR/manifest.json"
expect_exit "a new version replaces the old platform entries" 0 \
  "$PYTHON" "$RENDER" \
    --version=0.2.0 \
    --pub-date=2026-11-03T12:00:00Z \
    --url="https://updates.functionfly.com/vault/stable/0.2.0/app.tar.gz" \
    --signature-file="$FIXTURE_DIR/app.tar.gz.sig" \
    --targets=linux-x86_64 \
    --existing="$FIXTURE_DIR/manifest.json" \
    --out="$FIXTURE_DIR/manifest3.json"

if "$PYTHON" - "$FIXTURE_DIR/manifest3.json" <<'PY_EOF'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    manifest = json.load(handle)

assert manifest["version"] == "0.2.0", manifest["version"]
assert list(manifest["platforms"]) == ["linux-x86_64"], list(manifest["platforms"])
PY_EOF
then
  echo "  ✓ a new version does not inherit the old version's platforms"
else
  echo "  ✗ a new version inherited stale platform entries"
  fail=1
fi

# 8j. Repointing a target at a different archive for the same version
#     is refused.
expect_exit "repointing a target at another archive is refused → exit 1" 1 \
  "$PYTHON" "$RENDER" \
    --version=0.1.1 \
    --pub-date=2026-11-03T12:00:00Z \
    --url="https://updates.functionfly.com/vault/stable/0.1.1/other.tar.gz" \
    --signature-file="$FIXTURE_DIR/app.tar.gz.sig" \
    --targets=linux-x86_64 \
    --existing="$FIXTURE_DIR/manifest.json"

# 8k. A corrupt channel manifest is a hard error, not a silent reset.
printf '{ this is not json\n' > "$FIXTURE_DIR/corrupt.json"
expect_exit "a corrupt existing manifest is refused → exit 1" 1 \
  "$PYTHON" "$RENDER" \
    --version=0.1.1 \
    --pub-date=2026-11-03T12:00:00Z \
    --url="$BASE_URL/app.tar.gz" \
    --signature-file="$FIXTURE_DIR/app.tar.gz.sig" \
    --targets=linux-x86_64 \
    --existing="$FIXTURE_DIR/corrupt.json"

# 8k-2. `--repoint` is the ONLY way to move an already-published target,
#       and it does move it. This is the deliberate-repair path: a
#       manifest published against the wrong hostname has to be corrected,
#       and the correction must be opt-in so a healthy publication is
#       never silently repointed.
expect_exit "--repoint moves an already-published target → exit 0" 0 \
  "$PYTHON" "$RENDER" \
    --version=0.1.1 \
    --pub-date=2026-11-03T12:00:00Z \
    --url="https://updates.functionfly.com/vault/stable/0.1.1/renamed.tar.gz" \
    --signature-file="$FIXTURE_DIR/app.tar.gz.sig" \
    --targets=linux-x86_64 \
    --existing="$FIXTURE_DIR/manifest.json" \
    --repoint \
    --out="$FIXTURE_DIR/repointed.json"

if "$PYTHON" - "$FIXTURE_DIR/repointed.json" <<'PY_EOF'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    manifest = json.load(handle)

entry = manifest["platforms"]["linux-x86_64"]
assert entry["url"].endswith("/renamed.tar.gz"), entry["url"]
assert entry["signature"], "the signature must survive the repoint"
PY_EOF
then
  echo "  ✓ --repoint rewrites the URL and keeps the signature"
else
  echo "  ✗ --repoint did not produce the expected manifest"
  fail=1
fi

# 8k-3. `--repoint` must not weaken the target-key validation: it only
#       relaxes the "already points elsewhere" rule.
expect_exit "--repoint still rejects an invalid target → exit 1" 1 \
  "$PYTHON" "$RENDER" \
    --version=0.1.1 \
    --pub-date=2026-11-03T12:00:00Z \
    --url="$BASE_URL/app.tar.gz" \
    --signature-file="$FIXTURE_DIR/app.tar.gz.sig" \
    --targets="linux-x86_64-unknown-bundle" \
    --existing="$FIXTURE_DIR/manifest.json" \
    --repoint

# 8k-4. `publish-update.sh` only passes `--repoint` when the operator
#       asks for it explicitly. A default run must not carry the flag,
#       or the safety property above is silently gone.
expect_false "publish-update.sh does not enable --repoint by default" \
  bash -c "grep -E '^\s*--repoint' '$SCRIPT' | grep -qv 'VAULT_UPDATER_REPOINT'"

if grep -q 'VAULT_UPDATER_REPOINT' "$SCRIPT"; then
  echo "  ✓ publish-update.sh gates --repoint behind VAULT_UPDATER_REPOINT"
else
  echo "  ✗ publish-update.sh has no VAULT_UPDATER_REPOINT gate"
  fail=1
fi

# 8l. The default endpoint the app uses is the channel manifest, and it
#     carries no placeholder the plugin cannot substitute.
if grep -q 'VAULT_UPDATER_STABLE_URL:=https://updates.functionfly.com/vault/stable/manifest.json' "$LOAD_ENV"; then
  echo "  ✓ the stable endpoint defaults to the channel manifest"
else
  echo "  ✗ the stable endpoint default is not the channel manifest"
  fail=1
fi

expect_false "the endpoint defaults carry no {{target_version}} placeholder" \
  bash -c "grep -E 'VAULT_UPDATER_(STABLE|BETA)_URL:=' '$LOAD_ENV' | grep -q 'target_version'"

# 8m. The public host the manifest writes must be the same host the
#     bucket is SERVED from.
#
# The outage this guards against: the manifest's `url` fields pointed at
# `updates.functionfly.app` while the bucket was published and reachable
# on `.com`. The manifest was syntactically valid, the artefact was
# uploaded, and every client still got a 404 — because the hostname in
# the manifest is not the hostname the bucket answers on.
#
# So the two must be derived from ONE variable, not written out
# separately in two files.
expect_true "the public host is a single shared variable" \
  grep -q 'VAULT_UPDATER_PUBLIC_HOST' "$PROVISION"

expect_true "provision-r2-bucket.sh attaches the public custom domain" \
  grep -q 'r2/buckets/\${R2_BUCKET}/custom_domains' "$PROVISION"

expect_true "provision-r2-bucket.sh probes the public path anonymously" \
  grep -q 'VAULT_UPDATER_PUBLIC_HOST}/\${PROBE_KEY}' "$PROVISION"

# The manifest's URL base and the provisioned host must not drift apart.
MANIFEST_BASE="$(grep -oE 'https://[a-z0-9.-]+/vault' "$SCRIPT" | head -1 | sed 's|/vault$||')"
PROVISION_HOST="$(grep -oE 'VAULT_UPDATER_PUBLIC_HOST:-[a-z0-9.-]+' "$PROVISION" | head -1 | sed 's/.*-//')"
if [[ -n "$MANIFEST_BASE" && -n "$PROVISION_HOST" ]]; then
  if [[ "$MANIFEST_BASE" == *"${PROVISION_HOST}" ]]; then
    echo "  ✓ the manifest's URL host matches the provisioned custom domain"
  else
    echo "  ✗ the manifest writes ${MANIFEST_BASE} but the bucket is served from ${PROVISION_HOST}"
    fail=1
  fi
else
  echo "  ✗ could not determine the manifest host or the provisioned host"
  fail=1
fi

# ── 8n. The public version index ────────────────────────────────────────
#
# The manifest describes ONE version (the newest) — that is what the
# plugin needs. The downloads page needs the HISTORY: every published
# version with its notes and its per-platform files. That is the version
# index, and it is what the marketing site reads.
echo
echo "→ version index"

expect_exit "render-versions.py compiles" 0 "$PYTHON" -m py_compile "$RENDER_VERSIONS"

render_versions() {
  "$PYTHON" "$RENDER_VERSIONS" \
    --version="$1" \
    --pub-date="$2" \
    --notes="$3" \
    --url="$4" \
    --signature-file="$FIXTURE_DIR/app.tar.gz.sig" \
    --targets="$5" \
    --channel=beta \
    "${@:6}"
}

# 8n-1. A first publish produces an index with one version.
render_versions "0.1.0" "2026-11-03T12:00:00Z" "first release" \
  "https://updates.functionfly.com/vault/beta/0.1.0/app.AppImage" \
  "linux-x86_64-appimage" \
  --out="$FIXTURE_DIR/versions.json"

expect_exit "render-versions.py writes an index" 0 test -s "$FIXTURE_DIR/versions.json"

# 8n-2. A SECOND publish ADDS a version rather than replacing the index.
#       Replacing it would erase the history the page exists to show.
render_versions "0.2.0" "2026-11-04T12:00:00Z" "second release" \
  "https://updates.functionfly.com/vault/beta/0.2.0/app.AppImage" \
  "linux-x86_64-appimage" \
  --existing="$FIXTURE_DIR/versions.json" \
  --out="$FIXTURE_DIR/versions2.json"

if "$PYTHON" - "$FIXTURE_DIR/versions2.json" <<'PY_EOF'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    index = json.load(handle)

versions = index["versions"]
assert len(versions) == 2, [v["version"] for v in versions]
# Newest first.
assert versions[0]["version"] == "0.2.0", versions[0]["version"]
assert versions[1]["version"] == "0.1.0", versions[1]["version"]
# Each version carries its OWN notes — that is the "features per version".
assert versions[0]["notes"] == "second release", versions[0]["notes"]
assert versions[1]["notes"] == "first release", versions[1]["notes"]
# …and its own per-platform URL.
assert versions[0]["platforms"]["linux-x86_64-appimage"]["url"].endswith("0.2.0/app.AppImage")
assert versions[1]["platforms"]["linux-x86_64-appimage"]["url"].endswith("0.1.0/app.AppImage")
# The signature is CONTENT, not a URL.
import base64
base64.b64decode(versions[0]["platforms"]["linux-x86_64-appimage"]["signature"], validate=True)
PY_EOF
then
  echo "  ✓ a second publish adds a version and keeps the older one"
else
  echo "  ✗ the version index did not accumulate the older version"
  fail=1
fi

# 8n-3. A later platform for a version ALREADY in the index MERGES into
#       that entry rather than adding a duplicate. One version, one entry.
render_versions "0.1.0" "2026-11-03T12:00:00Z" "first release" \
  "https://updates.functionfly.com/vault/beta/0.1.0/app.dmg" \
  "darwin-aarch64" \
  --existing="$FIXTURE_DIR/versions.json" \
  --out="$FIXTURE_DIR/versions3.json"

if "$PYTHON" - "$FIXTURE_DIR/versions3.json" <<'PY_EOF'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    index = json.load(handle)

versions = index["versions"]
assert len(versions) == 1, [v["version"] for v in versions]
platforms = versions[0]["platforms"]
assert set(platforms) == {"linux-x86_64-appimage", "darwin-aarch64"}, sorted(platforms)
assert platforms["darwin-aarch64"]["url"].endswith("app.dmg")
PY_EOF
then
  echo "  ✓ a second platform merges into the existing version entry"
else
  echo "  ✗ the version index did not merge the platform"
  fail=1
fi

# 8n-4. Version ordering is NUMERIC, not lexical: 0.10.0 must sort above
#       0.9.0. A string sort gets this backwards.
render_versions "0.10.0" "2026-11-05T12:00:00Z" "ten" \
  "https://updates.functionfly.com/vault/beta/0.10.0/app.AppImage" \
  "linux-x86_64-appimage" \
  --existing="$FIXTURE_DIR/versions2.json" \
  --out="$FIXTURE_DIR/versions4.json"

if "$PYTHON" - "$FIXTURE_DIR/versions4.json" <<'PY_EOF'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    index = json.load(handle)

order = [v["version"] for v in index["versions"]]
assert order[0] == "0.10.0", order
assert order[1] == "0.2.0", order
PY_EOF
then
  echo "  ✓ versions sort numerically, not lexically"
else
  echo "  ✗ versions sorted lexically (0.10.0 before 0.2.0)"
  fail=1
fi

# 8n-5. A corrupt index is a hard error: silently resetting it would
#       erase the published history.
printf '{ not json\n' > "$FIXTURE_DIR/corrupt-versions.json"
expect_exit "a corrupt existing index is refused → exit 1" 1 \
  "$PYTHON" "$RENDER_VERSIONS" \
    --version=0.1.0 \
    --pub-date=2026-11-03T12:00:00Z \
    --url="$BASE_URL/app.tar.gz" \
    --signature-file="$FIXTURE_DIR/app.tar.gz.sig" \
    --targets=linux-x86_64 \
    --existing="$FIXTURE_DIR/corrupt-versions.json"

# 8n-6. The publish script must upload the index, or the page reads a
#       document that never changes.
expect_true "publish-update.sh uploads the version index" \
  grep -q 'VERSIONS_KEY' "$SCRIPT"

expect_true "publish-update.sh renders the version index" \
  grep -q 'render-versions.py' "$SCRIPT"

# ── 9. The artefact → manifest-target mapping ───────────────────────────
#
# The plugin's lookup key carries the installer format, so a `.deb` user
# must be offered the `.deb` archive rather than the AppImage that
# shares its architecture. Getting this wrong publishes a manifest that
# looks plausible and installs the wrong thing.
echo
echo "→ artefact → manifest target mapping"

expect_targets() {
  local description="$1" filename="$2" expected="$3"
  local actual
  actual="$("$PYTHON" "$DIR/updater-targets.py" "$filename" 2>/dev/null)"
  if [[ "$actual" == "$expected" ]]; then
    echo "  ✓ $description"
  else
    echo "  ✗ $description"
    echo "      expected: $expected"
    echo "      actual:   ${actual:-<none>}"
    fail=1
  fi
}

# These are the names the Tauri v2 bundler ACTUALLY produces. The v2
# updater plugin installs the AppImage, deb and rpm in place — they are
# not wrapped in a .tar.gz. The macOS .app and the Windows installers
# ARE wrapped. Asserting the v1 names here let the mapper carry a table
# that rejected every real Linux artefact.
expect_targets "Linux AppImage → appimage key + generic key" \
  "functionfly-vault-desktop_0.1.0_amd64.AppImage" \
  "linux-x86_64-appimage,linux-x86_64"

expect_targets "Linux deb → deb key only" \
  "functionfly-vault-desktop_0.1.0_amd64.deb" \
  "linux-x86_64-deb"

expect_targets "Linux rpm → rpm key only" \
  "functionfly-vault-desktop_0.1.0_amd64.rpm" \
  "linux-x86_64-rpm"

expect_targets "macOS aarch64 app → app key + generic key" \
  "functionfly-vault-desktop_aarch64.app.tar.gz" \
  "darwin-aarch64-app,darwin-aarch64"

expect_targets "macOS universal app → both architectures" \
  "functionfly-vault-desktop_universal.app.tar.gz" \
  "darwin-aarch64-app,darwin-aarch64,darwin-x86_64-app,darwin-x86_64"

expect_targets "Windows NSIS → nsis key + generic key" \
  "functionfly-vault-desktop_x64-setup.exe" \
  "windows-x86_64-nsis,windows-x86_64"

expect_targets "Windows MSI → msi key only" \
  "functionfly-vault-desktop_x64_en-US.msi" \
  "windows-x86_64-msi"

# The v1 zip forms still map, so an artefact produced by an older
# pipeline is not orphaned by the correction.
expect_targets "Windows NSIS (v1 zip form) still maps" \
  "functionfly-vault-desktop_x64-setup.nsis.zip" \
  "windows-x86_64-nsis,windows-x86_64"

expect_targets "Windows MSI (v1 zip form) still maps" \
  "functionfly-vault-desktop_x64_en-US.msi.zip" \
  "windows-x86_64-msi"

expect_exit "an installer that is not an updater artefact is rejected → exit 1" 1 \
  "$PYTHON" "$DIR/updater-targets.py" "functionfly-vault-desktop_0.1.0_amd64.tar.gz"

# The Tauri v1 artefact names must be REJECTED, not mapped. The v2
# bundler never produces them, so a manifest entry built from one would
# point at a URL that does not exist.
expect_exit "the Tauri v1 .AppImage.tar.gz name is rejected → exit 1" 1 \
  "$PYTHON" "$DIR/updater-targets.py" "functionfly-vault-desktop_0.1.0_amd64.AppImage.tar.gz"

expect_exit "the Tauri v1 .deb.tar.gz name is rejected → exit 1" 1 \
  "$PYTHON" "$DIR/updater-targets.py" "functionfly-vault-desktop_0.1.0_amd64.deb.tar.gz"

expect_exit "the Tauri v1 .rpm.tar.gz name is rejected → exit 1" 1 \
  "$PYTHON" "$DIR/updater-targets.py" "functionfly-vault-desktop_0.1.0_amd64.rpm.tar.gz"

# Every key the mapper emits must be a key the plugin can look up.
if "$PYTHON" - "$DIR/updater-targets.py" <<'PY_EOF'
import re
import subprocess
import sys

names = [
    "functionfly-vault-desktop_0.1.0_amd64.AppImage",
    "functionfly-vault-desktop_0.1.0_amd64.deb",
    "functionfly-vault-desktop_0.1.0_amd64.rpm",
    "functionfly-vault-desktop_aarch64.app.tar.gz",
    "functionfly-vault-desktop_universal.app.tar.gz",
    "functionfly-vault-desktop_x64-setup.exe",
    "functionfly-vault-desktop_x64_en-US.msi",
]

pattern = re.compile(
    r"^(linux|darwin|windows)-(x86_64|aarch64|i686|armv7|riscv64)"
    r"(-(appimage|deb|rpm|app|msi|nsis))?$"
)

for name in names:
    out = subprocess.run(
        [sys.executable, sys.argv[1], name], capture_output=True, text=True, check=True
    ).stdout.strip()
    for key in out.split(","):
        assert pattern.match(key), f"{name} -> {key} is not a plugin lookup key"
PY_EOF
then
  echo "  ✓ every mapped key is a key the plugin can look up"
else
  echo "  ✗ a mapped key is not a plugin lookup key"
  fail=1
fi

# ── 10. (Optional) End-to-end round-trip against the bundled mock R2. ──
#    Skipped unless `MOCK_R2_STORE` is set, `python3` is on PATH,
#    and `aws` is installed. Captured evidence at
#    `scripts/release/evidence/2026-10-06-mock-r2-round-trip.md`.
if [[ -n "${MOCK_R2_STORE:-}" ]] && command -v python3 >/dev/null 2>&1 && command -v aws >/dev/null 2>&1; then
  echo
  echo "→ mock R2 round-trip (optional)"
  MOCK_DIR="$DIR/_mock-r2"
  MOCK_PORT=18235
  rm -rf "$MOCK_DIR"
  MOCK_R2_STORE="$MOCK_DIR" python3 "$DIR/mock-r2.py" "$MOCK_PORT" >/dev/null 2>&1 &
  MOCK_PID=$!
  # Note: $MOCK_PID must be expanded at trap-registration time (single quotes).
  trap 'kill "$MOCK_PID" 2>/dev/null || true; rm -rf "$FIXTURE_DIR"' EXIT
  # Wait for the mock to start listening
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    if curl -s -o /dev/null "http://127.0.0.1:${MOCK_PORT}/healthcheck" 2>/dev/null; then
      break
    fi
    sleep 0.2
  done

  FIXTURE_SIDECAR="$FIXTURE_DIR/functionfly-vault-desktop_0.0.1_amd64.AppImage.tar.gz"
  FIXTURE_SIG="$FIXTURE_DIR/functionfly-vault-desktop_0.0.1_amd64.AppImage.tar.gz.sig"
  if [[ ! -s "$FIXTURE_SIDECAR" ]]; then head -c 4096 /dev/urandom > "$FIXTURE_SIDECAR"; fi
  cp "$FIXTURE_DIR/app.tar.gz.sig" "$FIXTURE_SIG"

  if bash -c "source $LOAD_ENV && \
      R2_ACCOUNT_ID=mock R2_ACCESS_KEY_ID=FAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKEFAKE \
      R2_SECRET_ACCESS_KEY=fakefakesecret_secret_secret_secret_secret_secret_see \
      R2_ENDPOINT_OVERRIDE=http://127.0.0.1:${MOCK_PORT} \
      $SCRIPT stable 0.0.1 \
        $FIXTURE_SIDECAR $FIXTURE_SIG linux-x86_64" >/dev/null 2>&1; then
    if [[ -s "$MOCK_DIR/functionfly-vault-updates/vault/stable/0.0.1/functionfly-vault-desktop_0.0.1_amd64.AppImage.tar.gz" \
       && -s "$MOCK_DIR/functionfly-vault-updates/vault/stable/0.0.1/functionfly-vault-desktop_0.0.1_amd64.AppImage.tar.gz.sig" \
       && -s "$MOCK_DIR/functionfly-vault-updates/vault/stable/manifest.json" ]]; then
      echo "  ✓ publish-update.sh uploaded side-car + .sig + manifest to mock R2"
    else
      echo "  ✗ publish-update.sh succeeded but expected objects missing on the mock"
      fail=1
    fi

    # The manifest that landed must satisfy the same contract.
    if "$PYTHON" - "$MOCK_DIR/functionfly-vault-updates/vault/stable/manifest.json" <<'PY_EOF'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    manifest = json.load(handle)

assert manifest["version"] == "0.0.1", manifest.get("version")
assert "linux-x86_64" in manifest["platforms"], list(manifest["platforms"])
entry = manifest["platforms"]["linux-x86_64"]
assert entry["url"].startswith("https://"), entry["url"]
assert not entry["signature"].startswith("http"), entry["signature"]
PY_EOF
    then
      echo "  ✓ the manifest published to the mock satisfies the contract"
    else
      echo "  ✗ the manifest published to the mock violates the contract"
      fail=1
    fi
  else
    echo "  ✗ publish-update.sh failed against the mock R2"
    fail=1
  fi
  rm -rf "$FIXTURE_DIR"
fi

echo
if [[ $fail -eq 0 ]]; then
  echo "All smoke checks green ✓"
  exit 0
else
  echo "Smoke checks failed ✗"
  exit 1
fi
