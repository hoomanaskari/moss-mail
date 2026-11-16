import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { resetStore, work } from "../test/fixtures";
import type { LocalDraftRecovery, MailBackend } from "../services/mail";
import { LocalDraftRecoveries } from "./LocalDraftRecoveries";

import { draftsSaved, keepDraft, watchDrafts } from "../state/drafts";
import { useStore } from "../state/store";

const recovery: LocalDraftRecovery = { id: "local", version: "v1", updatedAt: 1, missingFiles: ["missing"], draft: { id: "original", revision: "base", mode: "forward", fromAccountId: "removed", to: [{ name: "Alex", email: "alex@example.test", initials: "A" }], cc: [], bcc: [], subject: "Project", body: "My unsaved work", bodyHtml: "", remindIfNoReply: false, updatedAt: 1, signatureId: null, fieldsOpen: true, showCc: false, showBcc: false, attachments: [{ id: "missing", name: "plans.pdf", bytes: 12, contentType: "application/pdf" }] } };
const entry = { id: recovery.id, version: recovery.version, originalId: recovery.draft.id, updatedAt: 1 };

it("requires explicit account and disclosure review before adopting with missing files", async () => {
  const adopted = { ...recovery.draft, id: "new-draft", mode: "new" as const, attachments: [] };
  const { backend } = fakeMail({ localDraftRecoveryIndex: vi.fn(async () => [entry]), localDraftRecovery: vi.fn(async () => recovery), adoptLocalDraftRecovery: vi.fn(async () => adopted) });
  const onAdopt = vi.fn(); const user = userEvent.setup();
  render(<LocalDraftRecoveries accounts={[work]} onAdopt={onAdopt} onResolve={vi.fn()} />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  expect(await screen.findByText("My unsaved work")).toBeVisible();
  expect(screen.getByText(/without the original quoted message/)).toBeVisible();
  expect(screen.getByText(/sync this text/)).toBeVisible();
  expect(screen.getByText(/Check Sent and Scheduled before sending a recovered copy/)).toBeVisible();
  const open = screen.getByRole("button", { name: "Open as new draft" });
  expect(open).toBeDisabled();
  await user.selectOptions(screen.getByRole("combobox", { name: "Save recovery to account" }), work.id);
  await user.click(screen.getByRole("checkbox", { name: "I understand this opens a standalone email" }));
  expect(open).toBeDisabled();
  await user.click(screen.getByRole("checkbox", { name: "Continue without the unavailable files" }));
  await user.click(open);
  await waitFor(() => expect(onAdopt).toHaveBeenCalledWith(adopted, recovery.id, recovery.version));
  expect(backend.adoptLocalDraftRecovery).toHaveBeenCalledWith("local", "v1", expect.any(String), work.id, ["missing"]);
});

it("allows explicit discard of an unreadable entry using its independent revision", async () => {
  const { backend } = fakeMail({ localDraftRecoveryIndex: vi.fn(async () => [entry]), localDraftRecovery: vi.fn(async () => { throw new Error("Recovery is damaged"); }), discardLocalDraftRecovery: vi.fn(async () => {}) });
  const onResolve = vi.fn(); const user = userEvent.setup();
  render(<LocalDraftRecoveries accounts={[]} onAdopt={vi.fn()} onResolve={onResolve} />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Recovery is damaged");
  await user.click(screen.getByRole("button", { name: "Discard local recovery" }));
  expect(backend.discardLocalDraftRecovery).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Confirm discard" }));
  await waitFor(() => expect(onResolve).toHaveBeenCalledWith("local", "v1", null));
  expect(backend.discardLocalDraftRecovery).toHaveBeenCalledWith("local", "v1");
});

it("still reports a committed adoption when the review screen was closed while it waited", async () => {
  const copy = { ...recovery, missingFiles: [] };
  let finish!: (draft: typeof recovery.draft) => void;
  fakeMail({ localDraftRecoveryIndex: vi.fn(async () => [entry]), localDraftRecovery: vi.fn(async () => copy), adoptLocalDraftRecovery: vi.fn<MailBackend["adoptLocalDraftRecovery"]>(() => new Promise((resolve) => { finish = resolve; })) });
  const onAdopt = vi.fn(); const user = userEvent.setup();
  const view = render(<LocalDraftRecoveries accounts={[work]} onAdopt={onAdopt} onResolve={vi.fn()} />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.selectOptions(await screen.findByRole("combobox"), work.id);
  await user.click(screen.getByRole("checkbox", { name: "I understand this opens a standalone email" }));
  await user.click(screen.getByRole("button", { name: "Open as new draft" }));
  view.unmount(); finish({ ...copy.draft, id: "adopted" });
  await waitFor(() => expect(onAdopt).toHaveBeenCalledWith(expect.objectContaining({ id: "adopted" }), "local", "v1"));
});

it("recovers from an index error and allows cancelling discard of a disappeared entry", async () => {
  const index = vi.fn<MailBackend["localDraftRecoveryIndex"]>().mockRejectedValueOnce(new Error("Index unavailable")).mockResolvedValue([entry]);
  const { backend } = fakeMail({ localDraftRecoveryIndex: index, localDraftRecovery: vi.fn(async () => undefined) });
  const user = userEvent.setup();render(<LocalDraftRecoveries accounts={[]} onAdopt={vi.fn()} onResolve={vi.fn()} />);
  expect(await screen.findByRole("alert")).toHaveTextContent("Index unavailable");
  await user.click(screen.getByRole("button", { name: "Refresh recoveries" }));
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("no longer available");
  await user.click(screen.getByRole("button", { name: "Discard local recovery" }));
  await user.click(screen.getByRole("button", { name: "Keep recovery" }));
  expect(screen.queryByRole("button", { name: "Confirm discard" })).not.toBeInTheDocument();
  expect(backend.discardLocalDraftRecovery).not.toHaveBeenCalled();
});

it.each([undefined, []])("shows unavailable metadata and a missing account without enabling adoption: %s", async (attachments) => {
  const copy = { ...recovery, draft: { ...recovery.draft, subject: "", attachments } };
  fakeMail({ localDraftRecoveryIndex: vi.fn(async () => [entry]), localDraftRecovery: vi.fn(async () => copy) });
  const user = userEvent.setup();render(<LocalDraftRecoveries accounts={[]} onAdopt={vi.fn()} onResolve={vi.fn()} />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  expect(await screen.findByRole("heading", { name: "No subject" })).toBeVisible();
  expect(screen.getByText("Unavailable attachment")).toBeVisible();
  expect(screen.getByText(/Add an account before opening/)).toBeVisible();
  expect(screen.getByRole("button", { name: "Open as new draft" })).toBeDisabled();
});

it("retries an ambiguous adoption with exactly the same target and reports a retired result", async () => {
  const copy = { ...recovery, missingFiles: [] };
  const adopt = vi.fn<MailBackend["adoptLocalDraftRecovery"]>().mockRejectedValueOnce(new Error("reply lost")).mockResolvedValueOnce(undefined);
  fakeMail({ localDraftRecoveryIndex: vi.fn(async () => [entry, { ...entry, id: "other" }]), localDraftRecovery: vi.fn(async () => copy), adoptLocalDraftRecovery: adopt });
  const onResolve = vi.fn();const user = userEvent.setup();
  render(<LocalDraftRecoveries accounts={[work]} onAdopt={vi.fn()} onResolve={onResolve} />);
  await waitFor(() => expect(screen.getAllByRole("button", { name: "Review local recovery" })).toHaveLength(2));
  await user.click(screen.getAllByRole("button", { name: "Review local recovery" })[0]);
  await user.selectOptions(await screen.findByRole("combobox"), work.id);
  await user.click(screen.getByRole("checkbox", { name: "I understand this opens a standalone email" }));
  await user.click(screen.getByRole("button", { name: "Open as new draft" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("reply lost");
  expect(screen.getByRole("combobox")).toBeDisabled();
  expect(screen.getByRole("checkbox")).toBeDisabled();
  await user.click(screen.getByRole("button", { name: "Open as new draft" }));
  await waitFor(() => expect(onResolve).toHaveBeenCalledWith("local", "v1", expect.any(String)));
  expect(adopt.mock.calls[1]).toEqual(adopt.mock.calls[0]);
  expect(screen.getByRole("status")).toHaveTextContent("no longer editable");
  expect(screen.getAllByRole("button", { name: "Review local recovery" })).toHaveLength(1);
});

it("keeps a failed discard available for an idempotent retry", async () => {
  const discard = vi.fn<MailBackend["discardLocalDraftRecovery"]>().mockRejectedValueOnce(new Error("disk unavailable")).mockResolvedValueOnce();
  fakeMail({ localDraftRecoveryIndex: vi.fn(async () => [entry]), localDraftRecovery: vi.fn(async () => recovery), discardLocalDraftRecovery: discard });
  const onResolve = vi.fn();const user = userEvent.setup();
  render(<LocalDraftRecoveries accounts={[]} onAdopt={vi.fn()} onResolve={onResolve} />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.click(await screen.findByRole("button", { name: "Discard local recovery" }));
  await user.click(screen.getByRole("button", { name: "Confirm discard" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("disk unavailable");
  expect(onResolve).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Confirm discard" }));
  await waitFor(() => expect(onResolve).toHaveBeenCalledWith("local", "v1", null));
  expect(discard.mock.calls).toEqual([["local", "v1"], ["local", "v1"]]);
});

function deferred<T>() {
  let resolve!: (value: T) => void;let reject!: (error: unknown) => void;
  const promise = new Promise<T>((ok, fail) => { resolve = ok;reject = fail; });
  return { promise, resolve, reject };
}

it.each([false, true])("ignores a completed index load after unmount, failure=%s", async (failure) => {
  const pending = deferred<typeof entry[]>();
  const index = vi.fn(() => pending.promise);
  fakeMail({ localDraftRecoveryIndex: index });
  const view = render(<LocalDraftRecoveries accounts={[]} onAdopt={vi.fn()} onResolve={vi.fn()} />);
  await waitFor(() => expect(index).toHaveBeenCalledOnce());view.unmount();
  await act(async () => { if (failure) pending.reject(new Error("closed"));else pending.resolve([entry]); });
  expect(screen.queryByRole("region", { name: "Local draft recoveries" })).not.toBeInTheDocument();
});

it.each([false, true])("ignores a completed review load after unmount, failure=%s", async (failure) => {
  const pending = deferred<LocalDraftRecovery>();
  const read = vi.fn(() => pending.promise);
  fakeMail({ localDraftRecoveryIndex: vi.fn(async () => [entry]), localDraftRecovery: read });
  const user = userEvent.setup();const onAdopt = vi.fn();const onResolve = vi.fn();
  const view = render(<LocalDraftRecoveries accounts={[]} onAdopt={onAdopt} onResolve={onResolve} />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));view.unmount();
  await act(async () => { if (failure) pending.reject(new Error("closed"));else pending.resolve(recovery); });
  expect(read).toHaveBeenCalledWith("local");expect(onAdopt).not.toHaveBeenCalled();expect(onResolve).not.toHaveBeenCalled();
});

it.each([false, true])("reports only a committed discard after unmount, failure=%s", async (failure) => {
  const pending = deferred<void>();const discard = vi.fn(() => pending.promise);
  fakeMail({ localDraftRecoveryIndex: vi.fn(async () => [entry]), localDraftRecovery: vi.fn(async () => recovery), discardLocalDraftRecovery: discard });
  const user = userEvent.setup();const onResolve = vi.fn();
  const view = render(<LocalDraftRecoveries accounts={[]} onAdopt={vi.fn()} onResolve={onResolve} />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.click(await screen.findByRole("button", { name: "Discard local recovery" }));
  await user.click(screen.getByRole("button", { name: "Confirm discard" }));view.unmount();
  await act(async () => { if (failure) pending.reject(new Error("closed"));else pending.resolve(); });
  if (failure) expect(onResolve).not.toHaveBeenCalled();else expect(onResolve).toHaveBeenCalledWith("local", "v1", null);
});

it("does not report an adoption that failed after the screen closed", async () => {
  const pending = deferred<typeof recovery.draft | undefined>();
  fakeMail({ localDraftRecoveryIndex: vi.fn(async () => [entry]), localDraftRecovery: vi.fn(async () => ({ ...recovery, missingFiles: [] })), adoptLocalDraftRecovery: vi.fn(() => pending.promise) });
  const onAdopt = vi.fn();const onResolve = vi.fn();const user = userEvent.setup();
  const view = render(<LocalDraftRecoveries accounts={[work]} onAdopt={onAdopt} onResolve={onResolve} />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.selectOptions(await screen.findByRole("combobox"), work.id);
  await user.click(screen.getByRole("checkbox", { name: "I understand this opens a standalone email" }));
  await user.click(screen.getByRole("button", { name: "Open as new draft" }));view.unmount();
  await act(async () => pending.reject(new Error("disk failed")));
  expect(onAdopt).not.toHaveBeenCalled();expect(onResolve).not.toHaveBeenCalled();
});

it("warns and blocks adoption when the composer has text newer than the reviewed recovery", async () => {
  resetStore([], [work]);vi.spyOn(console, "warn").mockImplementation(() => {});
  let copy: LocalDraftRecovery;
  fakeMail({
    saveDraft: vi.fn(async () => { throw { kind: "draftConflict" }; }),
    keepLocalDraftRecovery: vi.fn<MailBackend["keepLocalDraftRecovery"]>(async (id, _previous, draft) => {
      copy = { id, version: "v1", draft, missingFiles: [], updatedAt: 1 };
      return { status: "saved", recovery: copy };
    }),
    localDraftRecoveryIndex: vi.fn(async () => [{ id: copy.id, version: copy.version, originalId: copy.draft.id, updatedAt: 1 }]),
    localDraftRecovery: vi.fn(async () => copy),
  });
  const stop = watchDrafts(useStore);
  try {
    useStore.getState().compose("new");useStore.getState().updateDraft({ body: "Saved text" });
    keepDraft(useStore.getState().draft!);await draftsSaved();
    useStore.getState().updateDraft({ body: "Newer text still in editor" });
    const user = userEvent.setup();render(<LocalDraftRecoveries accounts={[work]} onAdopt={vi.fn()} onResolve={vi.fn()} />);
    await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
    expect(await screen.findByText(/newer local edits/)).toBeVisible();
    await user.selectOptions(screen.getByRole("combobox"), work.id);
    await user.click(screen.getByRole("checkbox", { name: "I understand this opens a standalone email" }));
    expect(screen.getByRole("button", { name: "Open as new draft" })).toBeDisabled();
  } finally { stop(); }
});

it.each(["review", "refresh", "remount"])("retains an ambiguous adoption identity across %s even after the open journal disappears", async (action) => {
  const copy = { ...recovery, missingFiles: [] };
  const index = vi.fn(async () => [entry]);
  const read = vi.fn<MailBackend["localDraftRecovery"]>(async () => copy);
  const adopt = vi.fn<MailBackend["adoptLocalDraftRecovery"]>().mockRejectedValueOnce(new Error("reply lost")).mockResolvedValueOnce({ ...copy.draft, id: "adopted" });
  fakeMail({ localDraftRecoveryIndex: index, localDraftRecovery: read, adoptLocalDraftRecovery: adopt });
  const onAdopt = vi.fn();const user = userEvent.setup();
  const panel = () => <LocalDraftRecoveries accounts={[work]} onAdopt={onAdopt} onResolve={vi.fn()} />;
  let view = render(panel());
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.selectOptions(await screen.findByRole("combobox"), work.id);
  await user.click(screen.getByRole("checkbox", { name: "I understand this opens a standalone email" }));
  await user.click(screen.getByRole("button", { name: "Open as new draft" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("reply lost");
  if (action !== "review") index.mockResolvedValue([]);read.mockResolvedValue(undefined);
  if (action === "remount") { view.unmount();view = render(panel()); }
  else if (action === "refresh") await user.click(screen.getByRole("button", { name: "Refresh recoveries" }));
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.click(await screen.findByRole("button", { name: "Open as new draft" }));
  await waitFor(() => expect(onAdopt).toHaveBeenCalledOnce());
  expect(adopt.mock.calls[1]).toEqual(adopt.mock.calls[0]);
});

it("releases a definitively refused adoption and loads the new revision for fresh review", async () => {
  const copy = { ...recovery, missingFiles: [] };
  const read = vi.fn<MailBackend["localDraftRecovery"]>().mockResolvedValueOnce(copy).mockResolvedValue({ ...copy, version: "v2", draft: { ...copy.draft, body: "Updated saved copy" } });
  const adopt = vi.fn<MailBackend["adoptLocalDraftRecovery"]>().mockRejectedValueOnce({ kind: "localRecoveryRejected", message: "Recovery changed. Review it again." }).mockResolvedValue({ ...copy.draft, id: "new" });
  fakeMail({ localDraftRecoveryIndex: vi.fn(async () => [entry]), localDraftRecovery: read, adoptLocalDraftRecovery: adopt });
  const user = userEvent.setup(); const onAdopt = vi.fn();
  render(<LocalDraftRecoveries accounts={[work]} onAdopt={onAdopt} onResolve={vi.fn()} />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.selectOptions(await screen.findByRole("combobox"), work.id);
  await user.click(screen.getByRole("checkbox", { name: "I understand this opens a standalone email" }));
  await user.click(screen.getByRole("button", { name: "Open as new draft" }));
  expect(await screen.findByText("Updated saved copy")).toBeVisible();
  expect(screen.getByRole("combobox")).toBeEnabled();
  expect(screen.getByRole("button", { name: "Open as new draft" })).toBeDisabled();
  await user.selectOptions(screen.getByRole("combobox"), work.id);
  await user.click(screen.getByRole("checkbox", { name: "I understand this opens a standalone email" }));
  await user.click(screen.getByRole("button", { name: "Open as new draft" }));
  await waitFor(() => expect(onAdopt).toHaveBeenCalledOnce());
  expect(adopt.mock.calls[1][1]).toBe("v2");
  expect(adopt.mock.calls[1][2]).not.toBe(adopt.mock.calls[0][2]);
});

it.each(["before refusal", "during refresh"])("clears a definitively refused attempt when closed %s", async (timing) => {
  const copy = { ...recovery, missingFiles: [] };
  let reject!: (reason: unknown) => void;
  let finishRead!: (value: LocalDraftRecovery) => void;
  const read = vi.fn<MailBackend["localDraftRecovery"]>().mockResolvedValueOnce(copy)
    .mockImplementationOnce(() => new Promise((resolve) => { finishRead = resolve; })).mockResolvedValue(copy);
  const adopt = vi.fn<MailBackend["adoptLocalDraftRecovery"]>(() => new Promise((_resolve, fail) => { reject = fail; }));
  fakeMail({ localDraftRecoveryIndex: vi.fn(async () => [entry]), localDraftRecovery: read, adoptLocalDraftRecovery: adopt });
  const user = userEvent.setup(); const panel = () => <LocalDraftRecoveries accounts={[work]} onAdopt={vi.fn()} onResolve={vi.fn()} />;
  const view = render(panel());
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.selectOptions(await screen.findByRole("combobox"), work.id);
  await user.click(screen.getByRole("checkbox", { name: "I understand this opens a standalone email" }));
  await user.click(screen.getByRole("button", { name: "Open as new draft" }));
  if (timing === "before refusal") view.unmount();
  await act(async () => { reject({ kind: "localRecoveryRejected", message: "Changed" }); });
  if (timing === "during refresh") { await waitFor(() => expect(read).toHaveBeenCalledTimes(2));view.unmount();await act(async () => { finishRead(copy); }); }
  read.mockReset().mockResolvedValue(copy);
  render(panel());
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  expect(await screen.findByRole("combobox")).toBeEnabled();
  expect(screen.getByRole("button", { name: "Open as new draft" })).toBeDisabled();
});

it("does not duplicate an open entry when refreshing an ambiguous adoption", async () => {
  const copy = { ...recovery, missingFiles: [] };
  fakeMail({ localDraftRecoveryIndex: vi.fn(async () => [entry]), localDraftRecovery: vi.fn(async () => copy), adoptLocalDraftRecovery: vi.fn(async () => { throw new Error("reply lost"); }) });
  const user = userEvent.setup();render(<LocalDraftRecoveries accounts={[work]} onAdopt={vi.fn()} onResolve={vi.fn()} />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.selectOptions(await screen.findByRole("combobox"), work.id);
  await user.click(screen.getByRole("checkbox", { name: "I understand this opens a standalone email" }));
  await user.click(screen.getByRole("button", { name: "Open as new draft" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("reply lost");
  await user.click(screen.getByRole("button", { name: "Refresh recoveries" }));
  await screen.findByRole("button", { name: "Review local recovery" });
  expect(screen.getAllByRole("button", { name: "Review local recovery" })).toHaveLength(1);
});

it("shares an in-flight adoption across remounts and reports completion only once", async () => {
  const copy = { ...recovery, missingFiles: [] };
  let finish!: (value: typeof copy.draft) => void;
  const adopt = vi.fn<MailBackend["adoptLocalDraftRecovery"]>(() => new Promise((resolve) => { finish = resolve; }));
  fakeMail({ localDraftRecoveryIndex: vi.fn(async () => [entry]), localDraftRecovery: vi.fn(async () => copy), adoptLocalDraftRecovery: adopt });
  const user = userEvent.setup();const onAdopt = vi.fn();
  const panel = () => <LocalDraftRecoveries accounts={[work]} onAdopt={onAdopt} onResolve={vi.fn()} />;
  const view = render(panel());
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.selectOptions(await screen.findByRole("combobox"), work.id);
  await user.click(screen.getByRole("checkbox", { name: "I understand this opens a standalone email" }));
  await user.click(screen.getByRole("button", { name: "Open as new draft" }));
  view.unmount();render(panel());
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.click(await screen.findByRole("button", { name: "Open as new draft" }));
  expect(adopt).toHaveBeenCalledOnce();
  await act(async () => { finish({ ...copy.draft, id: "adopted" }); });
  expect(onAdopt).toHaveBeenCalledOnce();
});

it("allows discarding after an ambiguous failed adoption and forgets the retry only on success", async () => {
  const copy = { ...recovery, missingFiles: [] };
  const index = vi.fn(async () => [entry]);
  const discard = vi.fn<MailBackend["discardLocalDraftRecovery"]>().mockRejectedValueOnce(new Error("still unavailable")).mockResolvedValueOnce();
  const adopt = vi.fn<MailBackend["adoptLocalDraftRecovery"]>(async () => { throw new Error("disk full"); });
  fakeMail({ localDraftRecoveryIndex: index, localDraftRecovery: vi.fn(async () => copy), adoptLocalDraftRecovery: adopt, discardLocalDraftRecovery: discard });
  const onResolve = vi.fn();const user = userEvent.setup();
  const view = render(<LocalDraftRecoveries accounts={[work]} onAdopt={vi.fn()} onResolve={onResolve} />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.selectOptions(await screen.findByRole("combobox"), work.id);
  await user.click(screen.getByRole("checkbox", { name: "I understand this opens a standalone email" }));
  await user.click(screen.getByRole("button", { name: "Open as new draft" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("disk full");
  await user.click(screen.getByRole("button", { name: "Discard local recovery" }));
  await user.click(screen.getByRole("button", { name: "Confirm discard" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("still unavailable");
  await user.click(screen.getByRole("button", { name: "Confirm discard" }));
  expect(onResolve).toHaveBeenCalledWith("local", "v1", null);
  index.mockResolvedValue([]);
  view.unmount();
  render(<LocalDraftRecoveries accounts={[work]} onAdopt={vi.fn()} onResolve={onResolve} />);
  await waitFor(() => expect(screen.queryByRole("region", { name: "Local draft recoveries" })).not.toBeInTheDocument());
});

it("keeps a delivery-specific duplicate-send warning with a recovery copy", async () => {
  const copied = { ...recovery, id: "delivery-fixture" };
  fakeMail({ localDraftRecoveryIndex: vi.fn(async () => [{ ...entry, id: copied.id }]), localDraftRecovery: vi.fn(async () => copied) });
  const user = userEvent.setup(); render(<LocalDraftRecoveries accounts={[work]} onAdopt={vi.fn()} onResolve={vi.fn()} />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  expect(await screen.findByText(/copy from delivery recovery/)).toBeVisible();
  expect(screen.getByText(/The original may already have been sent/)).toBeVisible();
});

it("refreshes its index without losing an independent review's choices", async () => {
  const { backend }=fakeMail({ localDraftRecoveryIndex:vi.fn(async()=>[entry]), localDraftRecovery:vi.fn(async()=>recovery) });
  const user=userEvent.setup();const props={accounts:[work],onAdopt:vi.fn(),onResolve:vi.fn()};
  const view=render(<LocalDraftRecoveries {...props} refresh={0} />);
  await user.click(await screen.findByRole("button",{name:"Review local recovery"}));
  await user.selectOptions(screen.getByRole("combobox",{name:"Save recovery to account"}),work.id);
  await user.click(screen.getByRole("checkbox",{name:"I understand this opens a standalone email"}));
  view.rerender(<LocalDraftRecoveries {...props} refresh={1} />);
  await waitFor(()=>expect(backend.localDraftRecoveryIndex).toHaveBeenCalledTimes(2));
  expect(screen.getByRole("combobox",{name:"Save recovery to account"})).toHaveValue(work.id);
  expect(screen.getByRole("checkbox",{name:"I understand this opens a standalone email"})).toBeChecked();
});
