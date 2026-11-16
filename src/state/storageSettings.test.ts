import { expect, it, vi } from "vitest";
import { demoStorage } from "../services/mail";
import { fakeMail } from "../test/fakeMail";
import { useStorageSettings } from "./storageSettings";

const s = () => useStorageSettings.getState();

it("loads the figures and reloads them after each change", async () => {
  const { backend } = fakeMail();
  await s().load();
  expect(s().info).toEqual(demoStorage);
  await s().setOffline("recent");
  expect(backend.setOffline).toHaveBeenCalledWith("recent");
  await s().setCacheLimit(1_000_000_000);
  expect(backend.setCacheLimit).toHaveBeenCalledWith(1_000_000_000);
  await s().clearCache();
  expect(backend.clearCache).toHaveBeenCalledOnce();
  await s().rebuildIndex();
  expect(backend.rebuildIndex).toHaveBeenCalledOnce();
  await s().setAttachmentText(false);
  expect(backend.setAttachmentText).toHaveBeenCalledWith(false);
  expect(backend.storage).toHaveBeenCalledTimes(6);
});

it("a failure becomes a sentence on screen, and the next success clears it", async () => {
  fakeMail({ clearCache: vi.fn(() => Promise.reject({ kind: "store", message: "The disk is full." })) });
  await s().clearCache();
  expect(s().problem).toBe("The disk is full.");
  await s().load();
  expect(s().problem).toBeUndefined();
});

it("a new setting shows at once, and goes back if the engine refuses it", async () => {
  let refuse: (error: unknown) => void = () => {};
  fakeMail({ setOffline: vi.fn(() => new Promise<void>((_, r) => (refuse = r))), setCacheLimit: vi.fn(() => new Promise<void>(() => {})) });
  // Nothing to show yet: nothing changes on screen.
  void s().setCacheLimit(1_000_000_000);
  expect(s().info).toBeUndefined();
  await s().load();
  const setting = s().setOffline("recent");
  expect(s().info?.offline).toBe("recent");
  refuse({ kind: "store", message: "The disk is full." });
  await setting;
  expect(s().info?.offline).toBe(demoStorage.offline);
  expect(s().problem).toBe("The disk is full.");
  void s().setCacheLimit(1_000_000_000);
  expect(s().info?.cacheLimit).toBe(1_000_000_000);
});
