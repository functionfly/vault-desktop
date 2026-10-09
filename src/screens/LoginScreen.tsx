import { useCallback, useEffect, useState } from "react";
import { ipc, type DeviceCodePrompt, type StoreBackend } from "../lib/ipc";
import { VaultIpcError } from "../lib/tauri";

interface Props {
  error: string | null;
  onLoggedIn: () => Promise<void> | void;
  onCloseError: () => void;
}

/** Production orchestrator. Kept in sync with
 *  `DEFAULT_ORCHESTRATOR_URL` in `src-tauri/src/commands/mod.rs`; the Rust
 *  side seeds the same value from `VAULT_ORCHESTRATOR_URL` when present, and
 *  a returning user gets whatever they last signed into. */
const PRODUCTION_ORCHESTRATOR = "https://api.functionfly.com";

/** How the user is signing in. Browser is the default — it is the only mode
 *  where the app never touches a password. */
type AuthMode = "browser" | "password" | "apiKey";

type ProbeState =
  | { phase: "idle" }
  | { phase: "checking" }
  | { phase: "ok"; latencyMs: number }
  | { phase: "fail"; message: string };

export function LoginScreen({ error, onLoggedIn, onCloseError }: Props) {
  const [mode, setMode] = useState<AuthMode>("browser");
  const [baseUrl, setBaseUrl] = useState(PRODUCTION_ORCHESTRATOR);
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const [probe, setProbe] = useState<ProbeState>({ phase: "idle" });
  const [backend, setBackend] = useState<StoreBackend | null>(null);
  /** Set once the orchestrator has issued a code and we are waiting for the
   *  human. Drives the "waiting for approval" panel instead of the form. */
  const [devicePrompt, setDevicePrompt] = useState<DeviceCodePrompt | null>(null);

  // On mount, ask the Rust core what it already knows. A returning user gets
  // their last orchestrator pre-filled instead of the localhost default they
  // would otherwise have to retype every launch.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const info = await ipc.storeInfo();
        if (cancelled) return;
        setBackend(info.backend);
        if (info.stored_base_url) setBaseUrl(info.stored_base_url);
      } catch {
        // A store that cannot even describe itself is not worth blocking the
        // login form over — the user can still type a URL and sign in.
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const testConnection = useCallback(async () => {
    setProbe({ phase: "checking" });
    try {
      const result = await ipc.healthProbe(baseUrl.trim());
      setProbe(
        result.reachable
          ? { phase: "ok", latencyMs: result.latency_ms }
          : {
              phase: "fail",
              message:
                result.detail ??
                "That host did not answer. Check the URL and your connection.",
            },
      );
    } catch (e) {
      setProbe({ phase: "fail", message: messageFor(e) });
    }
  }, [baseUrl]);

  // Reset the probe whenever the URL changes — a green check next to the old
  // URL would be a lie.
  useEffect(() => {
    setProbe({ phase: "idle" });
  }, [baseUrl]);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setSubmitting(true);
    setLocalError(null);
    try {
      if (mode === "password") {
        await ipc.login(baseUrl.trim(), email, password);
      } else if (mode === "apiKey") {
        await ipc.loginWithApiKey(baseUrl.trim(), apiKey);
      } else {
        // Browser sign-in is two steps: ask for a code, show it, then block
        // until the human approves. The code is displayed between the two so
        // the user knows what the browser is about to ask them to confirm.
        const prompt = await ipc.beginDeviceLogin(baseUrl.trim());
        setDevicePrompt(prompt);
        await ipc.awaitDeviceLogin(prompt.user_code);
      }
      await onLoggedIn();
    } catch (e) {
      // Leaving the prompt up after a terminal failure would strand the user
      // on a code that can never be approved.
      setDevicePrompt(null);
      setLocalError(friendlyAuthError(e, mode));
    } finally {
      setSubmitting(false);
    }
  }

  const displayError = localError ?? error;
  const canSubmit =
    baseUrl.trim().length > 0 &&
    (mode === "password"
      ? email.length > 0 && password.length > 0
      : mode === "apiKey"
        ? apiKey.length > 0
        : true);

  return (
    <div className="flex h-full items-center justify-center p-6 overflow-y-auto">
      <div className="w-full max-w-sm card p-6">
        <div className="mb-5">
          <h1 className="text-lg font-semibold mb-1">
            Sign in to FunctionFly Vault
          </h1>
          <p className="text-xs text-neutral-500 dark:text-neutral-400">
            Your session token is stored on this machine — never in the vault
            itself.
          </p>
        </div>

        {/* Warn when the host has no OS keyring so the weaker storage is not
            a surprise discovered at the next sign-in. */}
        {backend === "file_fallback" && (
          <div className="mb-4 rounded-md border border-amber-300 dark:border-amber-800 bg-amber-50 dark:bg-amber-900/20 px-3 py-2 text-xs text-amber-900 dark:text-amber-200">
            <strong className="font-medium">No system keyring found.</strong>{" "}
            Your session is encrypted with a device-local key instead of the OS
            keychain — still encrypted at rest, but a profile with shell access
            can read it. On a desktop OS this message does not appear.
          </div>
        )}

        <form onSubmit={submit} className="space-y-4">
          {/* ── orchestrator ─────────────────────────────────────────────── */}
          <div>
            <div className="flex items-baseline justify-between gap-2">
              <label className="label mb-0" htmlFor="baseUrl">
                Orchestrator
              </label>
              {baseUrl.trim() !== PRODUCTION_ORCHESTRATOR && (
                <button
                  type="button"
                  onClick={() => setBaseUrl(PRODUCTION_ORCHESTRATOR)}
                  className="text-[11px] text-brand-600 hover:underline dark:text-brand-400"
                >
                  Use {PRODUCTION_ORCHESTRATOR.replace("https://", "")}
                </button>
              )}
            </div>
            <input
              id="baseUrl"
              className="input font-mono text-xs"
              type="url"
              required
              autoComplete="off"
              spellCheck={false}
              placeholder={PRODUCTION_ORCHESTRATOR}
              value={baseUrl}
              onChange={(e) => setBaseUrl(e.target.value)}
            />
            <div className="mt-1.5 flex items-center gap-2">
              <button
                type="button"
                onClick={testConnection}
                disabled={probe.phase === "checking" || !baseUrl.trim()}
                className="btn-secondary text-[11px] px-2 py-1"
              >
                {probe.phase === "checking" ? "Checking…" : "Test connection"}
              </button>
              {probe.phase === "ok" && (
                <span className="text-[11px] text-emerald-600 dark:text-emerald-400">
                  Reachable · {probe.latencyMs} ms
                </span>
              )}
            </div>
            {probe.phase === "fail" && (
              <p className="mt-1 text-[11px] text-red-600 dark:text-red-400">
                {probe.message}
              </p>
            )}
          </div>

          {/* ── credential mode ──────────────────────────────────────────── */}
          <div
            className="flex gap-1 rounded-md border border-neutral-300 dark:border-neutral-700 p-0.5 text-xs"
            role="tablist"
            aria-label="Sign-in method"
          >
            {(
              [
                ["browser", "Browser"],
                ["password", "Email + password"],
                ["apiKey", "API key"],
              ] as const
            ).map(([value, label]) => (
              <button
                key={value}
                type="button"
                role="tab"
                aria-selected={mode === value}
                onClick={() => setMode(value)}
                className={`flex-1 rounded px-2 py-1 ${
                  mode === value
                    ? "bg-brand-600 text-white"
                    : "text-neutral-500 hover:text-neutral-700 dark:hover:text-neutral-300"
                }`}
              >
                {label}
              </button>
            ))}
          </div>

          {mode === "browser" ? (
            /* Waiting-for-approval state wins over the form: once a code has
               been issued the only useful thing is to show it. */
            devicePrompt ? (
              <div
                className="space-y-3 rounded-md border border-neutral-300 dark:border-neutral-700 p-3"
                role="status"
                aria-live="polite"
              >
                <p className="text-xs text-neutral-600 dark:text-neutral-300">
                  Approve this code in your browser:
                </p>
                <p className="select-all text-center font-mono text-2xl tracking-[0.25em]">
                  {devicePrompt.user_code}
                </p>
                <a
                  className="block text-center text-xs text-brand-600 hover:underline dark:text-brand-400"
                  href={devicePrompt.verification_uri_complete}
                  target="_blank"
                  rel="noreferrer"
                >
                  Open the approval page
                </a>
                <p className="text-center text-[11px] text-neutral-500 dark:text-neutral-400">
                  Waiting for approval — this app will sign in on its own.
                </p>
              </div>
            ) : (
              <p className="text-xs text-neutral-500 dark:text-neutral-400">
                Sign in with your browser — no password is typed here. We'll
                show you a short code to confirm on the website.
              </p>
            )
          ) : mode === "password" ? (
            <>
              <div>
                <label className="label" htmlFor="email">
                  Email
                </label>
                <input
                  id="email"
                  className="input"
                  type="email"
                  required
                  autoComplete="username"
                  autoCapitalize="none"
                  spellCheck={false}
                  placeholder="you@company.com"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                />
              </div>
              <div>
                <div className="flex items-baseline justify-between gap-2">
                  <label className="label mb-0" htmlFor="password">
                    Password
                  </label>
                  <button
                    type="button"
                    onClick={() => setShowPassword((v) => !v)}
                    className="text-[11px] text-neutral-500 hover:underline"
                    aria-label={
                      showPassword ? "Hide password" : "Show password"
                    }
                  >
                    {showPassword ? "Hide" : "Show"}
                  </button>
                </div>
                <input
                  id="password"
                  className="input"
                  type={showPassword ? "text" : "password"}
                  required
                  autoComplete="current-password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                />
              </div>
            </>
          ) : (
            <div>
              <label className="label" htmlFor="apiKey">
                API key
              </label>
              <input
                id="apiKey"
                className="input font-mono text-xs"
                type="password"
                required
                autoComplete="off"
                spellCheck={false}
                placeholder="ffp_…"
                value={apiKey}
                onChange={(e) => setApiKey(e.target.value)}
              />
              <p className="text-[11px] text-neutral-500 dark:text-neutral-400 mt-1">
                Create one in the dashboard under{" "}
                <a
                  className="text-brand-600 hover:underline dark:text-brand-400"
                  href="https://app.functionfly.com/settings/api-keys"
                  target="_blank"
                  rel="noreferrer"
                >
                  Settings → API keys
                </a>
                .
              </p>
            </div>
          )}

          {displayError && (
            <div
              role="alert"
              className="rounded-md bg-red-50 dark:bg-red-900/30 border border-red-200 dark:border-red-800 px-3 py-2 text-xs text-red-800 dark:text-red-200"
            >
              <div className="flex items-start justify-between gap-2">
                <span>{displayError}</span>
                <button
                  type="button"
                  onClick={() => {
                    setLocalError(null);
                    onCloseError();
                  }}
                  className="text-red-500 hover:text-red-700 shrink-0"
                  aria-label="dismiss"
                >
                  ×
                </button>
              </div>
            </div>
          )}

          <button
            type="submit"
            disabled={submitting || !canSubmit}
            className="btn-primary w-full"
          >
            {submitting
              ? devicePrompt
                ? "Waiting for approval…"
                : "Signing in…"
              : mode === "browser"
                ? "Continue in browser"
                : "Sign in"}
          </button>
        </form>
      </div>
    </div>
  );
}

