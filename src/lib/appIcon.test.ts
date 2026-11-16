import { afterEach, describe, expect, it, vi } from "vitest";

const invoke = vi.fn(async () => {});
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

async function load(tauri: boolean) {
  vi.resetModules();
  vi.doMock("./platform", () => ({ isTauri: tauri }));
  return import("./appIcon");
}

afterEach(() => {
  invoke.mockClear();
  vi.doUnmock("./platform");
});

describe("appIcon", () => {
  it("leaves the icon to the Mac while Moss follows the system", async () => {
    const { appIcon } = await load(false);
    expect(appIcon("system")).toBe("system");
  });

  // The Mac's icon style (Settings → Appearance) can keep icons dark in Light mode, so a fixed
  // theme always shows its own look rather than guessing what the Mac shows.
  it("shows Moss's look whenever its theme is fixed", async () => {
    const { appIcon } = await load(false);
    expect(appIcon("light")).toBe("light");
    expect(appIcon("dark")).toBe("dark");
  });
});

describe("setAppIcon", () => {
  it("asks the desktop app to show that icon in the Dock", async () => {
    await (await load(true)).setAppIcon("dark");
    expect(invoke).toHaveBeenCalledWith("set_app_icon", { icon: "dark" });
  });

  it("does nothing in a browser tab", async () => {
    await (await load(false)).setAppIcon("dark");
    expect(invoke).not.toHaveBeenCalled();
  });

  it("keeps the icon it had when the app can't change it", async () => {
    invoke.mockRejectedValueOnce(new Error("no Dock"));
    await expect((await load(true)).setAppIcon("light")).resolves.toBeUndefined();
  });
});
