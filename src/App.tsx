import { useCallback, useEffect, useRef, useState } from "react";
import { ipc, type VaultStatus } from "./lib/ipc";
import { VaultIpcError } from "./lib/tauri";
import { EVENT_VAULT_LOCKED, listen } from "./lib/events";
import { LoginScreen } from "./screens/LoginScreen";
import { SecretsScreen } from "./screens/SecretsScreen";
import { UnlockScreen } from "./screens/UnlockScreen";

type Phase = "loading" | "needs-login" | "needs-unlock" | "ready";

/**
 * How often the webview tells the core that the user is still present.
 *
 * The core owns the countdown; this is only the heartbeat. It is
 * deliberately coarse — a one-minute heartbeat means the vault locks at
 * most a minute late, and the cost is one IPC call a minute.
 */
const ACTIVITY_HEARTBEAT_MS = 60_000;

export function App() {
  const [phase, setPhase] = useState<Phase>("loading");
  const [status, setStatus] = useState<VaultStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The timestamp of the last user interaction, so the heartbeat can be
  // skipped when nothing has happened.
  const lastInteraction = useRef<number>(Date.now());

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

  // Report activity to the Rust core so it can decide when the vault has
  // been idle for too long.
  //
  // The core owns the countdown on purpose: if the timer lived here, a
  // compromised renderer could simply not run it.
  const reportActivity = useCallback(() => {
    lastInteraction.current = Date.now();
    if (phase === "ready") {
      void ipc.reportActivity().catch(() => {
        // The core may have locked between the phase check and the call.
        // The next heartbeat will pick the new state up.
      });
    }
  }, [phase]);

  useEffect(() => {
    if (phase !== "ready") return;

    const events: Array<keyof WindowEventMap> = [
      "mousedown",
      "keydown",
      "wheel",
      "touchstart",
    ];
    for (const evt of events) {
      window.addEventListener(evt, reportActivity, { passive: true });
    }

    // The heartbeat. Every minute, if the user has done something since
    // the last beat, tell the core. If they have not, the core's own
    // countdown decides the vault locks.
    const timer = setInterval(() => {
      const idleFor = Date.now() - lastInteraction.current;
      if (idleFor >= ACTIVITY_HEARTBEAT_MS) {
        void ipc.reportActivity().catch(() => {});
        lastInteraction.current = Date.now();
      }
    }, ACTIVITY_HEARTBEAT_MS);

    // The core locks the vault when the idle period elapses and emits
    // this event. React to it by dropping back to the unlock screen.
    let unlisten: (() => void) | null = null;
    let cancelled = false;
    void listen(EVENT_VAULT_LOCKED, () => {
      void refresh();
    }).then((fn) => {
      if (cancelled) {
        fn();
        return;
      }
      unlisten = fn;
    });

    return () => {
      for (const evt of events) {
        window.removeEventListener(evt, reportActivity);
      }
      clearInterval(timer);
      unlisten?.();
    };
  }, [phase, reportActivity, refresh]);

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
