import { beforeEach, expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { resetStore } from "../test/fixtures";
import { useStore } from "../state/store";
import { refreshCredentialCleanup, resetCredentialCleanup, showCredentialCleanup, useCredentialCleanup } from "./credentialCleanup";
const s = () => useCredentialCleanup.getState();
beforeEach(() => resetStore([]));
it("shows durable pending work, retries it, and coalesces repeated retry clicks", async () => {
  let finish!: () => void;
  const { backend } = fakeMail({ credentialCleanupStatus: vi.fn(async () => ({ pending: true })), retryCredentialCleanup: vi.fn(() => new Promise<{ pending: boolean }>(resolve => { finish = () => resolve({ pending: false }); })) });
  await refreshCredentialCleanup();
  expect(s()).toEqual({ pending: true, busy: false, unavailable: false });
  const retry = refreshCredentialCleanup(true);
  await refreshCredentialCleanup(true);
  expect(backend.retryCredentialCleanup).toHaveBeenCalledOnce();
  expect(s().busy).toBe(true);
  finish(); await retry;
  expect(s()).toEqual({ pending: false, busy: false, unavailable: false });
});
it.each([undefined, { pending: "no" }])("does not treat malformed status as successful cleanup: %j", async value => {
  fakeMail({ credentialCleanupStatus: vi.fn().mockResolvedValue(value) });
  await refreshCredentialCleanup();
  expect(s()).toEqual({ pending: true, busy: false, unavailable: true });
});
it("reports unavailable status without exposing backend error details", async () => {
  fakeMail({ credentialCleanupStatus: vi.fn().mockRejectedValue(new Error("private fixture")) });
  await refreshCredentialCleanup();
  expect(s()).toEqual({ pending: true, busy: false, unavailable: true });
});
it.each([false, true])("ignores old success/failure after a newer removal result (%s)", async reject => {
  let finish!: () => void;
  fakeMail({ credentialCleanupStatus: vi.fn(() => new Promise<{ pending: boolean }>((resolve, fail) => { finish = () => reject ? fail(new Error("old")) : resolve({ pending: false }); })) });
  const request = refreshCredentialCleanup();
  showCredentialCleanup({ pending: true });
  finish(); await request;
  expect(s()).toEqual({ pending: true, busy: false, unavailable: false });
});
it("reset invalidates a pending status from the previous live session", async () => {
  let finish!: () => void;
  fakeMail({ credentialCleanupStatus: vi.fn(() => new Promise<{ pending: boolean }>(resolve => { finish = () => resolve({ pending: true }); })) });
  const request = refreshCredentialCleanup(); resetCredentialCleanup(); finish(); await request;
  expect(s()).toEqual({ pending: false, busy: false, unavailable: false });
});
it("removes the account while retaining a pending credential cleanup warning", async () => {
  fakeMail({ removeAccount: vi.fn(async () => ({ pending: true })) });
  expect(await useStore.getState().removeAccount("work")).toBe(true);
  expect(useStore.getState().accounts.some(a => a.id === "work")).toBe(false);
  expect(s().pending).toBe(true);
});
it("a late successful removal cannot hide cleanup pending from another account", async () => {
  let finishWork!: (value: { pending: boolean }) => void;
  let finishPersonal!: (value: { pending: boolean }) => void;
  fakeMail({
    removeAccount: vi.fn(id => new Promise<{ pending: boolean }>(resolve => {
      if (id === "work") finishWork = resolve;
      else finishPersonal = resolve;
    })),
    credentialCleanupStatus: vi.fn(async () => ({ pending: true })),
  });
  const work = useStore.getState().removeAccount("work");
  const personal = useStore.getState().removeAccount("personal");
  finishPersonal({ pending: true }); await personal;
  expect(s().pending).toBe(true);
  finishWork({ pending: false }); await work;
  expect(s().pending).toBe(true);
  expect(useStore.getState().accounts).toEqual([]);
});
