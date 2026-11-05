// The offline banner.
//
// When the orchestrator cannot be reached, the secrets list is empty and
// the user has no way to tell that apart from "you have no secrets".
// This banner says which it is, and shows the cached metadata so the
// inventory is still visible.
//
// The cache holds METADATA only. No ciphertext, no plaintext. So the
// banner can list what you have, but it cannot — and does not — show any
// value.

import { useCallback, useEffect, useState } from "react";
import { ipc, type MetadataCacheResult } from "../lib/ipc";

interface Props {
  /** True when the last list attempt failed because the host was unreachable. */
  offline: boolean;
  /** The error from the failed attempt, if any. */
  error: string | null;
}

export function OfflineBanner({ offline, error }: Props) {
  const [cache, setCache] = useState<MetadataCacheResult | null>(null);

  const loadCache = useCallback(async () => {
    try {
      setCache(await ipc.readMetadataCache());
    } catch {
      // The cache is a convenience; a failure to read it is not worth
      // another error banner.
      setCache(null);
    }
  }, []);

  useEffect(() => {
    if (offline) {
      void loadCache();
    }
  }, [offline, loadCache]);

  if (!offline) return null;

  const cached = cache?.available ? cache.secrets ?? [] : [];
  const cachedAt = cache?.cached_at
    ? new Date(cache.cached_at).toLocaleString()
    : null;

  return (
    <div className="border-b border-amber-200 dark:border-amber-800 bg-amber-50 dark:bg-amber-900/20 px-3 py-2 text-xs text-amber-900 dark:text-amber-100">
      <div className="flex items-start gap-2">
        <strong className="flex-shrink-0">Offline.</strong>
        <div className="min-w-0">
          {error && <div className="break-all">{error}</div>}
          {cached.length > 0 ? (
            <>
              <div className="mt-1">
                Showing {cached.length} cached secret
                {cached.length === 1 ? "" : "s"} from{" "}
                {cachedAt ?? "an earlier session"}. Metadata only — values
                need the orchestrator.
              </div>
              <ul className="mt-1 flex flex-wrap gap-1">
                {cached.slice(0, 40).map((s) => (
                  <li
                    key={s.id}
                    className="rounded bg-white/70 dark:bg-neutral-950/50 px-1.5 py-0.5 font-mono text-[11px]"
                    title={`${s.secret_type} · ${s.namespace}`}
                  >
                    {s.name}
                  </li>
                ))}
                {cached.length > 40 && (
                  <li className="text-[11px] px-1.5 py-0.5">
                    +{cached.length - 40} more
                  </li>
                )}
              </ul>
            </>
          ) : (
            <div className="mt-1">
              No cached metadata yet. The cache is written the next time
              the vault lists your secrets successfully.
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
