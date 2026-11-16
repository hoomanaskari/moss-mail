import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useDrafts, type SavedDraft } from "../state/drafts";
import { useStore } from "../state/store";
import { lena, personal, resetStore, thread, work } from "../test/fixtures";
import { fakeMail } from "../test/fakeMail";
import { DraftsScreen } from "./DraftsScreen";
import { changeWhatNoListShows, renderCounted } from "../test/renders";

const s = () => useStore.getState();
const rows = () => screen.queryAllByRole("option");
const selected = () => rows().find((r) => r.getAttribute("aria-selected") === "true");

const draft = (id: string, extra: Partial<SavedDraft> = {}): SavedDraft => ({
  id,
  mode: "new",
  fromAccountId: "work",
  to: [],
  cc: [],
  bcc: [],
  subject: "",
  body: "",
  bodyHtml: "",
  fieldsOpen: true,
  showCc: false,
  showBcc: false,
  remindIfNoReply: false,
  signatureId: null,
  updatedAt: Date.now(),
  ...extra,
});

beforeEach(() => {
  resetStore([thread("q4", { subject: "Q4 budget" })], [work, personal]);
  useDrafts.setState({
    drafts: [
      draft("a", { to: [lena], subject: "Lunch on Friday?", body: "Are you free?\nI’m buying." }),
      draft("b", { fromAccountId: "personal", mode: "reply", threadId: "q4", subject: "Re: Q4 budget", body: "Looks good" }),
      draft("c"),
      draft("d", { fromAccountId: "personal", to: [lena], cc: [lena], bcc: [lena] }),
    ],
  });
  s().openDrafts();
});

describe("DraftsScreen", () => {
  it("lists your unsent emails: who to, the subject and how it starts", () => {
    render(<DraftsScreen />);
    expect(screen.getByRole("heading", { name: "Drafts" })).toBeInTheDocument();
    expect(rows()).toHaveLength(4);
    expect(rows()[3]).toHaveTextContent("To Lena Vogt and 2 more");
    expect(rows()[0]).toHaveTextContent("To Lena Vogt");
    expect(rows()[0]).toHaveTextContent("Lunch on Friday?");
    expect(rows()[0]).toHaveTextContent("Are you free? I’m buying.");
    expect(rows()[2]).toHaveTextContent("No one yet");
    expect(rows()[2]).toHaveTextContent("No subject");
    expect(selected()).toHaveTextContent("Lunch on Friday?");
    expect(screen.getByText("Drafts save as you type, and show on your phone too. Sending one removes it.")).toBeInTheDocument();
  });

  it("says drafts stay on this Mac when every account in view keeps them here (Resend)", () => {
    const resend = { ...personal, connection: { provider: "resend" as const, summary: "Resend" } };
    act(() => useStore.setState({ accounts: [work, resend] }));
    act(() => s().setAccountFilter("personal"));
    render(<DraftsScreen />);
    expect(screen.getByText("Drafts are saved on this Mac as you type. Sending one removes it from here.")).toBeInTheDocument();
  });

  it("follows the account you are looking at", () => {
    act(() => s().setAccountFilter("personal"));
    act(() => s().openDrafts());
    render(<DraftsScreen />);
    expect(screen.getByRole("heading", { name: "Drafts in Personal" })).toBeInTheDocument();
    expect(rows()).toHaveLength(2);
  });

  it("the arrows move, J and K do nothing, ↵ opens the draft in the composer", async () => {
    const user = userEvent.setup();
    render(<DraftsScreen />);
    await user.keyboard("jJ");
    expect(selected()).toHaveTextContent("Lunch on Friday?");
    await user.keyboard("{ArrowDown}");
    expect(selected()).toHaveTextContent("Re: Q4 budget");
    await user.keyboard("kK");
    expect(selected()).toHaveTextContent("Re: Q4 budget");
    await user.keyboard("{ArrowDown}{ArrowUp}{ArrowUp}");
    expect(selected()).toHaveTextContent("Lunch on Friday?");
    await user.keyboard("{Enter}");
    expect(s()).toMatchObject({ view: "compose", draft: { id: "a" } });
  });

  it("O opens too, and a click opens the row clicked", async () => {
    const { unmount } = render(<DraftsScreen />);
    await userEvent.keyboard("o");
    expect(s().draft?.id).toBe("a");
    unmount();
    act(() => s().openDrafts());
    render(<DraftsScreen />);
    await userEvent.click(screen.getByText("Re: Q4 budget"));
    expect(s()).toMatchObject({ view: "compose", draft: { id: "b" }, openThreadId: "q4" });
  });

  it("⌫ or Delete deletes the selected draft, and so does its button", async () => {
    const user = userEvent.setup();
    render(<DraftsScreen />);
    await user.keyboard("{Backspace}");
    expect(useDrafts.getState().drafts.map((d) => d.id)).toEqual(["b", "c", "d"]);
    await user.keyboard("{Delete}");
    expect(useDrafts.getState().drafts.map((d) => d.id)).toEqual(["c", "d"]);
    await user.click(screen.getByRole("button", { name: "BackspaceDelete" }));
    await user.click(screen.getByRole("button", { name: "BackspaceDelete" }));
    expect(useDrafts.getState().drafts).toEqual([]);
    expect(screen.getByText("No drafts")).toBeInTheDocument();
  });

  it("the selected row offers Open", async () => {
    render(<DraftsScreen />);
    await userEvent.click(screen.getByRole("button", { name: "↵Open" }));
    expect(s().draft?.id).toBe("a");
  });

  it("hovering a row leaves the selection where it was; clicking one selects it", async () => {
    const { unmount } = render(<DraftsScreen />);
    await userEvent.hover(screen.getByText("Re: Q4 budget"));
    expect(selected()).toHaveTextContent("Lunch on Friday?");
    await userEvent.click(screen.getByText("Re: Q4 budget"));
    expect(useDrafts.getState().cursorId).toBe("b");
    unmount();
    act(() => s().back());
    render(<DraftsScreen />);
    expect(selected()).toHaveTextContent("Re: Q4 budget");
  });

  it("C starts a new email, / searches, Esc goes back", async () => {
    const user = userEvent.setup();
    const { unmount } = render(<DraftsScreen />);
    await user.keyboard("c");
    expect(s()).toMatchObject({ view: "compose", draft: { mode: "new" } });
    unmount();
    act(() => {
      s().back();
      s().openDrafts();
    });
    render(<DraftsScreen />);
    await user.keyboard("/");
    expect(s().view).toBe("search");
    act(() => s().leave());
    await user.keyboard("{Escape}");
    expect(s().view).toBe("inbox");
  });

  it("leaves ⌘ keys and typing alone", async () => {
    render(<DraftsScreen />);
    await userEvent.keyboard("{Control>}j{/Control}x");
    expect(selected()).toHaveTextContent("Lunch on Friday?");
    expect(s().view).toBe("drafts");
  });

  it("with no drafts, says where they come from and how to start one", async () => {
    useDrafts.setState({ drafts: [] });
    render(<DraftsScreen />);
    expect(screen.getByText("No drafts")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "CNew email" }));
    expect(s().view).toBe("compose");
  });

  it("the top bar's Back goes back", async () => {
    render(<DraftsScreen />);
    await userEvent.click(screen.getByRole("button", { name: "EscBack" }));
    expect(s().view).toBe("inbox");
  });
});

