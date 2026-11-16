import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import type { MailBackend } from "../services/mail";
import { fakeMail } from "../test/fakeMail";
import { resetStore, work } from "../test/fixtures";
import { draftsSaved, keepDraft, localRecoveryFor, resolveLocalRecovery, watchDrafts } from "../state/drafts";
import { useStore } from "../state/store";
import { DraftRecoveryNotice } from "./DraftRecoveryNotice";

async function conflict(fail = false) {
  resetStore([], [work]);vi.spyOn(console, "warn").mockImplementation(() => {});
  const keep = vi.fn<MailBackend["keepLocalDraftRecovery"]>(async (id, _previous, draft) => ({ status: "saved", recovery: { id, version: "v1", draft, missingFiles: [], updatedAt: 1 } }));
  if (fail) keep.mockRejectedValueOnce(new Error("disk full"));
  fakeMail({ saveDraft: vi.fn(async () => { throw { kind: "draftConflict" }; }), keepLocalDraftRecovery: keep });
  const stop = watchDrafts(useStore);
  useStore.getState().compose("new");useStore.getState().updateDraft({ body: "Local text" });
  keepDraft(useStore.getState().draft!);await draftsSaved();
  return { stop, keep, draft: useStore.getState().draft! };
}

it("offers review for a durably kept local conflict", async () => {
  const { stop, draft } = await conflict(); const review = vi.fn(); const user = userEvent.setup();
  try {
    render(<DraftRecoveryNotice draft={draft} onReview={review} />);
    expect(screen.getByRole("status")).toHaveTextContent("kept separately");
    await user.click(screen.getByRole("button", { name: "Review in Drafts" }));expect(review).toHaveBeenCalledOnce();
  } finally { stop(); }
});

it("offers retry without requiring another keystroke after persistence fails", async () => {
  const { stop, draft, keep } = await conflict(true); const user = userEvent.setup();
  try {
    render(<DraftRecoveryNotice draft={draft} onReview={vi.fn()} />);
    expect(screen.getByRole("status")).toHaveTextContent("Keep this window open");
    expect(screen.getByRole("status")).toHaveTextContent("disk full");
    await user.click(screen.getByRole("button", { name: "Retry local recovery" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("kept separately"));
    expect(keep).toHaveBeenCalledTimes(2);
  } finally { stop(); }
});

it("lets post-resolution typing be kept separately only through the explicit button", async () => {
  const { stop } = await conflict(); const user = userEvent.setup();
  try {
    const old = localRecoveryFor(useStore.getState().draft!)!.writer.id;
    useStore.getState().updateDraft({ body: "Newer text" });
    const draft = useStore.getState().draft!;
    resolveLocalRecovery(old, "v1", null);
    render(<DraftRecoveryNotice draft={draft} onReview={vi.fn()} />);
    expect(screen.getByRole("status")).toHaveTextContent("resolved");
    await user.click(screen.getByRole("button", { name: "Keep new edits separately" }));
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("kept separately"));
    expect(localRecoveryFor(draft)!.writer.id).not.toBe(old);
  } finally { stop(); }
});

it("shows ordinary pending typing without a false persistence failure", async () => {
  const { stop } = await conflict();
  try {
    useStore.getState().updateDraft({ body: "Typing more" });
    render(<DraftRecoveryNotice draft={useStore.getState().draft!} onReview={vi.fn()} />);
    expect(screen.getByRole("status")).toHaveTextContent("Waiting to save your latest changes");
    expect(screen.queryByRole("button", { name: "Retry local recovery" })).not.toBeInTheDocument();
  } finally { stop(); }
});

it("offers an explicit retry if the editor stopped watching before its debounce saved", async () => {
  const { stop } = await conflict();
  useStore.getState().updateDraft({ body: "Unflushed edits" });
  stop();
  render(<DraftRecoveryNotice draft={useStore.getState().draft!} onReview={vi.fn()} />);
  expect(screen.getByRole("status")).toHaveTextContent("Local recovery is not confirmed");
  expect(screen.getByRole("button", { name: "Retry local recovery" })).toBeEnabled();
});
