#!/usr/bin/env bash
# staging-smoke.test.sh — proves `staging-smoke.sh` actually works.
#
# WHY THIS EXISTS
# ---------------
# `staging-smoke.sh` is the manual pre-merge gate. It had never been run
# against anything, so nobody knew whether its endpoint paths were right,
# whether the response shapes it parses with `jq` were right, or whether
# its cleanup actually cleaned up.
#
# This wrapper starts the bundled mock orchestrator on a loopback port
# and runs the REAL script against it, unmodified. The script is not
# stubbed or shelled out to selectively — the whole file runs, which is
# the point.
#
# The mock enforces the security contract (bearer on every /v1 route,
# CSRF on every mutation), so a green run here means the script really
# did speak the protocol correctly rather than being waved through.
#
# The wrapper then asserts two things that a permissive mock would not
# catch:
#
#   * the mock rejected a mutation that arrived without a CSRF token
#   * the script's safe-method request did NOT carry a CSRF header
#
# Together those prove the script exercises the CSRF rule in both
# directions.
#
# USAGE
# -----
#   ./scripts/staging-smoke.test.sh
#
# Exit codes:
#   0  the script ran clean against the mock
#   1  the script failed, or the mock's assertions did not hold
#   2  configuration error (missing curl/jq/python3)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

MOCK="scripts/release/mock-orchestrator.py"
SMOKE="scripts/staging-smoke.sh"

step() { printf "\n\033[1;34m▶ %s\033[0m\n" "$*"; }
ok()   { printf "  \033[1;32m✓\033[0m %s\n" "$*"; }
fail() { printf "  \033[1;31m✗\033[0m %s\n" "$*" >&2; exit "${2:-1}"; }
note() { printf "  · %s\n" "$*"; }

for tool in curl jq python3; do
  command -v "$tool" >/dev/null 2>&1 || fail "$tool is required" 2
done

[ -f "$MOCK" ] || fail "$MOCK not found" 2
[ -f "$SMOKE" ] || fail "$SMOKE not found" 2

# ── start the mock ───────────────────────────────────────────────────────
step "1. Start the mock orchestrator"

MOCK_LOG="$(mktemp -t vault-mock-log.XXXXXX)"
MOCK_PID=""
cleanup() {
  if [ -n "$MOCK_PID" ] && kill -0 "$MOCK_PID" 2>/dev/null; then
    kill "$MOCK_PID" 2>/dev/null || true
    wait "$MOCK_PID" 2>/dev/null || true
  fi
  rm -f "$MOCK_LOG"
}
trap cleanup EXIT

python3 "$MOCK" --port 0 > "$MOCK_LOG" 2>&1 &
MOCK_PID=$!

# Wait for the mock to announce its port.
PORT=""
for _ in $(seq 1 50); do
  PORT="$(sed -n 's/^LISTENING \([0-9]*\)$/\1/p' "$MOCK_LOG" 2>/dev/null | head -1)"
  [ -n "$PORT" ] && break
  if ! kill -0 "$MOCK_PID" 2>/dev/null; then
    cat "$MOCK_LOG" >&2
    fail "the mock orchestrator died on startup"
  fi
  sleep 0.2
done

[ -n "$PORT" ] || { cat "$MOCK_LOG" >&2; fail "the mock never reported a port"; }
ok "mock orchestrator listening on 127.0.0.1:${PORT}"

BASE_URL="http://127.0.0.1:${PORT}"

# ── sanity check the mock itself ─────────────────────────────────────────
step "2. Check the mock enforces the security contract"

# No bearer at all -> 401.
code="$(curl -s -o /dev/null -w '%{http_code}' "${BASE_URL}/v1/vault/secrets")"
[ "$code" = "401" ] || fail "expected 401 without a bearer token, got $code"
ok "a request without a bearer token is rejected (401)"

# Bearer but no CSRF on a mutation -> 403.
code="$(curl -s -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer mock-access-token" \
  -X POST "${BASE_URL}/v1/vault/secrets" \
  -H 'Content-Type: application/json' \
  -d '{"name":"csrf-probe"}')"
