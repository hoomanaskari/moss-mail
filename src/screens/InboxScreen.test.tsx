import { act, fireEvent, render, screen } from "@testing-library/react";
import { Profiler } from "react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { holdDelay } from "../state/useHeldKey";
import { personal, resetDemoStore, resetStore, thread, work } from "../test/fixtures";
import { useStore } from "../state/store";
import { useServerChanges } from "../services/serverChanges";
import { useSync } from "../services/live";
import { fakeMail } from "../test/fakeMail";
import { useDrafts } from "../state/drafts";
import { useScheduled, type ScheduledEmail } from "../state/scheduled";
import { InboxScreen } from "./InboxScreen";
import * as store from "../state/store";

// Spies on the list and count helpers, to see when the screen works them out again.
vi.mock("../state/store", async (importOriginal) => {
  const real = await importOriginal<typeof import("../state/store")>();
  return {
    ...real,
    visibleThreads: vi.fn(real.visibleThreads),
    splitCountsFor: vi.fn(real.splitCountsFor),
    unreadCountsFor: vi.fn(real.unreadCountsFor),
    accountCountsFor: vi.fn(real.accountCountsFor),
    progressFor: vi.fn(real.progressFor),
  };
});

const s = () => useStore.getState();
const button = (text: RegExp) => screen.getAllByRole("button").find((b) => text.test(b.textContent ?? ""))!;

beforeEach(() => resetDemoStore());

