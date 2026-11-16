import { beforeEach, expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { resetStore, work } from "../test/fixtures";
import { useStore } from "../state/store";
import { handleEvent } from "./live";
import type { ServerChangeStatus } from "./mail";
import { discardServerChange, refreshServerChanges, resetServerChanges, retryServerChange, useServerChanges } from "./serverChanges";

export const retained: ServerChangeStatus = {
  accountId: "work", change: "0123456789abcdef0123456789abcdef", revision: 3, state: "blocked", failure: "uncertain", kind: "move",
  completed: 1, pending: 2, discarded: 0, dropped: 0,
};
beforeEach(() => { resetStore([]); useStore.setState({ accounts: [work] }); resetServerChanges(); });
it("keeps durable recovery when ordinary inbox sync recovers", async () => {
  fakeMail({ serverChangeStatus: vi.fn(async () => retained) });
  await refreshServerChanges("work");
  handleEvent({ type: "syncOk", accountId: "work" });
  expect(useServerChanges.getState().byAccount.work).toEqual({ status: retained, unavailable: false });
});
it("ignores an older response after the latest recovery status arrives", async () => {
  let finish!: (value: ServerChangeStatus | null) => void;
  const get = vi.fn().mockImplementationOnce(() => new Promise<ServerChangeStatus | null>((resolve) => { finish = resolve; })).mockResolvedValueOnce(null);
  fakeMail({ serverChangeStatus: get });
  const older = refreshServerChanges("work");
  await refreshServerChanges("work");
  finish(retained); await older;
  expect(useServerChanges.getState().byAccount.work).toEqual({ status: null, unavailable: false });
});
it.each(["reset", "remove"])("ignores in-flight status after account %s", async (action) => {
  let finish!: (value: ServerChangeStatus | null) => void;
  fakeMail({ serverChangeStatus: vi.fn(() => new Promise<ServerChangeStatus | null>((resolve) => { finish = resolve; })) });
  const pending = refreshServerChanges("work");
  if (action === "reset") resetServerChanges(); else useStore.setState({ accounts: [] });
  finish(retained); await pending;
  expect(useServerChanges.getState().byAccount).toEqual({});
});
it("retains known recovery and shows unavailable on a failed refresh without exposing raw errors", async () => {
  const get = vi.fn().mockResolvedValueOnce(retained).mockRejectedValueOnce(new Error("private provider details"));
  fakeMail({ serverChangeStatus: get });
  await refreshServerChanges("work"); await refreshServerChanges("work");
  expect(useServerChanges.getState().byAccount.work).toEqual({ status: retained, unavailable: true });
  expect(JSON.stringify(useServerChanges.getState())).not.toContain("private provider details");
});
it("does not query an account that is no longer present", async () => {
  const { backend } = fakeMail();
  await refreshServerChanges("missing");
  expect(backend.serverChangeStatus).not.toHaveBeenCalled();
});
it("does not let an old error replace a newer successful status", async () => {
  let fail!: (reason: Error) => void;
  const get = vi.fn().mockImplementationOnce(() => new Promise((_, reject) => { fail = reject; })).mockResolvedValueOnce(retained);
  fakeMail({ serverChangeStatus: get });
  const older = refreshServerChanges("work"); await refreshServerChanges("work");
  fail(new Error("old")); await older;
  expect(useServerChanges.getState().byAccount.work).toEqual({ status: retained, unavailable: false });
});
it.each([
  undefined, false, "invalid", { ...retained, accountId: "another" }, { ...retained, state: "future" },
  { ...retained, kind: "future" }, { ...retained, completed: -1 }, { ...retained, completed: 0.5 },
  { ...retained, completed: 65 }, { ...retained, completed: 64 }, { ...retained, pending: 0 },
  { ...retained, failure: null }, { ...retained, state: "claimed" },
  { ...retained, change: "0123456789ABCDEF0123456789ABCDEF" }, { ...retained, change: "0123" }, { ...retained, change: 7 },
  { ...retained, revision: -1 }, { ...retained, revision: 1.5 }, { ...retained, revision: "3" },
])("shows unavailable for invalid recovery metadata %#", async (value) => {
  fakeMail({ serverChangeStatus: vi.fn(async () => value as ServerChangeStatus | null) });
  await refreshServerChanges("work");
  expect(useServerChanges.getState().byAccount.work).toEqual({ status: null, unavailable: true });
});
it.each(["queued", "claimed"] as const)("accepts %s status without a failure", async (state) => {
  const status = { ...retained, state, failure: null };
  fakeMail({ serverChangeStatus: vi.fn(async () => status) });
  await refreshServerChanges("work");
  expect(useServerChanges.getState().byAccount.work).toEqual({ status, unavailable: false });
});
it("does not resurrect status when an account id is removed and added again", async () => {
  let finish!: (value: ServerChangeStatus | null) => void;
  fakeMail({ serverChangeStatus: vi.fn(() => new Promise<ServerChangeStatus | null>((resolve) => { finish = resolve; })) });
  const pending = refreshServerChanges("work");
  useStore.setState({ accounts: [] }); useStore.setState({ accounts: [work] });
  finish(retained); await pending;
  expect(useServerChanges.getState().byAccount).toEqual({});
});
it("keeps only the declared metadata fields from a status response", async () => {
  fakeMail({ serverChangeStatus: vi.fn(async () => ({ ...retained, payload: "private action payload" })) });
  await refreshServerChanges("work");
  expect(useServerChanges.getState().byAccount.work).toEqual({ status: retained, unavailable: false });
  expect(JSON.stringify(useServerChanges.getState())).not.toContain("private action payload");
});

it("does nothing to a change that isn't blocked, or isn't known", async () => {
  const { backend } = fakeMail();
  await retryServerChange("work");
  useServerChanges.setState({ byAccount: { work: { status: { ...retained, state: "queued", failure: null }, unavailable: false } } });
  await discardServerChange("work");
  expect(backend.retryServerChange).not.toHaveBeenCalled();
  expect(backend.discardServerChange).not.toHaveBeenCalled();
  expect(backend.serverChangeStatus).not.toHaveBeenCalled();
});