/** Rewrites raw IPC/auth errors into something actionable. The Rust core
 *  already returns coarse error kinds; this maps the cases users actually hit
 *  during onboarding — wrong password, unreachable host, MFA-only account —
 *  into copy that says what to do next. */
function friendlyAuthError(e: unknown, mode: AuthMode): string {
  const raw = messageFor(e);
  const lower = raw.toLowerCase();

  if (lower.includes("invalid") && lower.includes("credential")) {
    return mode === "password"
      ? "That email and password combination was not accepted. Check for typos, or use an API key instead."
      : mode === "browser"
        ? "The browser sign-in could not be completed. Start it again — if it keeps failing, use an API key instead."
        : "That API key was rejected. Copy it again from Settings → API keys — it may have been rotated.";
  }
  if (lower.includes("unauthenticated") || lower.includes("401")) {
    return mode === "password"
      ? "Sign-in was rejected. Check the email, password, and that the orchestrator URL above is the one you signed up against."
      : mode === "browser"
        ? "The browser sign-in was rejected. Confirm the URL above matches the site you signed up on, then start again."
        : "That API key was rejected by the orchestrator. Copy it again from Settings → API keys — it may have been rotated — and confirm the URL above matches the tenant that owns it.";
  }
  if (lower.includes("device_auth_disabled")) {
    return "This orchestrator does not offer browser sign-in. Use an API key instead (Settings → API keys).";
  }
  if (lower.includes("denied") || lower.includes("expired") || lower.includes("no longer valid")) {
    return mode === "browser"
      ? "That sign-in request ended before it was approved. Start again from this screen."
      : raw;
  }
  if (lower.includes("rate") && lower.includes("limit")) {
    return "Too many attempts from this machine. Wait a minute and try again.";
  }
  if (
    lower.includes("mfa") ||
    lower.includes("two-factor") ||
    lower.includes("2fa")
  ) {
    return "This account requires two-factor authentication, which the desktop client does not support yet. Use an API key instead.";
  }
  if (
    lower.includes("connect") ||
    lower.includes("dns") ||
    lower.includes("timed out")
  ) {
    return "Could not reach that orchestrator. Check the URL and your connection — the “Test connection” button above will confirm.";
  }
  if (lower.includes("not enabled") || lower.includes("plan_denied")) {
    return "This account does not have the Vault feature enabled. Ask your FunctionFly admin to enable it on your plan.";
  }
  return raw;
}

function messageFor(e: unknown): string {
  if (e instanceof VaultIpcError) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}
