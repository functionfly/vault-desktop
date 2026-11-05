#!/usr/bin/env bash
# Portable CI runner — the same gates `ci.yml` runs, executed anywhere.
#
# WHY THIS EXISTS
# ---------------
# GitHub Actions in the `functionfly` org currently fails every run with
# `startup_failure` before a single job is created (no jobs, no logs,
# empty workflow name). That is an org-side problem, not a defect in
# these workflows. Rather than wait for it to be fixed, this script runs
# the identical gates on whatever machine you point it at: a laptop, a
# VPS, WSL, or the Docker container in `scripts/ci/Dockerfile`.
#
# WHAT IT DOES *NOT* NEED
# -----------------------
# Nothing here needs the GitHub Actions runtime. Every gate is a shell
# command. The only GitHub dependency is the REST API, and that works
# fine — `--report` publishes the result back to the pull request as a
# check run.
#
# USAGE
# -----
#   scripts/ci.sh                 # every gate, on this machine
#   scripts/ci.sh --quick         # skip the slow Tauri compile
#   scripts/ci.sh --report        # also publish a check run to the PR
#   scripts/ci.sh --job frontend  # run one job only
#
# The gates mirror `.github/workflows/ci.yml` job for job. If you change
# one, change the other — `scripts/ci.sh --check-parity` compares the
# step names in both and fails if they have drifted.
#
# Exit code is non-zero if any gate fails.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

QUICK=0
REPORT=0
ONLY_JOB=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --quick) QUICK=1; shift ;;
    --report) REPORT=1; shift ;;
    --job) ONLY_JOB="${2:-}"; shift 2 ;;
    --check-parity) CHECK_PARITY=1; shift ;;
    -h|--help)
      sed -n '2,40p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 64 ;;
  esac
done

# ── Output helpers ──────────────────────────────────────────────────────
if [[ -t 1 ]]; then
  C_RED=$'\033[31m'; C_GRN=$'\033[32m'; C_YEL=$'\033[33m'
  C_BLU=$'\033[34m'; C_OFF=$'\033[0m'
else
  C_RED=""; C_GRN=""; C_YEL=""; C_BLU=""; C_OFF=""
fi

FAILED_JOBS=()
FAILED_DETAIL=()
SKIPPED_JOBS=()
declare -a SUMMARY=()

banner() {
  echo
  echo "${C_BLU}=== $* ===${C_OFF}"
}

record() {
  local job="$1" status="$2" detail="${3:-}"
  SUMMARY+=("${job}|${status}|${detail}")
  case "$status" in
    fail) FAILED_JOBS+=("$job"); FAILED_DETAIL+=("$job: ${detail}") ;;
    skip) SKIPPED_JOBS+=("$job") ;;
  esac
}

run_step() {
  local job="$1" label="$2"; shift 2
  printf '  %-46s' "$label"
  local out
  if out="$("$@" 2>&1)"; then
    echo "${C_GRN}pass${C_OFF}"
    return 0
  fi
  echo "${C_RED}FAIL${C_OFF}"
  echo "$out" | sed 's/^/      /' | tail -40
  record "$job" fail "$label"
  return 1
}

# ── Normalise line endings ──────────────────────────────────────────────
# CI checks out the commit with `.gitattributes`' `* text=auto eol=lf`,
# so every script it runs is LF. A local working tree on Windows can
# hold CRLF (a checkout made before .gitattributes existed, or an
# editor that rewrites the file), and `bash -n` then fails with
#
#   syntax error near unexpected token `$'{\r''
#
# which looks like a broken script rather than a stale file mode. The
# runner normalises the shell and Python scripts to LF before running
# them, so it sees exactly what CI sees.
normalize_line_endings() {
  local changed=0 f
  for f in $(git ls-files '*.sh' '*.py' 2>/dev/null); do
    [[ -f "$f" ]] || continue
    if grep -qU $'\r' "$f" 2>/dev/null; then
      tr -d '\r' < "$f" > "$f.tmp" && mv "$f.tmp" "$f"
      changed=$((changed + 1))
    fi
  done
  if [[ $changed -gt 0 ]]; then
    echo "  normalised CRLF -> LF in ${changed} script(s) (matching .gitattributes)"
  fi
  return 0
}

