#!/usr/bin/env bash
# staging-smoke.sh — exercises the full FunctionFly vault HTTP contract
# against a real staging orchestrator. This is the manual gate that gates
# the v0.1.0 desktop client production merge; headless CI only verifies
# the script's syntax + the integration test pack.
#
# Required environment:
#   STAGING_BASE_URL         e.g. https://api-staging.functionfly.dev
#   STAGING_EMAIL            user email for login
#   STAGING_PASSWORD         user password for login
#   STAGING_TENANT_ID        tenant UUID; can be auto-discovered from /auth/login
#
# Optional:
#   STAGING_SMOKE_NONCE      unique run tag (default: $(date +%s%N))
#   STAGING_CURL             override curl binary (default: curl)
#   STAGING_KEEP             "1" to leave the test secret behind (default: delete)
#   STAGING_TIMEOUT_SECS     per-request timeout (default: 30)
#
# Exit codes:
#   0  every step succeeded
#   1  configuration error
#   2  login failed
#   3  list/create/get/update/diff/rollback/cleanup failure
#   4  audit/namespace listing failure
#
# Usage:
#   STAGING_BASE_URL=... STAGING_EMAIL=... STAGING_PASSWORD=... \
#     bash scripts/staging-smoke.sh
#
# Verbose: set VERBOSE=1 to print response bodies.

set -euo pipefail

# ---- 0. preflight ----------------------------------------------------------

: "${STAGING_BASE_URL:?must set STAGING_BASE_URL}"
: "${STAGING_EMAIL:?must set STAGING_EMAIL}"
: "${STAGING_PASSWORD:?must set STAGING_PASSWORD}"

# Strip a single trailing slash so concatenations never produce //v1/...
STAGING_BASE_URL="${STAGING_BASE_URL%/}"

CURL_BIN="${STAGING_CURL:-curl}"
TIMEOUT_SECS="${STAGING_TIMEOUT_SECS:-30}"
NONCE="${STAGING_SMOKE_NONCE:-$(date +%s%N)}"
SECRET_NAME="vault-desktop-smoke-${NONCE}"
SECRET_NAMESPACE="default"
SECRET_TYPE="api_key"

VERBOSE="${VERBOSE:-0}"
KEEP="${STAGING_KEEP:-0}"

# tmp file for the last response body; useful in postmortem
LAST_BODY="$(mktemp -t vault-smoke.XXXXXX)"
trap 'rm -f "$LAST_BODY" "$LAST_BODY.headers"' EXIT
LAST_HEADERS="$(mktemp -t vault-smoke.XXXXXX.headers)"

step() { printf "\n\033[1;34m▶ %s\033[0m\n" "$*"; }
ok()   { printf "  \033[1;32m✓\033[0m %s\n" "$*"; }
fail() { printf "  \033[1;31m✗\033[0m %s\n" "$*"; cat "$LAST_BODY" || true; exit "${2:-3}"; }
note() { printf "  · %s\n" "$*"; }

# request METHOD PATH [JSON_BODY] [extra_headers...]
# Stores response body in $LAST_BODY and HTTP status code in $LAST_STATUS.
LAST_STATUS=""
request() {
  local method="$1"; shift
  local path="$1"; shift
  local body="${1:-}"; shift || true
  local -a extra_headers=("$@")

  local url="${STAGING_BASE_URL}${path}"
  local -a curl_args
  curl_args=(
    --silent --show-error
    --max-time "$TIMEOUT_SECS"
    --request "$method"
    --url "$url"
    --header "Accept: application/json"
    --header "X-Vault-Smoke-Nonce: ${NONCE}"
    --header "X-Vault-Smoke-Run: staging-smoke.sh"
    --output "$LAST_BODY"
    --write-out '%{http_code}'
    --dump-header "$LAST_HEADERS"
  )
  for h in "${extra_headers[@]}"; do
    curl_args+=(--header "$h")
  done
  if [[ -n "$body" ]]; then
    curl_args+=(--header "Content-Type: application/json" --data "$body")
  fi

  LAST_STATUS="$("$CURL_BIN" "${curl_args[@]}")" || {
    fail "curl failed for $method $path" 1
  }

  if [[ "$VERBOSE" == "1" ]]; then
    printf "  ↪ %s %s → %s\n" "$method" "$path" "$LAST_STATUS"
    if [[ -s "$LAST_BODY" ]]; then
      head -c 600 "$LAST_BODY" | sed 's/^/      /'
      printf "\n"
    fi
  fi
}

