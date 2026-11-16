import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useScheduled, type ScheduledEmail } from "../state/scheduled";
import { useStore } from "../state/store";
import { fakeMail } from "../test/fakeMail";
import { lena, personal, resetStore, thread, work } from "../test/fixtures";
import { ScheduledScreen } from "./ScheduledScreen";
import { changeWhatNoListShows, renderCounted } from "../test/renders";

const s = () => useStore.getState();
const rows = () => screen.queryAllByRole("option");
const selected = () => rows().find((r) => r.getAttribute("aria-selected") === "true");

// Tuesday 3 November 2026, 1:20 PM.
const now = new Date(2026, 10, 3, 13, 20).getTime();
const saturday8 = new Date(2026, 10, 7, 8, 0).getTime();

const email = (id: string, extra: Partial<ScheduledEmail["draft"]> = {}, more: Partial<ScheduledEmail> = {}): ScheduledEmail => ({
  revision: 1,
  operationId: `op-${id}`,
  draft: {
    id,
    mode: "new",
    fromAccountId: "work",
    to: [lena],
    cc: [],
    bcc: [],
    subject: `Email ${id}`,
    body: "See you then.",
    bodyHtml: "",
    fieldsOpen: true,
    showCc: false,
    showBcc: false,
    remindIfNoReply: false,
    signatureId: null,
    updatedAt: 1,
    ...extra,
  },
  sendAt: saturday8,
  nextTry: saturday8,
  tries: 0,
  ...more,
});

beforeEach(() => {
  vi.useFakeTimers({ now, shouldAdvanceTime: true });
  resetStore([thread("q4")], [work, personal]);
  useScheduled.setState({
    emails: [
      email("a", { subject: "Lunch on Friday?" }),
      email("b", { fromAccountId: "personal", subject: " ", to: [] }, { tries: 2, error: "can’t reach the server", nextTry: new Date(2026, 10, 3, 13, 25).getTime(), sendAt: now - 60_000 }),
    ],
    cursorId: undefined,
    picking: undefined,
  });
  useStore.setState({ view: "inbox" });
});
afterEach(() => vi.useRealTimers());