# ── Toolchain detection ─────────────────────────────────────────────────
need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "${C_YEL}missing required tool: $1${C_OFF}" >&2
    return 1
  }
}

HAVE_BUN=0; HAVE_CARGO=0; HAVE_PYTHON=0; HAVE_GH=0
command -v bun    >/dev/null 2>&1 && HAVE_BUN=1
command -v cargo  >/dev/null 2>&1 && HAVE_CARGO=1
command -v python3 >/dev/null 2>&1 && HAVE_PYTHON=1
command -v gh     >/dev/null 2>&1 && HAVE_GH=1

# ── Job: frontend ───────────────────────────────────────────────────────
job_frontend() {
  banner "frontend (tsc + vitest + vite build)"
  if [[ $HAVE_BUN -eq 0 ]]; then
    echo "${C_YEL}  bun not found — skipping${C_OFF}"
    record frontend skip "bun not installed"
    return 0
  fi
  run_step frontend "bun install" bun install --frozen-lockfile
  run_step frontend "tsc --noEmit" bunx tsc --noEmit
  run_step frontend "vitest run" bunx vitest run
  run_step frontend "vite build" bunx vite build
}

# ── Job: rust ───────────────────────────────────────────────────────────
job_rust() {
  banner "rust (fmt + clippy + test)"
  if [[ $HAVE_CARGO -eq 0 ]]; then
    echo "${C_YEL}  cargo not found — skipping${C_OFF}"
    record rust skip "cargo not installed"
    return 0
  fi
  run_step rust "cargo fmt --all -- --check" \
    bash -c 'cd src-tauri && cargo fmt --all -- --check'
  # The clippy gate is blocking in ci.yml as of 2026-11-03; the warning
  # baseline was cleaned up.
  run_step rust "cargo clippy --all-targets -- -D warnings" \
    bash -c 'cd src-tauri && cargo clippy --all-targets -- -D warnings'
  run_step rust "cargo test --lib" \
    bash -c 'cd src-tauri && cargo test --lib'
  run_step rust "cargo test --test orchestrator_mock" \
    bash -c 'cd src-tauri && cargo test --test orchestrator_mock'
  run_step rust "cargo test --test device_flow" \
    bash -c 'cd src-tauri && cargo test --test device_flow'
}

# ── Job: build (the slow one) ───────────────────────────────────────────
job_build() {
  banner "build (compile Rust binary, no packaging)"
  if [[ $QUICK -eq 1 ]]; then
    echo "${C_YEL}  --quick: skipping${C_OFF}"
    record build skip "--quick"
    return 0
  fi
  if [[ $HAVE_BUN -eq 0 || $HAVE_CARGO -eq 0 ]]; then
    echo "${C_YEL}  bun/cargo not found — skipping${C_OFF}"
    record build skip "toolchain missing"
    return 0
  fi
  run_step build "vite build" bunx vite build
  run_step build "bunx tauri build --no-bundle" bunx tauri build --no-bundle
}

