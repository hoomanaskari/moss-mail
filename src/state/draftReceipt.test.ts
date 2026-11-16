import { expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { resetStore, work } from "../test/fixtures";
import { draftsSaved, keepDraft, useDrafts, watchDrafts } from "./drafts";
import { useStore } from "./store";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));
import { tauriMail } from "../services/tauriMail";

it("chains a queued edit through a read-back acknowledgement after the save reply was lost", async () => {
  resetStore([], [work]);
  fakeMail({ saveDraft: tauriMail.saveDraft });
  const stop = watchDrafts(useStore);
  let writes = 0;
  let persisted: Record<string, unknown> | undefined;
  invoke.mockImplementation(async (command: string, args: { draft: Record<string, unknown> }) => {
    if (command === "draft_save_receipt") return persisted;
    expect(command).toBe("save_draft");
    writes++;
    if (writes === 1) {
      persisted = { ...args.draft, revision: "first-ack" };
      throw new Error("The reply was lost after commit");
    }
    expect(args.draft.revision).toBe("first-ack");
    persisted = { ...args.draft, revision: "second-ack" };
    return persisted;
  });
  try {
    useStore.getState().compose("new");
    useStore.getState().updateDraft({ body: "First edit" });
    keepDraft(useStore.getState().draft!);
    useStore.getState().updateDraft({ body: "Latest edit" });
    keepDraft(useStore.getState().draft!);
    await draftsSaved();
    expect(writes).toBe(2);
    expect(invoke.mock.calls.map(([command]) => command)).toEqual(["save_draft", "draft_save_receipt", "save_draft"]);
    expect(persisted).toMatchObject({ body: "Latest edit", revision: "second-ack" });
    expect(useDrafts.getState().unsaved).toEqual({});
    expect(useStore.getState().draft).toMatchObject({ body: "Latest edit", revision: "second-ack" });
  } finally { stop(); }
});

it.each(["send", "sendAfter", "scheduleSend"] as const)("settles an unconfirmed save before %s without writing the draft again", async (action) => {
  invoke.mockReset();
  resetStore([], [work]);
  fakeMail({ saveDraft: tauriMail.saveDraft, draftSaveReceipt: tauriMail.draftSaveReceipt });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  const stop = watchDrafts(useStore);
  let reads = 0;
  let persisted: Record<string, unknown> | undefined;
  const command = { send: "send", sendAfter: "send_after", scheduleSend: "schedule_send" }[action];
  invoke.mockImplementation(async (name: string, args: { draft: Record<string, unknown>; recovery?: Record<string, unknown> }) => {
    if (name === "save_draft") {
      persisted = { ...args.draft, revision: "confirmed" };
      throw new Error("save reply lost");
    }
    if (name === "draft_save_receipt") {
      if (++reads === 1) throw new Error("receipt reply lost");
      return persisted;
    }
    expect(name).toBe(command);
    expect((args.recovery ?? args.draft).revision).toBe("confirmed");
    return action === "scheduleSend" ? { status: "scheduled" } : true;
  });
  try {
    useStore.getState().compose("new");
    useStore.getState().updateDraft({ body: "Latest edit" });
    const draft = useStore.getState().draft!;
    keepDraft(draft); await draftsSaved();
    if (action === "send") await tauriMail.send(draft);
    else if (action === "sendAfter") await tauriMail.sendAfter("operation", draft, 0);
    else await tauriMail.scheduleSend(draft, Date.now() + 1000, "operation");
    expect(invoke.mock.calls.map(([name]) => name)).toEqual(["save_draft", "draft_save_receipt", "draft_save_receipt", command]);
    expect(useDrafts.getState().unconfirmed.size).toBe(0);
  } finally { stop(); }
});

it.each(["send", "sendAfter", "scheduleSend"] as const)("blocks %s while the unconfirmed save cannot be read", async (action) => {
  invoke.mockReset();
  resetStore([], [work]);
  fakeMail({ saveDraft: tauriMail.saveDraft, draftSaveReceipt: tauriMail.draftSaveReceipt });
  vi.spyOn(console, "warn").mockImplementation(() => {});
  const stop = watchDrafts(useStore);
  invoke.mockRejectedValue(new Error("receipt unavailable"));
  try {
    useStore.getState().compose("new");
    useStore.getState().updateDraft({ body: "Keep this text" });
    const draft = useStore.getState().draft!;
    keepDraft(draft); await draftsSaved();
    const actionResult = action === "send" ? tauriMail.send(draft) : action === "sendAfter"
      ? tauriMail.sendAfter("operation", draft, 0)
      : tauriMail.scheduleSend(draft, Date.now() + 1000, "operation");
    await expect(actionResult).rejects.toThrow("receipt unavailable");
    expect(invoke.mock.calls.map(([name]) => name)).toEqual(["save_draft", "draft_save_receipt", "draft_save_receipt"]);
    expect(useDrafts.getState().unconfirmed.size).toBe(1);
    expect(useStore.getState().draft?.body).toBe("Keep this text");
  } finally { stop(); }
});
