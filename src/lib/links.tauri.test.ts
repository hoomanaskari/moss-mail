import { expect, it, vi } from "vitest";

const openUrl = vi.fn(async () => {});
vi.mock("./platform", () => ({ isTauri: true }));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl }));

const { openLink } = await import("./links");

it("in the desktop app, links open in the default browser", async () => {
  const open = vi.spyOn(window, "open").mockImplementation(() => null);
  openLink("https://example.com/a");
  await vi.waitFor(() => expect(openUrl).toHaveBeenCalledWith("https://example.com/a"));
  openLink("javascript:alert(1)");
  expect(openUrl).toHaveBeenCalledOnce();
  expect(open).not.toHaveBeenCalled();
  open.mockRestore();
});