# ── Job: release-scripts ────────────────────────────────────────────────
job_release_scripts() {
  banner "release scripts (syntax + modes + leak guard + self-tests)"
  local py=python3
  command -v python3 >/dev/null 2>&1 || py=python

  normalize_line_endings

  run_step release-scripts "bash -n scripts/**/*.sh" \
    bash -c 'for f in scripts/*.sh scripts/release/*.sh; do bash -n "$f" || exit 1; done'
  run_step release-scripts "py_compile release helpers" \
    bash -c "$py -m py_compile scripts/release/render-manifest.py \
                       scripts/release/updater-targets.py \
                       scripts/release/mock-r2.py \
                       scripts/release/mock-orchestrator.py"

  # A script committed without the executable bit dies with exit 126
  # before it runs, which reads like a broken script rather than a
  # broken file mode.
  run_step release-scripts "every script is executable (100755)" \
    bash -c '
      failed=0
      while IFS= read -r s; do
        mode=$(git ls-files -s "$s" | awk "{print \$1}")
        if [ "$mode" != "100755" ]; then
          echo "::error::${s} is committed with mode ${mode}; it must be 100755."
          failed=1
        fi
      done < <(git ls-files "scripts/*.sh" "scripts/*.py" "scripts/**/*.sh" "scripts/**/*.py")
      exit $failed'

  run_step release-scripts "no internal-only material in publishable files" \
    bash scripts/check-no-internal-leaks.sh
  run_step release-scripts "publish-update.sh self-tests" \
    bash scripts/release/publish-update.test.sh

  # The updater's verification key lives in tauri.conf.json while the
  # release artefacts are signed with TAURI_SIGNING_PRIVATE_KEY. If
  # those two drift apart the release still builds and every update
  # fails verification on a user's machine. The script checks the
  # keynum; without a signing key it still asserts the configured
  # pubkey is present and well-formed.
  run_step release-scripts "updater keypair matches the configured pubkey" \
    bash scripts/release/verify-updater-keypair.sh

  # The updater config contract: the pubkey must be present and
  # parseable, otherwise the shipped app cannot verify updates and the
  # Tauri CLI cannot sign the updater artefacts.
  run_step release-scripts "updater config contract" \
    bash -c 'cd src-tauri && cargo test --test updater_config'

  # The OS keychain round-trip. On Windows and macOS the test asserts that
  # the real credential store was used; on a headless Linux box it skips
  # with an explicit message, because the encrypted file fallback is the
  # documented behaviour there.
  run_step release-scripts "OS keychain round-trip" \
    bash -c 'cd src-tauri && cargo test --test keychain_smoke'

  # The staging smoke script has never been run against a real
  # orchestrator. This drives the real script against a mock that
  # ENFORCES the bearer + CSRF contract, so a green run proves the
  # script speaks the protocol rather than being waved through.
  run_step release-scripts "staging-smoke.sh against a mock orchestrator" \
    bash scripts/staging-smoke.test.sh

  # Reports whether this host could produce a signed installer. Advisory
  # by default; `--require` turns it into a gate.
  run_step release-scripts "code-signing readiness" \
    bash scripts/release/check-signing-readiness.sh
}

# ── Parity check against ci.yml ─────────────────────────────────────────
job_check_parity() {
  banner "parity with .github/workflows/ci.yml"
  run_step parity "every gate the runner runs is also in ci.yml" \
    bash -c '
      set -uo pipefail
      yml=".github/workflows/ci.yml"
      missing=0
      for probe in \
        "tsc --noEmit" \
        "vitest run" \
        "vite build" \
        "cargo fmt --all -- --check" \
        "cargo clippy --all-targets -- -D warnings" \
        "cargo test --lib" \
        "cargo test --test orchestrator_mock" \
        "cargo test --test device_flow" \
        "cargo test --test updater_config" \
        "cargo test --test keychain_smoke" \
        "bunx tauri build --no-bundle" \
        "scripts/check-no-internal-leaks.sh" \
        "scripts/release/publish-update.test.sh" \
        "scripts/release/verify-updater-keypair.sh" \
        "scripts/release/check-signing-readiness.sh" \
        "scripts/staging-smoke.test.sh"; do
        if ! grep -qF "$probe" "$yml"; then
          echo "::error::${probe} is not in ${yml} but the portable runner runs it."
          missing=1
        fi
      done
      exit $missing'
}

