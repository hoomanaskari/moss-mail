import { afterEach, expect, it, vi } from "vitest";
import { installedMoss, mossDev, useAppIdentity } from "./appIdentity";

const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...args: unknown[]) => invoke(...args) }));
const { loadAppIdentity } = await import("./tauriIdentity");

afterEach(() => useAppIdentity.setState(installedMoss));

it("is the installed Moss until the desktop app says otherwise", () => {
  expect(useAppIdentity.getState()).toEqual({ name: "Moss", dev: false, updates: true, agentName: "moss" });
});

it("learns from the desktop app that it is Moss Dev", async () => {
  invoke.mockResolvedValue(mossDev);
  await loadAppIdentity();
  expect(invoke).toHaveBeenCalledWith("app_identity");
  expect(useAppIdentity.getState()).toEqual({ name: "Moss Dev", dev: true, updates: false, agentName: "moss-dev" });
});