it("offers local recovery review from Drafts and opens an explicitly adopted fresh draft", async () => {
  const recovered = draft("local-source", { subject: "Recovered", body: "Local text" });
  const adopted = { ...recovered, id: "fresh-recovery" };
  fakeMail({
    localDraftRecoveryIndex: vi.fn(async () => [{ id: "recovery", version: "v1", originalId: recovered.id, updatedAt: 1 }]),
    localDraftRecovery: vi.fn(async () => ({ id: "recovery", version: "v1", draft: recovered, missingFiles: [], updatedAt: 1 })),
    adoptLocalDraftRecovery: vi.fn(async () => adopted),
    drafts: vi.fn(async () => [adopted]),
  });
  const user = userEvent.setup();render(<DraftsScreen />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.selectOptions(await screen.findByRole("combobox"), work.id);
  await user.click(screen.getByRole("checkbox", { name: "I understand this opens a standalone email" }));
  await user.click(screen.getByRole("button", { name: "Open as new draft" }));
  expect(s().view).toBe("compose");expect(s().draft).toEqual(adopted);
  expect(useDrafts.getState().drafts).toContainEqual(adopted);
});

it("refreshes ordinary drafts after explicitly discarding a local recovery", async () => {
  const { backend } = fakeMail({
    localDraftRecoveryIndex: vi.fn(async () => [{ id: "recovery", version: "v1", originalId: "old", updatedAt: 1 }]),
    localDraftRecovery: vi.fn(async () => undefined),
    discardLocalDraftRecovery: vi.fn(async () => {}),
    drafts: vi.fn(async () => [draft("winner", { body: "Winning draft" })]),
  });
  const user = userEvent.setup();render(<DraftsScreen />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.click(await screen.findByRole("button", { name: "Discard local recovery" }));
  await user.click(screen.getByRole("button", { name: "Confirm discard" }));
  await waitFor(() => expect(useDrafts.getState().drafts.map((copy) => copy.id)).toEqual(["winner"]));
  expect(backend.discardLocalDraftRecovery).toHaveBeenCalledWith("recovery", "v1");
});

it.each([false, true])("retains an adopted row and any newer local edits after leaving review: %s", async (alreadyEdited) => {
  const recovered = draft("source", { body: "Recovered text" });const adopted = { ...recovered, id: "new-adopted" };
  let finish!: (value: SavedDraft) => void;
  fakeMail({
    localDraftRecoveryIndex: vi.fn(async () => [{ id: "recovery", version: "v1", originalId: recovered.id, updatedAt: 1 }]),
    localDraftRecovery: vi.fn(async () => ({ id: "recovery", version: "v1", draft: recovered, missingFiles: [], updatedAt: 1 })),
    adoptLocalDraftRecovery: vi.fn(() => new Promise<SavedDraft>((resolve) => { finish = resolve; })),
    drafts: vi.fn(async () => [adopted]),
  });
  const user = userEvent.setup();const view = render(<DraftsScreen />);
  await user.click(await screen.findByRole("button", { name: "Review local recovery" }));
  await user.selectOptions(await screen.findByRole("combobox"), work.id);
  await user.click(screen.getByRole("checkbox", { name: "I understand this opens a standalone email" }));
  await user.click(screen.getByRole("button", { name: "Open as new draft" }));
  act(() => s().leave());view.unmount();
  const edited = { ...adopted, body: "Newer edits after sync showed the adopted draft" };
  if (alreadyEdited) useDrafts.setState({ drafts: [edited], unsaved: { [edited.id]: edited } });
  await act(async () => finish(adopted));
  await waitFor(() => expect(useDrafts.getState().drafts).toContainEqual(alreadyEdited ? edited : adopted));
  if (alreadyEdited) expect(useDrafts.getState().unsaved[edited.id]).toBe(edited);
  expect(s().view).toBe("inbox");
});

it("reviews delivery recovery from Drafts without letting list shortcuts steal its buttons", async () => {
  const { backend } = fakeMail({ deliveryAttention: vi.fn(async () => ({ entries: [{ id: "review", accountId: "work", state: "accepted" as const, interruptedFrom: null, createdAt: 1, fileCount: 0, fileBytes: 0 }], next: null })), resolveDelivery: vi.fn(async () => ({ status: "resolved" as const, recoveryId: null })) });
  const user = userEvent.setup(); render(<DraftsScreen />);
  const review = await screen.findByRole("button", { name: /^Review delivery for / });
  review.focus(); await user.keyboard("{Enter}");
  expect(await screen.findByText(/server accepted this email/)).toBeVisible();
  expect(s().view).toBe("drafts");
  await user.click(screen.getByRole("button", { name: "Discard recovery files" }));
  await user.click(screen.getByRole("checkbox", { name: "I understand I am discarding recovery files without creating a copy" }));
  await user.click(screen.getByRole("button", { name: "Confirm review" }));
  await waitFor(() => expect(backend.resolveDelivery).toHaveBeenCalledWith("review", "reviewed"));
  expect(backend.localDraftRecoveryIndex).toHaveBeenCalledOnce();
});


it("refreshes independent recoveries after preserving a delivery copy", async () => {
  const { backend } = fakeMail({ deliveryAttention: vi.fn(async () => ({ entries: [{ id: "copy-review", accountId: "work", state: "uncertain" as const, interruptedFrom: null, createdAt: 1, fileCount: 0, fileBytes: 0 }], next: null })), deliveryAttentionDraft: vi.fn(async () => draft("copy-source")), resolveDelivery: vi.fn(async () => ({ status: "resolved" as const, recoveryId: "delivery-copy" })) });
  const user = userEvent.setup(); render(<DraftsScreen />);
  await user.click(await screen.findByRole("button", { name: /^Review delivery for / }));
  await user.click(await screen.findByRole("button", { name: "Keep a recovery copy" }));
  await user.click(screen.getByRole("button", { name: "Confirm review" }));
  await waitFor(() => expect(backend.localDraftRecoveryIndex).toHaveBeenCalledTimes(2));
});

it("a store change it doesn't show doesn't redraw it", () => {
  const { renders } = renderCounted(<DraftsScreen />);
  changeWhatNoListShows();
  expect(renders).not.toHaveBeenCalled();
});
