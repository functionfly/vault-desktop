// Clipboard auto-clear contract.
//
// The security property under test: a secret copied to the clipboard is
// wiped, and the wipe does NOT destroy something the user copied
// afterwards.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  cancelPendingClear,
  clearIfUnchanged,
  CLIPBOARD_CLEAR_AFTER_MS,
  copySecret,
} from "./clipboard";

let clipboard = "";
let writeMock: ReturnType<typeof vi.fn>;
let readMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  clipboard = "";
  writeMock = vi.fn(async (text: string) => {
    clipboard = text;
  });
  readMock = vi.fn(async () => clipboard);

  Object.defineProperty(globalThis.navigator, "clipboard", {
    value: { writeText: writeMock, readText: readMock },
    configurable: true,
    writable: true,
  });

  vi.useFakeTimers();
});

afterEach(() => {
  cancelPendingClear();
  vi.useRealTimers();
});

describe("copySecret", () => {
  it("writes the value to the clipboard", async () => {
    await copySecret("hunter2");
    expect(clipboard).toBe("hunter2");
    expect(writeMock).toHaveBeenCalledWith("hunter2");
  });

  it("clears the clipboard after the timeout", async () => {
    await copySecret("hunter2");
    expect(clipboard).toBe("hunter2");

    await vi.advanceTimersByTimeAsync(CLIPBOARD_CLEAR_AFTER_MS + 100);

    expect(clipboard).toBe("");
  });

  it("does not clear the clipboard before the timeout", async () => {
    await copySecret("hunter2");
    await vi.advanceTimersByTimeAsync(CLIPBOARD_CLEAR_AFTER_MS - 1000);
    expect(clipboard).toBe("hunter2");
  });

  it("does not destroy a value the user copied afterwards", async () => {
    await copySecret("hunter2");

    // The user copies something else while the wipe is pending.
    clipboard = "something else entirely";
    await vi.advanceTimersByTimeAsync(CLIPBOARD_CLEAR_AFTER_MS + 100);

    expect(clipboard).toBe("something else entirely");
  });

  it("cancels the pending wipe when the handle is cancelled", async () => {
    const handle = await copySecret("hunter2");
    handle.cancel();
    await vi.advanceTimersByTimeAsync(CLIPBOARD_CLEAR_AFTER_MS + 100);

    // The value was copied, but the user hid the revealed secret, so the
    // clipboard is left alone rather than being wiped behind their back.
    expect(clipboard).toBe("hunter2");
  });

  it("a second copy replaces the first wipe", async () => {
    await copySecret("first");
    await copySecret("second");
    await vi.advanceTimersByTimeAsync(CLIPBOARD_CLEAR_AFTER_MS + 100);

    expect(clipboard).toBe("");
  });
});

describe("clearIfUnchanged", () => {
  it("wipes the clipboard when the value is still ours", async () => {
    clipboard = "hunter2";
    const cleared = await clearIfUnchanged("hunter2");
    expect(cleared).toBe(true);
    expect(clipboard).toBe("");
  });

  it("leaves the clipboard alone when the value differs", async () => {
    clipboard = "different";
    const cleared = await clearIfUnchanged("hunter2");
    expect(cleared).toBe(false);
    expect(clipboard).toBe("different");
  });

  it("wipes anyway when the clipboard cannot be read", async () => {
    // Failing to read is not evidence that the clipboard holds something
    // else — it is evidence that we cannot check. Leaving a secret on the
    // clipboard because the browser denied a read is the worse failure.
    readMock.mockRejectedValueOnce(new Error("permission denied"));
    const cleared = await clearIfUnchanged("hunter2");
    expect(cleared).toBe(true);
    expect(writeMock).toHaveBeenCalledWith("");
  });
});