describe("InboxScreen", () => {
  describe("re-rendering", () => {
    const helpers = () => [store.visibleThreads, store.splitCountsFor, store.unreadCountsFor, store.accountCountsFor, store.progressFor].map((f) => vi.mocked(f));

    it("a store change it doesn't show doesn't re-render it", () => {
      const renders = vi.fn();
      render(
        <Profiler id="inbox" onRender={renders}>
          <InboxScreen />
        </Profiler>,
      );
      renders.mockClear();
      act(() => useStore.setState({ paletteQuery: "budget", replyDraft: "Thanks", previews: { x: null } }));
      expect(renders).not.toHaveBeenCalled();
    });

    it("moving the cursor doesn't sort or count the inbox again", () => {
      render(<InboxScreen />);
      const first = screen.getByRole("option", { selected: true });
      for (const f of helpers()) f.mockClear();
      act(() => s().moveCursor(1));
      expect(screen.getByRole("option", { selected: true })).not.toBe(first);
      for (const f of helpers()) expect(f).not.toHaveBeenCalled();
    });

    it("a changed email sorts and counts the inbox again", () => {
      render(<InboxScreen />);
      for (const f of helpers()) f.mockClear();
      act(() => s().markDone(s().cursorId));
      for (const f of helpers()) expect(f).toHaveBeenCalled();
    });
  });

  it("Drafts and Scheduled sit after the splits while they have something in them", async () => {
    useDrafts.setState({ drafts: [] });
    useScheduled.setState({ emails: [] });
    const { unmount } = render(<InboxScreen />);
    expect(screen.queryByRole("button", { name: /Scheduled/ })).not.toBeInTheDocument();
    unmount();
    const email = { draft: { id: "a", fromAccountId: "work" }, sendAt: 1, nextTry: 1, tries: 0 } as ScheduledEmail;
    useScheduled.setState({ emails: [email, { ...email, draft: { ...email.draft, id: "b", fromAccountId: "personal" } }] });
    render(<InboxScreen />);
    expect(screen.getByRole("button", { name: /Scheduled/ })).toHaveTextContent("2");
    await userEvent.click(screen.getByRole("button", { name: /Scheduled/ }));
    expect(s().view).toBe("scheduled");
    useScheduled.setState({ emails: [] });
  });

  it("says which accounts can't sync, and a signed-out one offers to sign in again", async () => {
    const user = userEvent.setup();
    useSync.setState({
      problems: {
        personal: { problem: "signedOut", message: "no saved sign-in" },
        studio: { problem: "offline", message: "no network" },
        work: { problem: "server", message: "server error" },
        gone: { problem: "signedOut", message: "an account removed since" },
      },
    });
    render(<InboxScreen />);
    expect(screen.getByRole("tab", { name: /Personal/ })).toHaveTextContent("Not syncing");
    expect(screen.getByRole("tab", { name: /Studio/ })).toHaveTextContent("Offline");
    expect(screen.getByRole("tab", { name: /Work/ })).toHaveTextContent("Not syncing");
    expect(
      screen.getByText("Personal isn’t syncing: Moss is signed out of maya.ross@fastmail.com, so new mail can’t come in."),
    ).toBeInTheDocument();
    // A server problem says so too, with what went wrong; being offline needs no notice.
    expect(screen.getByText("Work isn’t syncing. Moss keeps trying.")).toBeInTheDocument();
    expect(screen.getByText("Why: server error")).toBeInTheDocument();
    expect(screen.getAllByText(/isn’t syncing/)).toHaveLength(2);
    expect(screen.queryByText("Why: no network")).not.toBeInTheDocument();
    // What the server said, so a sign-out can be explained.
    expect(screen.getByText("Why: no saved sign-in")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Sign in again" }));
    expect(s().view).toBe("add-account");
  });

  it("checks every account for new mail when asked", async () => {
    const { backend } = fakeMail();
    render(<InboxScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Check for new mail" }));
    expect(backend.syncNow).toHaveBeenCalledWith();
  });

  it("shows every account’s inbox by default", () => {
    render(<InboxScreen />);
    expect(screen.getByText("Inbox")).toBeInTheDocument();
    expect(screen.queryByText(/Showing only/)).not.toBeInTheDocument();
  });

  it("opens the command palette from the hint", async () => {
    render(<InboxScreen />);
    await userEvent.setup().click(button(/Ask AI or run a command/));
    expect(s().paletteOpen).toBe(true);
  });

  it("filtering to one account shows where you are in the title and the cards, with no extra bar", () => {
    s().setAccountFilter("work");
    render(<InboxScreen />);
    expect(screen.getByText("Work inbox")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: new RegExp(work.name) })).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByText(/Showing only/)).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Show all/ })).not.toBeInTheDocument();
  });

  it("an empty split explains itself and offers the next steps", async () => {
    s().setAccountFilter("work");
    s().setSplit("calendar");
    const { rerender } = render(<InboxScreen />);
    expect(screen.getByText("No invites in Work")).toBeInTheDocument();
    const user = userEvent.setup();
    await user.click(button(/Next: Newsletters/));
    expect(s().split).toBe("newsletters");
    s().setSplit("calendar");
    rerender(<InboxScreen />);
    await user.click(button(/Show all accounts/));
    expect(s().accountFilter).toBe("all");
  });

  it("account badges and today’s progress count the mail you have, not the design’s numbers", () => {
    resetStore([
      thread("reply"),
      thread("snoozed", { section: "later" }),
      thread("news", { split: "newsletters", section: "info", accountId: "personal" }),
    ]);
    useStore.setState({ doneThreads: [thread("done", { doneAt: Date.now() })] });
    render(<InboxScreen />);
    expect(screen.getByRole("tab", { name: /All accounts.*2/ })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: new RegExp(`${work.name}.*1`) })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: new RegExp(`${personal.name}.*1`) })).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "1 of 3 done" })).toBeInTheDocument();
    expect(screen.getByText("2 left")).toBeInTheDocument();
  });

  it("a tab with unread mail says so, even when you are on another tab", () => {
    resetStore([thread("snoozed", { section: "later" }), thread("news", { split: "newsletters", section: "info", unread: true })]);
    render(<InboxScreen />);
    expect(screen.getByRole("tab", { name: /Newsletters/ })).toHaveAccessibleName(/1 unread/);
    expect(screen.getByRole("tab", { name: /Important/ })).not.toHaveAccessibleName(/unread/);
  });

  it("the progress opens the Done list", async () => {
    render(<InboxScreen />);
    await userEvent.click(screen.getByRole("button", { name: /See what you’ve done/ }));
    expect(s().view).toBe("done");
  });

  it("an empty split across all accounts doesn’t offer “Show all accounts”", () => {
    useStore.setState({ threads: [] });
    render(<InboxScreen />);
    expect(screen.getByText("Nothing needs you")).toBeInTheDocument();
    expect(screen.queryByText("Show all accounts")).not.toBeInTheDocument();
  });

  it("with nothing waiting anywhere, the empty list offers inbox zero on Z", async () => {
    useStore.setState({ threads: [] });
    render(<InboxScreen />);
    // Today’s progress also says “Inbox zero”; this is the empty list’s step.
    await userEvent.click(button(/^ZInbox zero$/));
    expect(s().view).toBe("zero");
  });

  it("an account cleared while another still waits doesn’t offer inbox zero", () => {
    resetStore([thread("h", { accountId: "personal" })]);
    s().setAccountFilter("work");
    render(<InboxScreen />);
    expect(screen.getAllByRole("button").map((b) => b.textContent)).not.toContain("ZInbox zero");
  });

  it("an email reminded in Other leaves what's waiting there: apart under Coming back later, and out of the count", () => {
    resetStore([thread("a"), thread("o1", { split: "other", section: "info", subject: "Laptop update" }), thread("o2", { split: "other", section: "info" })]);
    s().setSplit("other");
    const { rerender } = render(<InboxScreen />);
    expect(screen.getByRole("tab", { name: /Other/ })).toHaveTextContent("2");
    act(() => s().remindLater("o1", Date.now() + 86_400_000));
    rerender(<InboxScreen />);
    expect(screen.getByRole("tab", { name: /Other/ })).toHaveTextContent("1");
    const later = screen.getByRole("group", { name: "Coming back later" });
    expect(later).toHaveTextContent("Laptop update");
    expect(screen.getByRole("group", { name: "Emails" })).not.toHaveTextContent("Laptop update");
  });

  it("row buttons mark done, ask when to remind you, and reply", async () => {
    render(<InboxScreen />);
    const user = userEvent.setup();
    const first = s().cursorId!;
    await user.click(screen.getAllByRole("button", { name: /Done/ })[0]);
    expect(s().threads.some((t) => t.id === first)).toBe(false);
    const second = s().cursorId!;
    await user.click(screen.getAllByRole("button", { name: /Later/ })[0]);
    expect(s().remindingId).toBe(second);
    await user.click(screen.getAllByRole("button", { name: /Reply/ })[0]);
    expect(s().view).toBe("compose");
  });

  it("row buttons delete, and block a new sender", async () => {
    const jordan = { name: "Jordan Blake", email: "jordan@growthleads.example", initials: "JB" };
    resetStore([thread("a"), thread("n", { from: jordan, sender: "new" }), thread("k")]);
    render(<InboxScreen />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /Delete/ }));
    expect(s()).toMatchObject({ cursorId: "n", toast: { text: "Moved to Trash" } });
    await user.click(screen.getByRole("button", { name: /Block/ }));
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["n"]);
    expect(s().threads.map((t) => t.id)).toEqual(["k"]);
  });

  it("clicking an email opens it, clicking an account filters, + opens Add account", async () => {
    render(<InboxScreen />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("tab", { name: /Personal/ }));
    expect(s().accountFilter).toBe("personal");
    await user.click(screen.getByRole("tab", { name: /Newsletters/ }));
    expect(s().split).toBe("newsletters");
    await user.click(screen.getByRole("button", { name: "Add account" }));
    expect(s().view).toBe("add-account");
    await user.dblClick(screen.getAllByRole("option")[0]);
    expect(s().view).toBe("thread");
  });

  it("an account with several addresses shows each one's mail on its own", async () => {
    const icloud = {
      ...personal,
      addresses: [{ email: "hi@rossstudio.art", state: "added" as const, source: "you" as const, seen: 1, blocked: false }],
    };
    resetStore(
      [
        thread("a", { accountId: "personal", toAddress: "hi@rossstudio.art", subject: "Shoot" }),
        thread("b", { accountId: "personal", toAddress: personal.address, subject: "Rent" }),
        thread("c", { accountId: "work", subject: "Budget" }),
      ],
      [work, icloud],
    );
    s().setAccountFilter("personal");
    const { rerender } = render(<InboxScreen />);
    const bar = screen.getByRole("tablist", { name: "Addresses" });
    expect(bar).toHaveTextContent("All addresses2");
    await userEvent.click(screen.getByRole("tab", { name: /hi@rossstudio\.art/ }));
    expect(s().addressFilter).toBe("hi@rossstudio.art");
    rerender(<InboxScreen />);
    expect(screen.getByText("Shoot")).toBeInTheDocument();
    expect(screen.queryByText("Rent")).not.toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /hi@rossstudio\.art/ })).toHaveAttribute("aria-selected", "true");
    expect(screen.queryByText(/Showing only/)).not.toBeInTheDocument();
  });

  it("an account with one address has no address row", () => {
    resetStore([thread("c", { toAddress: work.address })], [work]);
    s().setAccountFilter("work");
    render(<InboxScreen />);
    expect(screen.queryByRole("tablist", { name: "Addresses" })).not.toBeInTheDocument();
  });

  describe("holding ⌘ (Ctrl off a Mac)", () => {
    const accounts = () => screen.getByRole("tablist", { name: "Accounts" });
    const hold = () => {
      fireEvent.keyDown(window, { key: "Control", ctrlKey: true });
      act(() => vi.advanceTimersByTime(holdDelay));
    };
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("shows each account's key until it is let go", () => {
      render(<InboxScreen />);
      hold();
      expect(accounts()).toHaveTextContent("Ctrl+0");
      expect(screen.getByRole("tab", { name: /Work/ })).toHaveTextContent("Ctrl+1");
      fireEvent.keyUp(window, { key: "Control" });
      expect(accounts().querySelector("kbd")).toBeNull();
    });

    it("not while the command palette is open, where ⌘1 doesn't switch accounts", () => {
      s().setPaletteOpen(true);
      render(<InboxScreen />);
      hold();
      expect(accounts().querySelector("kbd")).toBeNull();
    });
  });
});


it("renders persistent server recovery in the real inbox", () => {
  resetStore([]);
  useServerChanges.setState({ byAccount: { work: { unavailable: false, status: {
    accountId: "work", change: "0123456789abcdef0123456789abcdef", revision: 1, state: "blocked", failure: "uncertain", kind: "move",
    completed: 1, pending: 2, discarded: 0, dropped: 0,
  } } } });
  render(<InboxScreen />);
  expect(screen.getByRole("status", { name: "Work server change status" })).toHaveTextContent("Work: Move is waiting");
  expect(screen.getByRole("status", { name: "Work server change status" })).toHaveTextContent("2 remain pending");
});
