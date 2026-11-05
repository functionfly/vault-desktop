// Clipboard handling for revealed secrets.
//
// A secret copied to the clipboard and never cleared is the most common way
// a secrets manager leaks: the value sits in the OS clipboard, readable by
// every other application on the machine, long after the user closed the
// window.
//
// So every copy of a secret schedules a wipe. The wipe is best-effort —
// the browser clipboard API cannot prove the clipboard was not changed by
// something else in the meantime — so the implementation compares what it
// wrote against what is there now, and only overwrites if it is still ours.
//
// The timer is tracked so it can be cancelled when the value is hidden or
// the component unmounts. Without that, a pending wipe could fire after
// the user copied something else entirely and destroy their clipboard.

/** How long a copied secret stays on the clipboard. */
export const CLIPBOARD_CLEAR_AFTER_MS = 30_000;

/** How often the wipe checks whether the clipboard still holds our value. */
const POLL_INTERVAL_MS = 1_000;

export interface ClipboardHandle {
  /** Cancels the pending wipe. Safe to call more than once. */
  cancel: () => void;
}

let activeTimer: ReturnType<typeof setInterval> | null = null;
let activeTimeout: ReturnType<typeof setTimeout> | null = null;

/**
 * Cancels any pending clipboard wipe.
 *
 * Call this before copying something that is NOT a secret, so the pending
 * wipe does not destroy it.
 */
export function cancelPendingClear(): void {
  if (activeTimer !== null) {
    clearInterval(activeTimer);
    activeTimer = null;
  }
  if (activeTimeout !== null) {
    clearTimeout(activeTimeout);
    activeTimeout = null;
  }
}

/**
 * Copies `text` to the clipboard and schedules it to be wiped.
 *
 * Returns a handle that cancels the wipe. The caller should cancel it when
 * the revealed value is hidden or the component unmounts.
 */
export async function copySecret(
  text: string,
  afterMs: number = CLIPBOARD_CLEAR_AFTER_MS,
): Promise<ClipboardHandle> {
  cancelPendingClear();
  await navigator.clipboard.writeText(text);

  const started = Date.now();
  const tick = async () => {
    const elapsed = Date.now() - started;
    if (elapsed >= afterMs) {
      await clearIfUnchanged(text);
      cancelPendingClear();
      return;
    }
    // The clipboard may have been overwritten by another app in the
    // meantime. If so, leave it alone — it is no longer our value, and
    // clobbering it would destroy the user's data.
    const current = await readClipboardSafe();
    if (current !== null && current !== text) {
      cancelPendingClear();
    }
  };

  activeTimer = setInterval(() => {
    void tick();
  }, POLL_INTERVAL_MS);
  activeTimeout = setTimeout(() => {
    void clearIfUnchanged(text);
    cancelPendingClear();
  }, afterMs);

  return {
    cancel: () => {
      cancelPendingClear();
    },
  };
}

/**
 * Overwrites the clipboard with empty text, but only if it still holds
 * `expected`.
 *
 * The comparison matters: if the user copied something else after the
 * secret, wiping blindly would destroy their data.
 *
 * When the clipboard CANNOT be read, the wipe happens anyway. Failing to
 * read is not evidence that the clipboard holds something else — it is
 * evidence that we cannot check — and leaving a secret on the clipboard
 * because the browser denied a read is the worse failure of the two.
 */
export async function clearIfUnchanged(expected: string): Promise<boolean> {
  const current = await readClipboardSafe();
  if (current !== null && current !== expected) {
    // The clipboard holds something else now. Leave it alone — it is the
    // user's data, not ours.
    return false;
  }
  try {
    await navigator.clipboard.writeText("");
    return true;
  } catch {
    return false;
  }
}

async function readClipboardSafe(): Promise<string | null> {
  try {
    return await navigator.clipboard.readText();
  } catch {
    // Reading the clipboard can be denied by permissions. Treat it as
    // "unknown" rather than assuming it is unchanged.
    return null;
  }
}
