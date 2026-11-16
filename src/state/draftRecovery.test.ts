import { expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { resetStore, work } from "../test/fixtures";
import { localRecoveryHasNewerEdits, releaseResolvedLocalDraft, confirmDraftSaved, draftsSaved, keepDraft, keepResolvedEdits, localRecoveryFor, localRecoveryIsCurrent, resolveLocalRecovery, useDrafts, watchDrafts } from "./drafts";
import type { MailBackend } from "../services/mail";
import { scheduleDraft, useScheduled } from "./scheduled";
import { useStore } from "./store";

it("keeps a conflicting editing chain in recovery and blocks handoff until explicit adoption", async () => {
  resetStore([], [work]);
  vi.spyOn(console, "warn").mockImplementation(() => {});
  const { backend: service } = fakeMail({
    saveDraft: vi.fn<MailBackend["saveDraft"]>(async () => { throw { kind: "draftConflict", message: "Changed elsewhere" }; }),
    keepLocalDraftRecovery: vi.fn<MailBackend["keepLocalDraftRecovery"]>(async (id, _previous, draft) => ({ status: "saved", recovery: { id, version: draft.body, draft, missingFiles: [], updatedAt: 1 } })),
  });
  const stop = watchDrafts(useStore);
  try {
    useStore.getState().compose("new");
    useStore.getState().updateDraft({ body: "First local text" });
    keepDraft(useStore.getState().draft!); await draftsSaved();
    useStore.getState().updateDraft({ body: "More local text" });
    keepDraft(useStore.getState().draft!); await draftsSaved();
    expect(service.saveDraft).toHaveBeenCalledTimes(1);
    expect(service.keepLocalDraftRecovery).toHaveBeenCalledTimes(2);
    const calls = vi.mocked(service.keepLocalDraftRecovery).mock.calls;
    expect(calls[1][0]).toBe(calls[0][0]);
    expect(calls[1][1]).toBe("First local text");
    expect(calls[1][2].body).toBe("More local text");
    await expect(confirmDraftSaved(useStore.getState().draft!)).rejects.toThrow("Review the local recovery");
    expect(useStore.getState().draft?.body).toBe("More local text");
  } finally { stop(); }
});

it("journals the newest typing when an older save reports a conflict", async () => {
  resetStore([], [work]);
  let reject!: (error: unknown) => void;
  const { backend: service } = fakeMail({
    saveDraft: vi.fn<MailBackend["saveDraft"]>(() => new Promise((_resolve, fail) => { reject = fail; })),
    keepLocalDraftRecovery: vi.fn<MailBackend["keepLocalDraftRecovery"]>(async (id, _previous, draft) => ({ status: "saved", recovery: { id, version: "v1", draft, missingFiles: [], updatedAt: 1 } })),
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  const stop = watchDrafts(useStore);
  try {
    useStore.getState().compose("new");
    useStore.getState().updateDraft({ body: "Submitted text" });
    keepDraft(useStore.getState().draft!);
    await Promise.resolve(); await Promise.resolve();
    useStore.getState().updateDraft({ body: "Latest typing not yet queued" });
    reject({ kind: "draftConflict" }); await draftsSaved();
    expect(service.keepLocalDraftRecovery).toHaveBeenCalledWith(expect.any(String), undefined, expect.objectContaining({ body: "Latest typing not yet queued" }));
    expect(useDrafts.getState().drafts[0].body).toBe("Latest typing not yet queued");
    expect(useDrafts.getState().unsaved[useStore.getState().draft!.id].body).toBe("Latest typing not yet queued");
  } finally { stop(); }
});

it("does not let an older queued save replace newer typing already captured for recovery", async () => {
  resetStore([], [work]);
  let reject!: (error: unknown) => void;
  const { backend: service } = fakeMail({
    saveDraft: vi.fn<MailBackend["saveDraft"]>(() => new Promise((_resolve, fail) => { reject = fail; })),
    keepLocalDraftRecovery: vi.fn<MailBackend["keepLocalDraftRecovery"]>(async (id, _previous, draft) => ({ status: "saved", recovery: { id, version: draft.body, draft, missingFiles: [], updatedAt: 1 } })),
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  const stop = watchDrafts(useStore);
  try {
    useStore.getState().compose("new");
    useStore.getState().updateDraft({ body: "A submitted" });
    keepDraft(useStore.getState().draft!);
    await Promise.resolve(); await Promise.resolve();
    useStore.getState().updateDraft({ body: "B queued" });
    keepDraft(useStore.getState().draft!);
    useStore.getState().updateDraft({ body: "C latest" });
    reject({ kind: "draftConflict" }); await draftsSaved();
    expect(vi.mocked(service.keepLocalDraftRecovery).mock.calls.map((call) => call[2].body)).toEqual(["C latest"]);
  } finally { stop(); }
});

it.each(["send", "sendDraft", "schedule", "delete", "discard"])("refuses %s before changing a frozen composer's local state", async (action) => {
  resetStore([], [work]);
  const { backend } = fakeMail({
    saveDraft: vi.fn<MailBackend["saveDraft"]>(async () => { throw { kind: "draftConflict" }; }),
    keepLocalDraftRecovery: vi.fn<MailBackend["keepLocalDraftRecovery"]>(async (id, _previous, draft) => ({ status: "saved", recovery: { id, version: "v1", draft, missingFiles: [], updatedAt: 1 } })),
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  const stop = watchDrafts(useStore);
  try {
    useStore.getState().compose("new");useStore.getState().updateDraft({ subject: "Project", body: "Conflict text" });
    keepDraft(useStore.getState().draft!);await draftsSaved();
    useStore.getState().updateDraft({ body: "Still typing" });
    const draft = useStore.getState().draft!;
    const before = useDrafts.getState(); const scheduled = useScheduled.getState().emails;
    if (action === "send") useStore.getState().send();
    else if (action === "sendDraft") useStore.getState().sendDraft(draft);
    else if (action === "schedule") scheduleDraft(draft, Date.now() + 10000);
    else if (action === "delete") useStore.getState().deleteDraft(draft.id);
    else useStore.getState().discardDraft();
    expect(useStore.getState().draft).toBe(draft);
    expect(useStore.getState().sending).toBeFalsy();
    expect(useDrafts.getState().drafts).toBe(before.drafts);
    expect(useDrafts.getState().unsaved).toBe(before.unsaved);
    expect(useDrafts.getState().retired).toBe(before.retired);
    expect(useScheduled.getState().emails).toBe(scheduled);
    expect(backend.sendAfter).not.toHaveBeenCalled();expect(backend.scheduleSend).not.toHaveBeenCalled();expect(backend.deleteDraft).not.toHaveBeenCalled();
    expect(useStore.getState().sendError).toMatch(/Review the local recovery/);
  } finally { stop(); }
});

it.each([false, true])("resolving recovery preserves edits newer than the reviewed snapshot: %s", async (newer) => {
  resetStore([], [work]);
  const { backend } = fakeMail({
    saveDraft: vi.fn<MailBackend["saveDraft"]>(async () => { throw { kind: "draftConflict" }; }),
    keepLocalDraftRecovery: vi.fn<MailBackend["keepLocalDraftRecovery"]>(async (id, _previous, draft) => ({ status: "saved", recovery: { id, version: "reviewed", draft, missingFiles: [], updatedAt: 1 } })),
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  const stop = watchDrafts(useStore);
  try {
    useStore.getState().compose("new");useStore.getState().updateDraft({ body: "Reviewed local text" });
    keepDraft(useStore.getState().draft!);await draftsSaved();
    const record = localRecoveryFor(useStore.getState().draft!)!;
    if (newer) useStore.getState().updateDraft({ body: "Typing after review" });
    resolveLocalRecovery(record.writer.id, "reviewed", "adopted-draft");
    expect(localRecoveryFor(useStore.getState().draft!)?.outcome).toEqual({ status: "resolved", adoptedId: "adopted-draft" });
    expect(useDrafts.getState().drafts).toHaveLength(newer ? 1 : 0);
    expect(Object.keys(useDrafts.getState().unsaved)).toHaveLength(newer ? 1 : 0);
    if (newer) {
      keepDraft(useStore.getState().draft!);await draftsSaved();
      expect(useStore.getState().draft?.body).toBe("Typing after review");
      expect(backend.keepLocalDraftRecovery).toHaveBeenCalledTimes(1);
    }
  } finally { stop(); }
});

it("starts a fresh recovery for post-resolution edits only on an explicit request", async () => {
  resetStore([], [work]);
  const { backend } = fakeMail({
    saveDraft: vi.fn<MailBackend["saveDraft"]>(async () => { throw { kind: "draftConflict" }; }),
    keepLocalDraftRecovery: vi.fn<MailBackend["keepLocalDraftRecovery"]>(async (id, _previous, draft) => ({ status: "saved", recovery: { id, version: "v1", draft, missingFiles: [], updatedAt: 1 } })),
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  const stop = watchDrafts(useStore);
  try {
    useStore.getState().compose("new");useStore.getState().updateDraft({ body: "Reviewed" });
    keepDraft(useStore.getState().draft!);await draftsSaved();
    const oldId = localRecoveryFor(useStore.getState().draft!)!.writer.id;
    useStore.getState().updateDraft({ body: "New edits" });
    resolveLocalRecovery(oldId, "v1", null);
    keepResolvedEdits(useStore.getState().draft!);await draftsSaved();
    const newId = localRecoveryFor(useStore.getState().draft!)!.writer.id;
    expect(newId).not.toBe(oldId);
    expect(backend.keepLocalDraftRecovery).toHaveBeenLastCalledWith(newId, undefined, expect.objectContaining({ body: "New edits" }));
    expect(backend.saveDraft).toHaveBeenCalledTimes(1);
  } finally { stop(); }
});

it("isolates a recovered chain from another editor of the same draft and refuses list handoff", async () => {
  resetStore([], [work]);vi.spyOn(console, "warn").mockImplementation(() => {});
  const save = vi.fn<MailBackend["saveDraft"]>().mockRejectedValueOnce({ kind: "draftConflict" }).mockImplementation(async (draft) => ({ ...draft, revision: "winner-next" }));
  const { backend } = fakeMail({ saveDraft: save, keepLocalDraftRecovery: vi.fn<MailBackend["keepLocalDraftRecovery"]>(async (id, _previous, draft) => ({ status: "saved", recovery: { id, version: "v1", draft, missingFiles: [], updatedAt: 1 } })) });
  useStore.getState().compose("new");useStore.getState().updateDraft({ body: "My local copy" });
  const mine = useStore.getState().draft!;keepDraft(mine);await draftsSaved();
  expect(localRecoveryIsCurrent(mine)).toBe(true);
  const other = { ...mine, revision: "winner", body: "Other editor" };
  keepDraft(other);await draftsSaved();
  expect(localRecoveryFor(other)).toBeUndefined();
  expect(save).toHaveBeenLastCalledWith(expect.objectContaining({ revision: "winner", body: "Other editor" }));
  useStore.setState({ view: "drafts" });useStore.getState().sendDraft(mine);
  expect(backend.sendAfter).not.toHaveBeenCalled();
  expect(useStore.getState().toast?.text).toMatch(/Review the local recovery/);
  const normal = { ...mine, id: "normal-new" };
  expect(localRecoveryIsCurrent(normal)).toBe(false);
  keepResolvedEdits(normal);await draftsSaved();
  expect(save).toHaveBeenCalledTimes(3);expect(backend.keepLocalDraftRecovery).toHaveBeenCalledTimes(1);
  resolveLocalRecovery("unrelated", "v1", null);
  expect(localRecoveryFor(mine)?.outcome?.status).toBe("saved");
  resolveLocalRecovery(localRecoveryFor(mine)!.writer.id, "stale-review", null);
  expect(localRecoveryFor(mine)?.latest?.body).toBe("My local copy");
  expect(useDrafts.getState().drafts.some((row) => row.body === "Other editor")).toBe(true);
});

it.each([false, true])("a superseded recovery writer cannot change a fresh explicit copy, failure=%s", async (failure) => {
  resetStore([], [work]);vi.spyOn(console, "warn").mockImplementation(() => {});
  let finish!: (value: Awaited<ReturnType<MailBackend["keepLocalDraftRecovery"]>>) => void;
  let reject!: (error: unknown) => void;
  const keep = vi.fn<MailBackend["keepLocalDraftRecovery"]>().mockImplementationOnce(() => new Promise((ok, fail) => { finish = ok;reject = fail; })).mockImplementation(async (id, _previous, draft) => ({ status: "saved", recovery: { id, version: "new-version", draft, missingFiles: [], updatedAt: 1 } }));
  fakeMail({ saveDraft: vi.fn(async () => { throw { kind: "draftConflict" }; }), keepLocalDraftRecovery: keep });
  const stop = watchDrafts(useStore);
  try {
    useStore.getState().compose("new");useStore.getState().updateDraft({ body: "Old copy" });
    keepDraft(useStore.getState().draft!);
    await vi.waitFor(() => expect(keep).toHaveBeenCalledOnce());
    const old = localRecoveryFor(useStore.getState().draft!)!;
    resolveLocalRecovery(old.writer.id, "old-version", null);
    useStore.getState().updateDraft({ body: "New explicit copy" });
    keepResolvedEdits(useStore.getState().draft!);
    const freshId = localRecoveryFor(useStore.getState().draft!)!.writer.id;
    if (failure) reject(new Error("lost reply"));
    else finish({ status: "saved", recovery: { id: old.writer.id, version: "old-version", draft: keep.mock.calls[0][2], missingFiles: [], updatedAt: 1 } });
    await draftsSaved();
    expect(localRecoveryFor(useStore.getState().draft!)!.writer.id).toBe(freshId);
    expect(localRecoveryFor(useStore.getState().draft!)!.outcome).toMatchObject({ status: "saved", recovery: { version: "new-version", draft: { body: "New explicit copy" } } });
  } finally { stop(); }
});

it.each(["saved", "resolved"] as const)("keeps terminal state when a delayed %s result arrives", async (status) => {
  resetStore([], [work]);vi.spyOn(console, "warn").mockImplementation(() => {});
  let finish!: (value: Awaited<ReturnType<MailBackend["keepLocalDraftRecovery"]>>) => void;
  const keep = vi.fn<MailBackend["keepLocalDraftRecovery"]>(() => new Promise((resolve) => { finish = resolve; }));
  fakeMail({ saveDraft: vi.fn(async () => { throw { kind: "draftConflict" }; }), keepLocalDraftRecovery: keep });
  const stop = watchDrafts(useStore);
  try {
    useStore.getState().compose("new");useStore.getState().updateDraft({ body: "Local copy" });
    keepDraft(useStore.getState().draft!);await vi.waitFor(() => expect(keep).toHaveBeenCalledOnce());
    const id = localRecoveryFor(useStore.getState().draft!)!.writer.id;
    if (status === "saved") resolveLocalRecovery(id, "v1", "adopted");
    finish(status === "saved" ? { status, recovery: { id, version: "v1", draft: keep.mock.calls[0][2], missingFiles: [], updatedAt: 1 } } : { status, adoptedId: "adopted" });
    await draftsSaved();
    expect(localRecoveryFor(useStore.getState().draft!)?.outcome).toEqual({ status: "resolved", adoptedId: "adopted" });
    expect(localRecoveryIsCurrent(useStore.getState().draft!)).toBe(false);
  } finally { stop(); }
});

it.each(["discard", "delete"])("explicitly releases resolved local edits without deleting the winning draft: %s", async (action) => {
  resetStore([], [work]);
  vi.spyOn(console, "warn").mockImplementation(() => {});
  const { backend } = fakeMail({
    saveDraft: vi.fn(async () => { throw { kind: "draftConflict" }; }),
    keepLocalDraftRecovery: vi.fn<MailBackend["keepLocalDraftRecovery"]>(async (id, _previous, draft) => ({ status: "saved", recovery: { id, version: "v1", draft, missingFiles: [], updatedAt: 1 } })),
  });
  const stop = watchDrafts(useStore);
  try {
    useStore.getState().compose("new");useStore.getState().updateDraft({ body: "Reviewed" });
    keepDraft(useStore.getState().draft!);await draftsSaved();
    const draft = useStore.getState().draft!;
    useStore.getState().updateDraft({ body: "Newer local text to discard" });
    resolveLocalRecovery(localRecoveryFor(draft)!.writer.id, "v1", null);
    const winner = { ...draft, body: "Authoritative winning text", updatedAt: 1, revision: "winner" };
    vi.mocked(backend.drafts).mockResolvedValue([winner]);
    if (action === "discard") useStore.getState().discardDraft();
    else useStore.getState().deleteDraft(draft.id);
    await draftsSaved();
    expect(useDrafts.getState().drafts).toEqual([winner]);
    expect(useDrafts.getState().unsaved).toEqual({});
    if (action === "delete") expect(useStore.getState().toast?.text).toBe("Local edits discarded");
    expect(backend.deleteDraft).not.toHaveBeenCalled();
    expect(backend.saveDraft).toHaveBeenCalledTimes(1);
    expect(backend.keepLocalDraftRecovery).toHaveBeenCalledTimes(1);
  } finally { stop(); }
});

it("compares review versions and ignores other recovery chains", async () => {
  resetStore([], [work]);vi.spyOn(console, "warn").mockImplementation(() => {});
  const keep = vi.fn<MailBackend["keepLocalDraftRecovery"]>().mockRejectedValueOnce(new Error("disk full"))
    .mockImplementation(async (id, _previous, draft) => ({ status: "saved", recovery: { id, version: "v1", draft, missingFiles: [], updatedAt: 1 } }));
  fakeMail({ saveDraft: vi.fn(async () => { throw { kind: "draftConflict" }; }), keepLocalDraftRecovery: keep });
  const stop = watchDrafts(useStore);
  try {
    useStore.getState().compose("new");useStore.getState().updateDraft({ body: "Mine" });
    keepDraft(useStore.getState().draft!);await draftsSaved();
    const id = localRecoveryFor(useStore.getState().draft!)!.writer.id;
    expect(localRecoveryHasNewerEdits(id, "v1")).toBe(true);
    expect(localRecoveryHasNewerEdits("another", "v1")).toBe(false);
    keepDraft(useStore.getState().draft!);await draftsSaved();
    expect(localRecoveryHasNewerEdits(id, "stale")).toBe(true);
    expect(localRecoveryHasNewerEdits(id, "v1")).toBe(false);
    const mine = useStore.getState().draft!;
    useStore.getState().compose("new");useStore.getState().updateDraft({ body: "Other chain" });
    keepDraft(useStore.getState().draft!);await draftsSaved();
    keepDraft(mine);await draftsSaved();
    expect(useDrafts.getState().drafts.map((row) => row.body)).toContain("Other chain");
    resolveLocalRecovery(id, "stale", null);
    stop();
    expect(releaseResolvedLocalDraft(mine)).toBe(true);
    expect(useDrafts.getState().drafts.map((row) => row.body)).toEqual(["Other chain"]);
  } finally { stop(); }
});

it("does not replace another editor's pending save marker while journalling the same id", async () => {
  resetStore([], [work]);vi.spyOn(console, "warn").mockImplementation(() => {});
  let reject!: (error: unknown) => void;
  const save = vi.fn<MailBackend["saveDraft"]>().mockImplementationOnce(() => new Promise((_ok, fail) => { reject = fail; }))
    .mockImplementation(async (draft) => ({ ...draft, revision: "other-next" }));
  fakeMail({ saveDraft: save, keepLocalDraftRecovery: vi.fn<MailBackend["keepLocalDraftRecovery"]>(async (id, _previous, draft) => ({ status: "saved", recovery: { id, version: "v1", draft, missingFiles: [], updatedAt: 1 } })) });
  useStore.getState().compose("new");useStore.getState().updateDraft({ body: "Local conflicting text" });
  const mine = useStore.getState().draft!;keepDraft(mine);
  await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());
  const other = { ...mine, body: "Other editor's text", revision: "winner" };
  keepDraft(other);reject({ kind: "draftConflict" });await draftsSaved();
  expect(useDrafts.getState().unsaved).toEqual({});
  expect(useDrafts.getState().drafts[0].body).toBe("Other editor's text");
  expect(localRecoveryFor(mine)?.latest?.body).toBe("Local conflicting text");
});

it("does not mark the winning row failed when a journal reply arrives after local release", async () => {
  resetStore([], [work]);vi.spyOn(console, "warn").mockImplementation(() => {});
  let finish!: (value: Awaited<ReturnType<MailBackend["keepLocalDraftRecovery"]>>) => void;
  const keep = vi.fn<MailBackend["keepLocalDraftRecovery"]>(() => new Promise((resolve) => { finish = resolve; }));
  fakeMail({ saveDraft: vi.fn(async () => { throw { kind: "draftConflict" }; }), keepLocalDraftRecovery: keep });
  useStore.getState().compose("new");useStore.getState().updateDraft({ body: "Local copy" });
  const mine = useStore.getState().draft!;keepDraft(mine);
  await vi.waitFor(() => expect(keep).toHaveBeenCalledOnce());
  const id = localRecoveryFor(mine)!.writer.id;
  resolveLocalRecovery(id, "v1", null);releaseResolvedLocalDraft(mine);
  const winner = { ...mine, body: "Winner", revision: "winner", updatedAt: 1 };
  useDrafts.setState({ drafts: [winner], status: { [winner.id]: "saved" } });
  finish({ status: "saved", recovery: { id, version: "v1", draft: keep.mock.calls[0][2], missingFiles: [], updatedAt: 1 } });
  await draftsSaved();
  expect(useDrafts.getState().status[winner.id]).toBe("saved");
  expect(localRecoveryFor(mine)?.latest).toBeUndefined();
});

it("does not mark the winning row failed when a journal reply arrives for an unreleased older chain", async () => {
  resetStore([], [work]);vi.spyOn(console, "warn").mockImplementation(() => {});
  let finish!: (value: Awaited<ReturnType<MailBackend["keepLocalDraftRecovery"]>>) => void;
  const keep = vi.fn<MailBackend["keepLocalDraftRecovery"]>(() => new Promise((resolve) => { finish = resolve; }));
  fakeMail({ saveDraft: vi.fn(async () => { throw { kind: "draftConflict" }; }), keepLocalDraftRecovery: keep });
  useStore.getState().compose("new");useStore.getState().updateDraft({ body: "Local copy" });
  const mine = useStore.getState().draft!;keepDraft(mine);
  await vi.waitFor(() => expect(keep).toHaveBeenCalledOnce());
  const id = localRecoveryFor(mine)!.writer.id;
  
  const winner = { ...mine, body: "Winner", revision: "winner", updatedAt: 1 };
  useDrafts.setState({ drafts: [winner], status: { [winner.id]: "saved" } });
  finish({ status: "saved", recovery: { id, version: "v1", draft: keep.mock.calls[0][2], missingFiles: [], updatedAt: 1 } });
  await draftsSaved();
  expect(useDrafts.getState().status[winner.id]).toBe("saved");
  expect(localRecoveryFor(mine)?.latest?.body).toBe("Local copy");
});