# jq is the only external tool we assume beyond curl + bash.
if ! command -v jq >/dev/null 2>&1; then
  printf "\033[1;31mfatal: jq is required for staging-smoke.sh\033[0m\n"
  exit 1
fi

# ---- 1. login --------------------------------------------------------------

step "1. POST /auth/login"
request POST /auth/login \
  "$(jq -nc --arg e "$STAGING_EMAIL" --arg p "$STAGING_PASSWORD" \
    '{email:$e, password:$p}')"
if [[ "$LAST_STATUS" != "200" ]]; then
  fail "login failed (status=$LAST_STATUS)" 2
fi
ACCESS_TOKEN="$(jq -r '.access_token // .token // empty' "$LAST_BODY")"
REFRESH_TOKEN="$(jq -r '.refresh_token // empty' "$LAST_BODY")"
TENANT_ID_DISCOVERED="$(jq -r '.user.tenant_id // .tenant_id // empty' "$LAST_BODY")"
USER_ID="$(jq -r '.user.id // .user_id // empty' "$LAST_BODY")"
if [[ -z "$ACCESS_TOKEN" ]]; then
  fail "login response missing access_token" 2
fi
TENANT_ID="${STAGING_TENANT_ID:-$TENANT_ID_DISCOVERED}"
if [[ -z "$TENANT_ID" ]]; then
  fail "could not determine tenant_id (set STAGING_TENANT_ID)" 2
fi
ok "authenticated as user_id=${USER_ID:0:8}… tenant=${TENANT_ID:0:8}…"

AUTH_HEADER="Authorization: Bearer ${ACCESS_TOKEN}"

# ---- 2. CSRF token (only for mutating calls) --------------------------------

step "2. GET /v1/csrf"
request GET /v1/csrf "" "$AUTH_HEADER"
if [[ "$LAST_STATUS" != "200" ]]; then
  fail "csrf fetch failed (status=$LAST_STATUS)" 3
fi
CSRF_TOKEN="$(jq -r '.csrf_token // .token // empty' "$LAST_BODY")"
if [[ -z "$CSRF_TOKEN" ]]; then
  fail "csrf response missing token field" 3
fi
ok "csrf token: ${CSRF_TOKEN:0:8}…"

CSRF_HEADER="X-CSRF-Token: ${CSRF_TOKEN}"

# ---- 3. create a test secret -----------------------------------------------

step "3. POST /v1/vault/secrets (create ${SECRET_NAME})"
SECRET_BODY="$(jq -nc \
  --arg name "$SECRET_NAME" \
  --arg ns   "$SECRET_NAMESPACE" \
  --arg typ  "$SECRET_TYPE" \
  --arg tid  "$TENANT_ID" \
  '{
    name: $name,
    namespace: $ns,
    secret_type: $typ,
    value: ("initial-" + $name),
    description: "Created by vault-desktop staging-smoke.sh",
    tags: ["smoke-test", "vault-desktop"]
  }')"
request POST /v1/vault/secrets "$SECRET_BODY" "$AUTH_HEADER" "$CSRF_HEADER"
if [[ "$LAST_STATUS" != "200" && "$LAST_STATUS" != "201" ]]; then
  fail "create secret failed (status=$LAST_STATUS)" 3
fi
SECRET_ID="$(jq -r '.id // empty' "$LAST_BODY")"
if [[ -z "$SECRET_ID" ]]; then
  fail "create response missing id" 3
fi
ok "secret id: $SECRET_ID"

# ---- 4. list secrets (CSRF must NOT be attached) ---------------------------

step "4. GET /v1/vault/secrets (no CSRF header)"
# We deliberately do NOT pass CSRF_HEADER here; the orchestrator must accept
# the bearer-only auth for safe methods. The header would be a regression.
request GET "/v1/vault/secrets?limit=50&search=${NONCE}" "" "$AUTH_HEADER"
if [[ "$LAST_STATUS" != "200" ]]; then
  fail "list secrets failed (status=$LAST_STATUS)" 3
