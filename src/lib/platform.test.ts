import { afterEach, describe, expect, it, vi } from "vitest";

async function load({ tauri, userAgent }: { tauri: boolean; userAgent: string }) {
  vi.resetModules();
  if (tauri) Object.assign(window, { __TAURI_INTERNALS__: {} });
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue(userAgent);
  return import("./platform");
}

afterEach(() => {
  delete (window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  delete document.documentElement.dataset.titlebar;
  vi.restoreAllMocks();
});

describe("platform", () => {
  it("in a browser tab, leaves the title bar alone", async () => {
    const p = await load({ tauri: false, userAgent: "Mozilla/5.0 (Macintosh)" });
    expect(p.isTauri).toBe(false);
    expect(p.deviceName).toBe("Mac");
    expect(p.modKey).toBe("Meta");
    p.markNativeTitleBar();
    expect(document.documentElement.dataset.titlebar).toBeUndefined();
  });

  it("in the Mac app, makes room for the traffic lights", async () => {
    const p = await load({ tauri: true, userAgent: "Mozilla/5.0 (Macintosh)" });
    p.markNativeTitleBar();
    expect(document.documentElement.dataset.titlebar).toBe("mac");
  });

  it("on Windows and Linux, uses the native title bar and says “computer”", async () => {
    const p = await load({ tauri: true, userAgent: "Mozilla/5.0 (Windows NT 10.0)" });
    expect(p.deviceName).toBe("computer");
    expect(p.modKey).toBe("Control");
    p.markNativeTitleBar();
    expect(document.documentElement.dataset.titlebar).toBe("native");
  });
});
