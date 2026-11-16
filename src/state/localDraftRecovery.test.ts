import { describe, expect, it, vi } from "vitest";
import { LocalDraftRecoveryWriter } from "./localDraftRecovery";
import type { SavedDraft } from "./drafts";
import type { LocalDraftRecoveryWrite, MailBackend } from "../services/mail";

const draft = (body = "First local text"): SavedDraft => ({ id: "original", revision: "base", mode: "new", fromAccountId: "work", to: [], cc: [], bcc: [], subject: "Work", body, bodyHtml: "", remindIfNoReply: false, updatedAt: 1, signatureId: null, fieldsOpen: true, showCc: false, showBcc: false });
const saved = (body: SavedDraft, version: string): LocalDraftRecoveryWrite => ({ status: "saved", recovery: { id: "recovery", version, draft: body, missingFiles: [], updatedAt: 1 } });

function backend() {
  return { keepLocalDraftRecovery: vi.fn<MailBackend["keepLocalDraftRecovery"]>() };
}

describe("a conflicting composer's recovery writer", () => {
  it("retries the exact unacknowledged snapshot before saving newer text", async () => {
    const service = backend();
    const first = draft();
    const second = { ...draft("Newer local text"), revision: "must-not-change-base" };
    const writer = new LocalDraftRecoveryWriter("recovery", first, () => service);
    service.keepLocalDraftRecovery.mockRejectedValueOnce(new Error("reply lost"));
    await expect(writer.keep(first)).rejects.toThrow("reply lost");
    first.body = "Mutated after submission";
    service.keepLocalDraftRecovery.mockResolvedValueOnce(saved(draft(), "v1")).mockResolvedValueOnce(saved(second, "v2"));
    await expect(writer.keep(second)).resolves.toMatchObject({ status: "saved", recovery: { version: "v2" } });
    expect(service.keepLocalDraftRecovery.mock.calls).toEqual([
      ["recovery", undefined, draft()],
      ["recovery", undefined, draft()],
      ["recovery", "v1", { ...second, revision: "base" }],
    ]);
  });

  it("serializes concurrent edits and freezes each before it waits", async () => {
    const service = backend();
    let finish!: (value: LocalDraftRecoveryWrite) => void;
    service.keepLocalDraftRecovery.mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const first = draft(); const next = draft("Second text");
    const writer = new LocalDraftRecoveryWriter("recovery", first, () => service);
    const one = writer.keep(first); const two = writer.keep(next);
    await Promise.resolve();
    expect(service.keepLocalDraftRecovery).toHaveBeenCalledTimes(1);
    next.body = "Mutated while queued";
    service.keepLocalDraftRecovery.mockResolvedValueOnce(saved(draft("Second text"), "v2"));
    finish(saved(first, "v1"));
    await one; await two;
    expect(service.keepLocalDraftRecovery.mock.calls[1]).toEqual(["recovery", "v1", draft("Second text")]);
  });

  it.each([null, "adopted-draft"])("never creates a fresh recovery after terminal resolution %s", async (adoptedId) => {
    const service = backend(); const first = draft();
    const terminal = { status: "resolved" as const, adoptedId };
    service.keepLocalDraftRecovery.mockRejectedValueOnce(new Error("lost")).mockResolvedValueOnce(terminal);
    const writer = new LocalDraftRecoveryWriter("recovery", first, () => service);
    await expect(writer.keep(first)).rejects.toThrow("lost");
    await expect(writer.keep(draft("later"))).resolves.toEqual(terminal);
    await expect(writer.keep(draft("latest"))).resolves.toEqual(terminal);
    expect(service.keepLocalDraftRecovery).toHaveBeenCalledTimes(2);
    expect(service.keepLocalDraftRecovery.mock.calls[1]).toEqual(["recovery", undefined, first]);
  });
  it("refuses to bind a different draft to an existing recovery chain", async () => {
    const service = backend(); const first = draft();
    service.keepLocalDraftRecovery.mockResolvedValue(saved(first, "v1"));
    const writer = new LocalDraftRecoveryWriter("recovery", first, () => service);
    await expect(writer.keep({ ...first, id: "another-draft" })).rejects.toThrow("different draft");
    expect(service.keepLocalDraftRecovery).not.toHaveBeenCalled();
  });

  it("lets corrected content replace a definitively refused request without changing its predecessor", async () => {
    const service = backend(); const first = draft("Too large");
    const refused = { kind: "localRecoveryRejected", message: "Recovery storage is full" };
    service.keepLocalDraftRecovery.mockRejectedValueOnce(refused).mockResolvedValueOnce(saved(draft("Corrected"), "v1"));
    const writer = new LocalDraftRecoveryWriter("recovery", first, () => service);
    await expect(writer.keep(first)).rejects.toEqual(refused);
    await expect(writer.keep(draft("Corrected"))).resolves.toMatchObject({ status: "saved" });
    expect(service.keepLocalDraftRecovery.mock.calls).toEqual([
      ["recovery", undefined, first], ["recovery", undefined, draft("Corrected")],
    ]);
  });

  it("does not resend an acknowledged snapshot just because its local timestamp changed", async () => {
    const service = backend(); const first = draft(); const result = saved(first, "v1");
    service.keepLocalDraftRecovery.mockResolvedValue(result);
    const writer = new LocalDraftRecoveryWriter("recovery", first, () => service);
    await writer.keep(first);
    await expect(writer.keep({ ...first, updatedAt: 999 })).resolves.toEqual(result);
    expect(service.keepLocalDraftRecovery).toHaveBeenCalledTimes(1);
    await writer.keep(draft("Actual changed content"));
    expect(service.keepLocalDraftRecovery).toHaveBeenCalledTimes(2);
  });

  it("forgets acknowledged content after explicit resolution and ignores a late save reply", async () => {
    const service = backend();const first = draft();
    let finish!: (result: LocalDraftRecoveryWrite) => void;
    service.keepLocalDraftRecovery.mockResolvedValueOnce(saved(first, "v1"))
      .mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const writer = new LocalDraftRecoveryWriter("recovery", first, () => service);
    await writer.keep(first);
    const late = writer.keep(draft("Newer pending"));
    const queued = writer.keep(draft("Queued text"));await Promise.resolve();
    writer.resolve("adopted");
    finish(saved(draft("Newer pending"), "v2"));
    await expect(late).resolves.toEqual({ status: "resolved", adoptedId: "adopted" });
    await expect(queued).resolves.toEqual({ status: "resolved", adoptedId: "adopted" });
    await expect(writer.keep(draft("Newest"))).resolves.toEqual({ status: "resolved", adoptedId: "adopted" });
    expect(service.keepLocalDraftRecovery).toHaveBeenCalledTimes(2);
  });

});