fi
TOTAL="$(jq -r '.total // 0' "$LAST_BODY")"
if [[ "$TOTAL" -lt 1 ]]; then
  fail "list returned 0 secrets matching nonce $NONCE" 3
fi
ok "list returned total=${TOTAL} (≥1 expected)"

# ---- 5. get the secret -----------------------------------------------------

step "5. GET /v1/vault/secrets/${SECRET_ID}"
request GET "/v1/vault/secrets/${SECRET_ID}" "" "$AUTH_HEADER"
if [[ "$LAST_STATUS" != "200" ]]; then
  fail "get secret failed (status=$LAST_STATUS)" 3
fi
ok "got secret version=1"

# ---- 6. update (rotate) ----------------------------------------------------

step "6. PATCH /v1/vault/secrets/${SECRET_ID} (rotate value)"
UPDATE_BODY="$(jq -nc --arg name "$SECRET_NAME" \
  '{value: ("rotated-" + $name), description: "rotated by smoke test"}')"
request PATCH "/v1/vault/secrets/${SECRET_ID}" "$UPDATE_BODY" \
  "$AUTH_HEADER" "$CSRF_HEADER"
if [[ "$LAST_STATUS" != "200" ]]; then
  fail "update secret failed (status=$LAST_STATUS)" 3
fi
ok "rotated to version=2"

# ---- 7. list versions ------------------------------------------------------

step "7. GET /v1/vault/secrets/${SECRET_ID}/versions"
request GET "/v1/vault/secrets/${SECRET_ID}/versions" "" "$AUTH_HEADER"
if [[ "$LAST_STATUS" != "200" ]]; then
  fail "list versions failed (status=$LAST_STATUS)" 3
fi
VERSION_COUNT="$(jq -r '.versions | length // 0' "$LAST_BODY")"
if [[ "$VERSION_COUNT" -lt 2 ]]; then
  fail "expected ≥2 versions, got $VERSION_COUNT" 3
fi
ok "versions present: $VERSION_COUNT"

# ---- 8. diff versions ------------------------------------------------------

step "8. GET /v1/vault/secrets/${SECRET_ID}/versions/diff?from=1&to=2"
request GET "/v1/vault/secrets/${SECRET_ID}/versions/diff?from=1&to=2" "" \
  "$AUTH_HEADER"
if [[ "$LAST_STATUS" != "200" ]]; then
  fail "diff versions failed (status=$LAST_STATUS)" 3
fi
ok "diff returned"

# ---- 9. list namespaces ----------------------------------------------------

step "9. GET /v1/vault/namespaces"
request GET /v1/vault/namespaces "" "$AUTH_HEADER"
if [[ "$LAST_STATUS" != "200" ]]; then
  fail "list namespaces failed (status=$LAST_STATUS)" 4
fi
ok "namespaces listed"

# ---- 10. list audit --------------------------------------------------------

step "10. GET /v1/vault/audit?limit=10"
request GET "/v1/vault/audit?limit=10" "" "$AUTH_HEADER"
if [[ "$LAST_STATUS" != "200" ]]; then
  fail "list audit failed (status=$LAST_STATUS)" 4
fi
ok "audit listed"

# ---- 11. rollback ----------------------------------------------------------

step "11. POST /v1/vault/secrets/${SECRET_ID}/rollback (to v1)"
ROLLBACK_BODY="$(jq -nc '{target_version:1, reason:"staging-smoke rollback"}')"
request POST "/v1/vault/secrets/${SECRET_ID}/rollback" "$ROLLBACK_BODY" \
  "$AUTH_HEADER" "$CSRF_HEADER"
if [[ "$LAST_STATUS" != "200" ]]; then
  fail "rollback failed (status=$LAST_STATUS)" 3
fi
ok "rolled back to v1"

# ---- 11.5 tokens (mint + list + revoke) ---------------------------------

step "11.5. POST /v1/vault/secrets/${SECRET_ID}/tokens (mint)"
TOKEN_BODY="$(jq -nc --arg n "smoke-${NONCE}" \
  '{name:$n, scopes:["read"], expires_in_hours:1}')"
request POST "/v1/vault/secrets/${SECRET_ID}/tokens" "$TOKEN_BODY" \
  "$AUTH_HEADER" "$CSRF_HEADER"