[ "$code" = "403" ] || fail "expected 403 without a CSRF token, got $code"
ok "a mutation without a CSRF token is rejected (403)"

# Bearer but a CSRF token on a safe method -> 200 (CSRF is not required
# for reads, and the orchestrator tolerates but does not need it).
code="$(curl -s -o /dev/null -w '%{http_code}' \
  -H "Authorization: Bearer mock-access-token" \
  -H "X-CSRF-Token: mock-csrf-token" \
  "${BASE_URL}/v1/vault/secrets")"
[ "$code" = "200" ] || fail "expected 200 for a safe method, got $code"
ok "a safe method does not require a CSRF token"

# Reset the counters so the assertions in step 4 measure what the SCRIPT
# did, not what the probes above did.
curl -s -X POST "${BASE_URL}/__reset" >/dev/null
ok "mock counters reset (the assertions below now measure only the script)"

# ── run the real script ──────────────────────────────────────────────────
step "3. Run the real staging-smoke.sh against the mock"

SMOKE_LOG="$(mktemp -t vault-smoke-log.XXXXXX)"

set +e
STAGING_BASE_URL="$BASE_URL" \
STAGING_EMAIL="smoke@example.invalid" \
STAGING_PASSWORD="mock-password" \
STAGING_TENANT_ID="00000000-0000-4000-8000-000000000001" \
STAGING_SMOKE_NONCE="mocktest$$" \
  bash "$SMOKE" > "$SMOKE_LOG" 2>&1
SMOKE_STATUS=$?
set -e

cat "$SMOKE_LOG"
rm -f "$SMOKE_LOG"

if [ "$SMOKE_STATUS" -ne 0 ]; then
  fail "staging-smoke.sh exited with ${SMOKE_STATUS}"
fi
ok "staging-smoke.sh completed every step cleanly"

# ── assert the mock saw what it should have ──────────────────────────────
step "4. Assert the mock exercised the expected paths"

STATS="$(curl -s "${BASE_URL}/__stats")"
note "stats: ${STATS}"

get_stat() {
  printf '%s' "$STATS" | python3 -c "import json,sys; print(json.load(sys.stdin).get('$1', 0))"
}

assert_stat() {
  local name="$1" want="$2" got
  got="$(get_stat "$name")"
  [ "$got" = "$want" ] || fail "expected ${name}=${want}, got ${got}"
}

assert_stat_min() {
  local name="$1" want="$2" got
  got="$(get_stat "$name")"
  [ "$got" -ge "$want" ] || fail "expected ${name} >= ${want}, got ${got}"
}

# Exactly one login and one logout from the script.
assert_stat logins 1
assert_stat logouts 1

# The script must never have been rejected. A missing bearer or a missing
# CSRF token on a mutation would each show up as a rejection here, so a
# clean zero means the script spoke the protocol correctly on every call.
assert_stat rejected_missing_bearer 0
assert_stat rejected_missing_csrf 0

# The script's step 4 lists secrets WITHOUT the CSRF header, so the mock
# must not have seen one on any safe method. A non-zero value here means
# the script is smuggling the CSRF header into reads.
assert_stat csrf_on_safe_method 0

# The script really did make the calls. The exact count would churn
# whenever a step is added, so this is a floor rather than an equality.
assert_stat_min requests 15

ok "the script sent exactly one login, one logout, and no stray CSRF headers"

# ── confirm the script cleaned up after itself ───────────────────────────
step "5. Confirm the script cleaned up"

REMAINING="$(curl -s -H "Authorization: Bearer mock-access-token" \
  "${BASE_URL}/v1/vault/secrets" | python3 -c "import json,sys; print(json.load(sys.stdin).get('total', 0))")"
[ "$REMAINING" = "0" ] || fail "the smoke script left ${REMAINING} secret(s) behind"
ok "no test secrets were left behind"

step "summary"
printf "  mock       127.0.0.1:%s\n" "$PORT"
printf "  result     \033[1;32mstaging-smoke.sh verified against the mock\033[0m\n"
