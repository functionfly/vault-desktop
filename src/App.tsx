import { useCallback, useEffect, useState } from "react";
import { ipc, type VaultStatus } from "./lib/ipc";
import { VaultIpcError } from "./lib/tauri";
import { LoginScreen } from "./screens/LoginScreen";
import { SecretsScreen } from "./screens/SecretsScreen";
import { UnlockScreen } from "./screens/UnlockScreen";

type Phase = "loading" | "needs-login" | "needs-unlock" | "ready";

export function App() {
  const [phase, setPhase] = useState<Phase>("loading");
  const [status, setStatus] = useState<VaultStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const restore = await ipc.restoreSession();
      if (!restore.user) {
        setStatus(null);
        setPhase("needs-login");
        return;
      }
      const s = await ipc.vaultStatus();
      setStatus(s);
      if (!s.unlocked) {
        setPhase("needs-unlock");
      } else {
        setPhase("ready");
      }
    } catch (e) {
      // First launch with no keychain entry lands here — that's normal.
      setError(messageFor(e));
      setPhase("needs-login");
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  if (phase === "loading") {
    return (
      <div className="flex h-full items-center justify-center text-sm text-neutral-500">
        Loading…
      </div>
    );
  }

  if (phase === "needs-login") {
    return (
      <LoginScreen
        error={error}
        onLoggedIn={async () => {
          await refresh();
        }}
        onCloseError={() => setError(null)}
      />
    );
  }

  if (phase === "needs-unlock" || !status) {
    return (
      <UnlockScreen
        userEmail={status?.user?.email ?? null}
        baseUrl={status?.base_url ?? null}
        storeBackend={status?.store_backend ?? null}
        onUnlocked={async () => {
          await refresh();
        }}
        onSignOut={async () => {
          await ipc.logout();
          setPhase("needs-login");
        }}
      />
    );
  }

  return (
    <SecretsScreen
      status={status}
      onLock={async () => {
        await ipc.lockVault();
        setPhase("needs-unlock");
      }}
      onSignOut={async () => {
        await ipc.logout();
        setPhase("needs-login");
      }}
    />
  );
}

function messageFor(e: unknown): string {
  if (e instanceof VaultIpcError) return e.message;
  if (e instanceof Error) return e.message;
  return String(e);
}