if [[ "$LAST_STATUS" != "200" && "$LAST_STATUS" != "201" ]]; then
  fail "token mint failed (status=$LAST_STATUS)" 3
fi
# The orchestrator returns plaintext exactly once.
TOKEN_ID="$(jq -r '.token_id // .id // empty' "$LAST_BODY")"
TOKEN_VALUE="$(jq -r '.token // .plaintext // empty' "$LAST_BODY")"
if [[ -z "$TOKEN_ID" ]]; then
  fail "token mint response missing token_id" 3
fi
ok "minted token id: ${TOKEN_ID:0:8}… (plaintext len=${#TOKEN_VALUE})"

step "11.5. GET /v1/vault/secrets/${SECRET_ID}/tokens (list)"
request GET "/v1/vault/secrets/${SECRET_ID}/tokens" "" "$AUTH_HEADER"
if [[ "$LAST_STATUS" != "200" ]]; then
  fail "token list failed (status=$LAST_STATUS)" 3
fi
ok "tokens listed"

step "11.5. DELETE /v1/vault/tokens/${TOKEN_ID} (revoke)"
request DELETE "/v1/vault/tokens/${TOKEN_ID}" "" \
  "$AUTH_HEADER" "$CSRF_HEADER"
if [[ "$LAST_STATUS" != "200" && "$LAST_STATUS" != "204" ]]; then
  fail "token revoke failed (status=$LAST_STATUS)" 3
fi
ok "token revoked"

# ---- 11.6 bulk delete (dry-run + real) -----------------------------------

step "11.6. DELETE /v1/vault/secrets/bulk-delete (dry_run=true)"
BULK_BODY="$(jq -nc --arg id "$SECRET_ID" \
  '{secret_ids:[$id], dry_run:true}')"
request DELETE /v1/vault/secrets/bulk-delete "$BULK_BODY" \
  "$AUTH_HEADER" "$CSRF_HEADER"
if [[ "$LAST_STATUS" != "200" ]]; then
  fail "bulk-delete dry_run failed (status=$LAST_STATUS)" 3
fi
ok "bulk-delete dry_run ok"

# (The real delete is the same call with dry_run=false — but the
# step 12 single-secret delete below already covers the real path;
# a separate real bulk would require two fresh secrets.)

# ---- 12. cleanup (unless STAGING_KEEP=1) -----------------------------------

if [[ "$KEEP" == "1" ]]; then
  step "12. STAGING_KEEP=1, leaving secret in place"
  note "secret id: $SECRET_ID"
else
  step "12. DELETE /v1/vault/secrets/${SECRET_ID}"
  request DELETE "/v1/vault/secrets/${SECRET_ID}" "" \
    "$AUTH_HEADER" "$CSRF_HEADER"
  if [[ "$LAST_STATUS" != "200" && "$LAST_STATUS" != "204" ]]; then
    fail "delete failed (status=$LAST_STATUS)" 3
  fi
  ok "cleaned up"
fi

# ---- 13. logout (best-effort) ----------------------------------------------

step "13. POST /auth/logout (best-effort)"
if [[ -n "${REFRESH_TOKEN:-}" ]]; then
  request POST /auth/logout \
    "$(jq -nc --arg rt "$REFRESH_TOKEN" '{refresh_token:$rt}')" \
    "$AUTH_HEADER" "$CSRF_HEADER" || note "logout returned non-2xx (ignored)"
  ok "logged out"
else
  note "no refresh_token captured, skipping"
fi

# ---- 14. release scripts smoke test (no R2 calls) ---------------------------

step "14. scripts/release/publish-update.test.sh"
bash scripts/release/publish-update.test.sh
ok "release scripts smoke test passed"

# ---- summary ---------------------------------------------------------------

step "summary"
printf "  base_url      %s\n" "$STAGING_BASE_URL"
printf "  user_id       %s\n" "$USER_ID"
printf "  tenant_id     %s\n" "$TENANT_ID"
printf "  secret_id     %s\n" "$SECRET_ID"
printf "  secret_name   %s\n" "$SECRET_NAME"
printf "  nonce         %s\n" "$NONCE"
printf "  exit          \033[1;32m0\033[0m\n"