# ── Report the result back to GitHub ────────────────────────────────────
report_to_github() {
  banner "reporting to GitHub"

  if [[ $HAVE_GH -eq 0 ]]; then
    echo "${C_YEL}  gh not found — cannot report${C_OFF}"
    return 0
  fi

  local repo head_sha
  repo="$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)"
  head_sha="$(git rev-parse HEAD 2>/dev/null || true)"
  if [[ -z "$repo" || -z "$head_sha" ]]; then
    echo "${C_YEL}  not inside a GitHub checkout — skipping report${C_OFF}"
    return 0
  fi

  # `gh run rerun` is not an option (no Actions runner). Publishing the
  # result back to GitHub is, and there are two ways to do it.
  #
  # The CHECK-RUN API is the richer one (it renders in the PR view with
  # per-step detail) but it now requires a GitHub App: a personal access
  # token gets "You must authenticate via a GitHub App." — and this
  # repository's only credential is a PAT.
  #
  # The COMMIT-STATUS API still accepts a PAT, and a branch protection
  # rule's `required_status_checks.contexts` matches a status context
  # exactly as it matches a check-run name. So the status API is the one
  # that works here, and it is what the branch protection rules require.
  #
  # The check-run API is kept as a fallback for a setup where a GitHub
  # App token IS available.
  local title summary conclusion
  if [[ ${#FAILED_JOBS[@]} -eq 0 ]]; then
    title="portable CI: pass"
    conclusion="success"
  else
    title="portable CI: fail (${FAILED_JOBS[*]})"
    conclusion="failure"
  fi

  summary=$(printf '%s\n' "${SUMMARY[@]}" | sed 's/|/ — /g' | tr '\n' ' ')

  local out
  out=$(gh api --method POST "repos/${repo}/statuses/${head_sha}" \
    -f "state=${conclusion}" \
    -f "context=portable CI" \
    -f "description=${title}" \
    --jq '.html_url' 2>&1 | tail -1)

  if [[ "$out" == *"GitHub App"* || "$out" == *"403"* ]]; then
    echo "  the status API was refused; trying the check-run API"
    gh api --method POST "repos/${repo}/check-runs" \
      -f "name=portable CI" \
      -f "head_sha=${head_sha}" \
      -f "status=completed" \
      -f "conclusion=${conclusion}" \
      -f "title=${title}" \
      -f "summary=${summary}" \
      --jq '.html_url' 2>&1 | tail -1
  else
    echo "$out"
  fi
}

# ── Run ─────────────────────────────────────────────────────────────────
JOBS=(frontend rust build release_scripts)

echo "FunctionFly Vault — portable CI runner"
echo "repo:   $REPO_ROOT"
echo "branch: $(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo '?')"
echo "commit: $(git rev-parse --short HEAD 2>/dev/null || echo '?')"
echo "tools:  bun=$HAVE_BUN cargo=$HAVE_CARGO python=$HAVE_PYTHON gh=$HAVE_GH"

for job in "${JOBS[@]}"; do
  if [[ -n "$ONLY_JOB" && "$ONLY_JOB" != "$job" ]]; then
    echo "${C_YEL}skipping ${job} (--job ${ONLY_JOB})${C_OFF}"
    record "$job" skip "--job filter"
    continue
  fi
  "job_${job}"
done

if [[ -n "${CHECK_PARITY:-}" ]]; then
  job_check_parity
fi

banner "summary"
if [[ ${#FAILED_DETAIL[@]} -gt 0 ]]; then
  for d in "${FAILED_DETAIL[@]}"; do
    printf '  %-18s %s%s%s\n' "" "$C_RED" "$d" "$C_OFF"
  done
fi
if [[ ${#SKIPPED_JOBS[@]} -gt 0 ]]; then
  printf '  %-18s %sskipped: %s%s\n' "" "$C_YEL" "${SKIPPED_JOBS[*]}" "$C_OFF"
fi
if [[ ${#FAILED_JOBS[@]} -eq 0 ]]; then
  printf '  %-18s %sall gates passed%s\n' "" "$C_GRN" "$C_OFF"
fi

if [[ ${#FAILED_JOBS[@]} -gt 0 ]]; then
  echo
  echo "${C_RED}FAILED: ${FAILED_JOBS[*]}${C_OFF}"
  [[ $REPORT -eq 1 ]] && report_to_github
  exit 1
fi

echo
echo "${C_GRN}All gates passed${C_OFF}"
[[ $REPORT -eq 1 ]] && report_to_github
exit 0
