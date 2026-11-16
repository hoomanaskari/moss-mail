import { afterEach, describe, expect, it, vi } from "vitest";

const close = vi.fn();
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => ({ close }) }));

async function load(tauri: boolean) {
  vi.resetModules();
  vi.doMock("./platform", () => ({ isTauri: tauri }));
  return import("./window");
}

afterEach(() => {
  close.mockClear();
  vi.doUnmock("./platform");
});

describe("closeWindow", () => {
  it("closes the desktop window", async () => {
    await (await load(true)).closeWindow();
    expect(close).toHaveBeenCalled();
  });

  it("does nothing in a browser tab", async () => {
    await (await load(false)).closeWindow();
    expect(close).not.toHaveBeenCalled();
  });
});
