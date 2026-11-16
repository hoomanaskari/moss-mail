import { beforeEach, expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { draft, message, personal, resetStore, thread, work } from "../test/fixtures";
import { useDrafts } from "./drafts";
import { useStore } from "./store";
const s = () => useStore.getState();
beforeEach(() => {
  resetStore([thread("work-mail"), thread("personal-mail", { accountId: "personal" })]);
  useDrafts.setState({ drafts: [] });
});

it("keeps the account and mail until the backend accepts removal, and coalesces repeated clicks", async () => {
  let finish!: () => void;
  const { backend } = fakeMail({ removeAccount: vi.fn(() => new Promise<{ pending: boolean }>(resolve => { finish = () => resolve({ pending: false }); })) });
  s().setAccountFilter("work");
  const pending = s().removeAccount("work");
  expect(s().accounts).toContainEqual(work);
  expect(s().threads.map(t => t.id)).toEqual(["work-mail", "personal-mail"]);
  expect(s().accountFilter).toBe("work");
  expect(s()).toMatchObject({ accountRemovals: { work: { pending: true } } });
  await s().removeAccount("work");
  expect(backend.removeAccount).toHaveBeenCalledOnce();
  finish();
  expect(await pending).toBe(true);
  expect(s().accounts).toEqual([personal]);
  expect(s().accountFilter).toBe("all");
  expect(s()).toMatchObject({ accountRemovals: {} });
});

it("keeps all mail and shows refusal, then allows a deliberate retry", async () => {
  const { backend } = fakeMail({ removeAccount: vi.fn().mockRejectedValueOnce({ message: "Resolve pending server changes first." }).mockResolvedValue({ pending: false }) });
  s().setAccountFilter("work");
  expect(await s().removeAccount("work")).toBe(false);
  expect(s().accounts).toContainEqual(work);
  expect(s().threads).toHaveLength(2);
  expect(s().accountFilter).toBe("work");
  expect(s()).toMatchObject({ accountRemovals: { work: { pending: false, error: "Resolve pending server changes first." } } });
  expect(await s().removeAccount("work")).toBe(true);
  expect(backend.removeAccount).toHaveBeenCalledTimes(2);
});

it("clears the removed account from loaded lists and saved drafts only after success", async () => {
  fakeMail();
  const old = thread("old", { messages: [message()] });
  const other = thread("other", { accountId: "personal" });
  useStore.setState({ doneThreads: [old, other], blockedThreads: [old, other], foundThreads: [old, other] });
  useDrafts.setState({ drafts: [draft("work-draft"), draft("other-draft", { fromAccountId: "personal" })] });
  expect(await s().removeAccount("work")).toBe(true);
  expect(s().doneThreads).toEqual([other]);
  expect(s().blockedThreads).toEqual([other]);
  expect(s().foundThreads).toEqual([other]);
  expect(useDrafts.getState().drafts.map(d => d.id)).toEqual(["other-draft"]);
});

it.each([false, true])("ignores a stale removal response after reset (reject=%s)", async (reject) => {
  let settle!: (value?: unknown) => void;
  fakeMail({ removeAccount: vi.fn(() => new Promise<{ pending: boolean }>((resolve, fail) => { settle = reject ? fail : () => resolve({ pending: false }); })) });
  const pending = s().removeAccount("work");
  resetStore([thread("new-session")]);
  settle(new Error("old failure"));
  expect(await pending).toBe(false);
  expect(s().accounts).toContainEqual(work);
  expect(s().threads.map(t => t.id)).toEqual(["new-session"]);
  expect(s()).toMatchObject({ accountRemovals: {} });
});

it("does not remove a replacement account when an old response arrives", async () => {
  let finish!: () => void;
  fakeMail({ removeAccount: vi.fn(() => new Promise<{ pending: boolean }>(resolve => { finish = () => resolve({ pending: false }); })) });
  const pending = s().removeAccount("work");
  useStore.setState({ accounts: [personal] });
  s().addAccount({ ...work, name: "Replacement" });
  finish();
  expect(await pending).toBe(false);
  expect(s().accounts.find(a => a.id === "work")?.name).toBe("Replacement");
});

it("does not ask the backend to remove an absent account", async () => {
  const { backend } = fakeMail();
  expect(await s().removeAccount("missing")).toBe(false);
  expect(backend.removeAccount).not.toHaveBeenCalled();
});

it("says how many server changes would be lost, and removes the account with them only when asked", async () => {
  const lost = { kind: "unsentServerChanges", count: 3, message: "3 changes haven't reached the server and will be lost." };
  const { backend } = fakeMail({ removeAccount: vi.fn().mockRejectedValueOnce(lost).mockResolvedValue({ pending: false }) });
  expect(await s().removeAccount("work")).toBe(false);
  expect(backend.removeAccount).toHaveBeenLastCalledWith("work");
  expect(s().accounts).toContainEqual(work);
  expect(s()).toMatchObject({ accountRemovals: { work: { pending: false, error: lost.message, unsentChanges: 3 } } });
  expect(await s().removeAccount("work", 3)).toBe(true);
  expect(backend.removeAccount).toHaveBeenLastCalledWith("work", 3);
  expect(s().accounts).toEqual([personal]);
});

it.each([
  { kind: "unsentServerChanges", message: "No count" },
  { kind: "unsentServerChanges", count: 0, message: "Nothing to lose" },
  { kind: "unsentServerChanges", count: 1.5, message: "Not a count" },
  { kind: "invalid", count: 2, message: "Another refusal" },
])("only a real count of lost changes offers removing anyway %#", async (error) => {
  fakeMail({ removeAccount: vi.fn().mockRejectedValue(error) });
  expect(await s().removeAccount("work")).toBe(false);
  expect(s().accountRemovals.work).toEqual({ token: expect.anything(), pending: false, error: error.message });
});