describe("ScheduledScreen", () => {
  it("lists what waits to send later: who to, the subject and when it goes", () => {
    useScheduled.setState({ cursorId: "a" });
    render(<ScheduledScreen />);
    expect(screen.getByRole("heading", { name: "Scheduled" })).toBeInTheDocument();
    expect(screen.getByText(/wait on this Mac/)).toBeInTheDocument();
    expect(rows()).toHaveLength(2);
    expect(rows()[0]).toHaveTextContent("To Lena Vogt");
    expect(rows()[0]).toHaveTextContent("Lunch on Friday?");
    // The selected row shows its keys in place of the time.
    expect(rows()[1]).toHaveTextContent("No one yet");
    expect(rows()[1]).toHaveTextContent("No subject");
    expect(rows()[1]).toHaveTextContent("1:19 PM");
    expect(rows()[1]).toHaveTextContent("Can’t reach the server. Trying again at 1:25 PM.");
    expect(rows()[1]).toHaveTextContent("Not sent yet");
    expect(rows()[0]).not.toHaveTextContent("Not sent yet");
  });

  it("shows one account's, and says so", () => {
    useStore.setState({ accountFilter: "personal" });
    render(<ScheduledScreen />);
    expect(screen.getByRole("heading", { name: `Scheduled in ${personal.name}` })).toBeInTheDocument();
    expect(rows()).toHaveLength(1);
  });

  it("says what to do when nothing is scheduled", async () => {
    useScheduled.setState({ emails: [] });
    render(<ScheduledScreen />);
    expect(screen.getByText("Nothing scheduled")).toBeInTheDocument();
    expect(screen.getByText(/⌘⇧↵|Ctrl\+Shift\+↵/)).toBeInTheDocument();
    // Nothing to edit, send or put back, so the footer doesn't offer it.
    for (const gone of ["Edit", "New time", "Send now", "Back to drafts"]) expect(screen.queryByText(gone)).not.toBeInTheDocument();
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).click(screen.getByRole("button", { name: /New email/ }));
    expect(s().view).toBe("compose");
  });

  it("is run from the keyboard", async () => {
    const { backend } = fakeMail({ unscheduleSend: vi.fn(async (id: string) => ({ status: "cancelled" as const, restored: useScheduled.getState().emails.find((email) => email.draft.id === id)!.draft })) });
    useStore.setState({ view: "scheduled", history: ["inbox"] });
    useScheduled.setState({ cursorId: "a" });
    render(<ScheduledScreen />);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{ArrowDown}");
    expect(selected()).toHaveTextContent("No subject");
    await user.keyboard("{ArrowUp}");
    expect(selected()).toHaveTextContent("Lunch on Friday?");

    await user.keyboard("h");
    expect(useScheduled.getState().picking).toEqual({ kind: "move", id: "a", operationId: "op-a", revision: 1 });
    act(() => useScheduled.setState({ picking: undefined }));

    await user.keyboard("{Meta>}{Enter}{/Meta}");
    expect(backend.rescheduleSend).toHaveBeenCalledWith("a", expect.any(Number), "op-a", 1);

    await user.keyboard("{Delete}");
    expect(backend.unscheduleSend).toHaveBeenCalledWith("a", "op-a");
    await act(() => vi.advanceTimersByTimeAsync(0));
    expect(rows()).toHaveLength(1);

    await user.keyboard("{Enter}");
    expect(backend.unscheduleSend).toHaveBeenCalledWith("b", "op-b");
    await act(() => vi.advanceTimersByTimeAsync(0));
    expect(s()).toMatchObject({ view: "compose", draft: { id: "b" } });
  });

  it("other keys: c writes a new email, / searches, Esc goes back, and ⌘ keys are left alone", async () => {
    fakeMail();
    useStore.setState({ view: "scheduled", history: ["inbox"] });
    const { unmount } = render(<ScheduledScreen />);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{Meta>}b{/Meta}");
    expect(s().view).toBe("scheduled");
    await user.keyboard("/");
    expect(s().view).toBe("search");
    act(() => useStore.setState({ view: "scheduled", history: ["inbox"] }));
    await user.keyboard("{Escape}");
    expect(s().view).toBe("inbox");
    act(() => useStore.setState({ view: "scheduled" }));
    await user.keyboard("c");
    expect(s()).toMatchObject({ view: "compose", draft: { mode: "new" } });
    unmount();
  });

  it("keys wait while the time picker is open, and do nothing on an empty list", async () => {
    const { backend } = fakeMail();
    useScheduled.setState({ picking: { kind: "move", id: "a", operationId: "op-a", revision: 1 }, cursorId: "a" });
    const { unmount } = render(<ScheduledScreen />);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{Backspace}");
    expect(backend.unscheduleSend).not.toHaveBeenCalled();
    unmount();
    useScheduled.setState({ picking: undefined, emails: [], cursorId: undefined });
    render(<ScheduledScreen />);
    await user.keyboard("h{Enter}{Backspace}{Meta>}{Enter}{/Meta}");
    expect(backend.unscheduleSend).not.toHaveBeenCalled();
    expect(backend.rescheduleSend).not.toHaveBeenCalled();
    expect(useScheduled.getState().picking).toBeUndefined();
  });

  it("⌫ puts the selected one back in Drafts", async () => {
    const { backend } = fakeMail();
    useScheduled.setState({ cursorId: "b" });
    render(<ScheduledScreen />);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    // A key the list doesn't use does nothing.
    await user.keyboard("x");
    expect(backend.unscheduleSend).not.toHaveBeenCalled();
    await user.keyboard("{Backspace}");
    expect(backend.unscheduleSend).toHaveBeenCalledWith("b", "op-b");
  });

  it("each row's buttons and a click work too", async () => {
    const { backend } = fakeMail({ unscheduleSend: vi.fn(async (id: string) => ({ status: "cancelled" as const, restored: useScheduled.getState().emails.find((email) => email.draft.id === id)!.draft })) });
    useStore.setState({ view: "scheduled" });
    useScheduled.setState({ cursorId: "a" });
    render(<ScheduledScreen />);
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.click(screen.getByRole("button", { name: /New time/ }));
    expect(useScheduled.getState().picking).toEqual({ kind: "move", id: "a", operationId: "op-a", revision: 1 });
    act(() => useScheduled.setState({ picking: undefined }));
    await user.click(screen.getByRole("button", { name: /Send now/ }));
    expect(backend.rescheduleSend).toHaveBeenCalledWith("a", expect.any(Number), "op-a", 1);
    await user.click(screen.getByRole("button", { name: /Back to drafts/ }));
    expect(backend.unscheduleSend).toHaveBeenCalledWith("a", "op-a");
    await act(() => vi.advanceTimersByTimeAsync(0));
    act(() => useScheduled.setState({ cursorId: "b" }));
    await user.click(screen.getByRole("button", { name: /Edit/ }));
    await act(() => vi.advanceTimersByTimeAsync(0));
    expect(s()).toMatchObject({ view: "compose", draft: { id: "b" } });
  });

  it("a click on a row opens it to edit", async () => {
    const { backend } = fakeMail();
    render(<ScheduledScreen />);
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).click(screen.getByText("Lunch on Friday?"));
    expect(backend.unscheduleSend).toHaveBeenCalledWith("a", "op-a");
  });
});

it.each([
  ["Sending is waiting for delivery recovery. No new send attempt has been made.", now],
  ["offline", now - 60_000],
])("does not promise a retry at an elapsed deadline: %s", (error, nextTry) => {
  useScheduled.setState({ emails: [email("waiting", {}, { error, nextTry, sendAt: now, tries: 0 })] });
  render(<ScheduledScreen />);
  expect(rows()[0]).not.toHaveTextContent("Trying again at");
  expect(rows()[0]).not.toHaveTextContent("..");
  expect(rows()[0]).toHaveTextContent(error === "offline" ? "Offline." : error);
});

it("a store change it doesn't show doesn't redraw it", () => {
  const { renders } = renderCounted(<ScheduledScreen />);
  changeWhatNoListShows();
  expect(renders).not.toHaveBeenCalled();
});
