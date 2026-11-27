import { act, fireEvent, renderHook } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { closeWindow } from "../lib/window";
import { draft, lena, personal, resetStore, thread, work } from "../test/fixtures";
import { useDrafts } from "./drafts";
import { useQuickReply } from "./quickReply";
import { digitFroms, pressDigit } from "../test/keys";
import { resetAskPanel, useAskPanel } from "./askPanel";
import { useScheduled } from "./scheduled";
import { openFromTrash, openTrash, useTrash } from "./trash";
import { sendBeatMs, useStore } from "./store";
import { useShortcuts } from "./useShortcuts";

vi.mock("../lib/window", () => ({ closeWindow: vi.fn() }));

const s = () => useStore.getState();

async function sendingDuring(action: () => Promise<void>) {
  const seen: boolean[] = [];
  const stop = useStore.subscribe((state, previous) => {
    if (!!state.sending !== !!previous.sending) seen.push(!!state.sending);
  });
  try {
    await action();
    return seen;
  } finally {
    stop();
  }
}

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  resetStore([
    thread("a", { replyOptions: [{ label: "Yes", draft: "Yes." }, { label: "No", draft: "No." }] }),
    thread("b"),
  ]);
  resetAskPanel();
  useScheduled.setState({ picking: undefined });
  renderHook(() => useShortcuts());
});

afterEach(() => {
  vi.clearAllTimers();
  vi.useRealTimers();
});

describe("single keys", () => {
  it("reading deleted mail: ↑/↓ go through Trash, E puts it back in the inbox, ⌫ does nothing", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    useTrash.setState({ threads: [thread("x"), thread("y")] });
    openTrash();
    await user.keyboard("{ArrowDown}");
    expect(s().view).toBe("trash");
    openFromTrash("x");
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(s().openThreadId).toBe("y");
    await user.keyboard("{ArrowUp}{Backspace}{Delete}hm");
    expect(s().openThreadId).toBe("x");
    expect(s().remindingId ?? s().movingId).toBeUndefined();
    // Any other key works as it does on any email: R replies.
    await user.keyboard("r");
    expect(s()).toMatchObject({ view: "compose", draft: { threadId: "x" } });
    act(() => useStore.setState({ view: "thread" }));
    expect(useTrash.getState().threads.map((t) => t.id)).toEqual(["x", "y"]);
    await user.keyboard("e");
    expect(s().threads.map((t) => t.id)).toEqual(["x", "a", "b"]);
    expect(useTrash.getState().threads.map((t) => t.id)).toEqual(["y"]);
  });

  it("↓ / ↑ move, Enter opens, e marks done", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{ArrowDown}");
    expect(s().cursorId).toBe("b");
    await user.keyboard("{ArrowUp}{Enter}");
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a" });
    await user.keyboard("e");
    expect(s().threads.map((t) => t.id)).toEqual(["b"]);
  });

  it("h opens the Remind menu for the email you're on, and other keys wait while it is open", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{ArrowDown}h");
    expect(s().remindingId).toBe("b");
    expect(s().threads.find((t) => t.id === "b")?.section).not.toBe("later");
    await user.keyboard("e{Meta>}k{/Meta}");
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b"]);
    expect(s().paletteOpen).toBe(false);
    s().closeRemind();
    await user.keyboard("{Enter}h");
    expect(s().remindingId).toBe("b");
  });

  it("m opens the Move menu for the email you're on or reading, and other keys wait while it is open", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{ArrowDown}m");
    expect(s().movingId).toBe("b");
    await user.keyboard("e");
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b"]);
    s().closeMove();
    await user.keyboard("{ArrowUp}{Enter}m");
    expect(s().movingId).toBe("a");
  });

  it("m does nothing where there's no email to move, such as inbox zero", async () => {
    useStore.setState({ view: "zero" });
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).keyboard("m");
    expect(s().movingId).toBeUndefined();
  });

  it("⇧H waits on your latest email, rather than opening the Remind menu", async () => {
    resetStore([thread("w", { youWroteLast: true, lastMessageAt: Date.now() }), thread("b")]);
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).keyboard("{Shift>}H{/Shift}");
    expect(s().remindingId).toBeUndefined();
    expect(s().threads[0]).toMatchObject({ section: "later", waiting: { why: { by: "you" } } });
  });

  it("J and K do nothing: only the arrow keys move", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const j = new KeyboardEvent("keydown", { key: "j", bubbles: true, cancelable: true });
    window.dispatchEvent(j);
    expect(j.defaultPrevented).toBe(false);
    await user.keyboard("jJ");
    expect(s().cursorId).toBe("a");
    await user.keyboard("{ArrowDown}kK");
    expect(s().cursorId).toBe("b");
  });

  it("⌫ and Delete move the email to Trash, from the list or while reading", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{Backspace}");
    expect(s()).toMatchObject({ toast: { text: "Moved to Trash" }, cursorId: "b" });
    await user.keyboard("{Meta>}z{/Meta}{Enter}{Delete}");
    expect(s().threads.map((t) => t.id)).toEqual(["b"]);
    expect(s()).toMatchObject({ view: "thread", openThreadId: "b" });
  });

  it("d opens the Done list, which then handles its own keys", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("d");
    expect(s().view).toBe("done");
    await user.keyboard("{Escape}e");
    expect(s().view).toBe("done");
  });

  it("⌘Z undoes the last done or snooze, from the list, an email or inbox zero", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("e{Meta>}z{/Meta}");
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b"]);
    await user.keyboard("{Enter}");
    s().remindLater();
    await user.keyboard("{Meta>}z{/Meta}");
    expect(s().threads[0].section).toBe("reply");
    await user.keyboard("ee");
    expect(s().view).toBe("zero");
    await user.keyboard("{Meta>}z{/Meta}");
    expect(s()).toMatchObject({ view: "thread", openThreadId: "b" });
  });

  it("⌘Z is left to the text field you are typing in", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("e");
    const field = document.body.appendChild(document.createElement("input"));
    field.focus();
    await user.keyboard("{Meta>}z{/Meta}");
    expect(s().threads.map((t) => t.id)).toEqual(["b"]);
    field.remove();
    // ⇧⌘Z (redo) isn't undo.
    await user.keyboard("{Meta>}{Shift>}z{/Shift}{/Meta}");
    expect(s().threads.map((t) => t.id)).toEqual(["b"]);
  });

  it("r replies, ⇧R replies to all, f forwards, c starts a new email", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("r");
    expect(s().draft?.mode).toBe("reply");
    s().back();
    await user.keyboard("{Shift>}R{/Shift}");
    expect(s().draft?.mode).toBe("reply-all");
    s().back();
    await user.keyboard("f");
    expect(s().draft?.mode).toBe("forward");
    s().back();
    await user.keyboard("c");
    expect(s().draft?.mode).toBe("new");
  });

  it("Tab cycles splits in the inbox and instant replies in a thread", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{Tab}");
    expect(s().split).toBe("other");
    s().setSplit("important");
    s().open("a");
    await user.keyboard("{Tab}");
    expect(s().replyChoice).toBe(1);
    await user.keyboard("{Tab}");
    expect(s().replyChoice).toBe(0);
  });

  it("are ignored while typing in a field", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const input = document.body.appendChild(document.createElement("input"));
    await user.click(input);
    await user.keyboard("{ArrowDown}e{Backspace}{Delete}");
    expect(s()).toMatchObject({ cursorId: "a", view: "inbox" });
    expect(s().threads).toHaveLength(2);
    input.remove();
  });

  it("are left to the palette while it is open", async () => {
    s().setPaletteOpen(true);
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).keyboard("{ArrowDown}");
    expect(s().cursorId).toBe("a");
  });
});

describe("everywhere", () => {
  it("⌘K opens the palette and / opens Search", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{Meta>}k{/Meta}");
    expect(s().paletteOpen).toBe(true);
    s().setPaletteOpen(false);
    await user.keyboard("/");
    expect(s()).toMatchObject({ view: "search", paletteOpen: false });
  });

  it("⌘, opens Settings, ? the shortcut list and ⌘N Add account", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{Meta>},{/Meta}");
    expect(s()).toMatchObject({ view: "settings", settingsSection: "accounts" });
    s().leave();
    await user.keyboard("?");
    expect(s()).toMatchObject({ view: "settings", settingsSection: "keyboard" });
    s().leave();
    await user.keyboard("{Meta>}n{/Meta}");
    expect(s().view).toBe("add-account");
  });

  it("leaves Add account, Settings, Search and Senders to handle their own keys", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    for (const open of [s().openAddAccount, s().openSettings, s().openSearch, s().openSenders]) {
      open();
      const view = s().view;
      await user.keyboard("{ArrowDown}e{Escape}{Meta>}2{/Meta}{Meta>}n{/Meta}?");
      expect(s()).toMatchObject({ view, cursorId: "a", accountFilter: "all" });
      expect(s().threads).toHaveLength(2);
      await user.keyboard("{Meta>}k{/Meta}");
      expect(s().paletteOpen).toBe(true);
      s().setPaletteOpen(false);
      s().leave();
    }
  });

  it("⌘1–9 pick an account and ⌘0 shows them all", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{Meta>}2{/Meta}");
    expect(s().accountFilter).toBe("personal");
    await user.keyboard("{Meta>}9{/Meta}");
    expect(s().accountFilter).toBe("personal");
    await user.keyboard("{Meta>}0{/Meta}");
    expect(s().accountFilter).toBe("all");
  });

  it("Esc closes recipient details first, then goes back", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    s().open("a");
    s().toggleRecipients();
    await user.keyboard("{Escape}");
    expect(s()).toMatchObject({ view: "thread", recipientsOpen: false });
    await user.keyboard("{Escape}");
    expect(s().view).toBe("inbox");
    // The inbox is as far back as it goes.
    const back = vi.spyOn(s(), "back");
    await user.keyboard("{Escape}");
    expect(back).not.toHaveBeenCalled();
    expect(s().view).toBe("inbox");
  });
});

describe("remote images", () => {
  it("⇧I shows images from the open email's sender from now on; i still shows the recipients", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{Shift>}I{/Shift}");
    expect(s().threads[0].messages[0].imageTrust).toBeUndefined();
    s().open("a");
    await user.keyboard("{Shift>}I{/Shift}");
    expect(s().threads[0].messages[0].imageTrust).toBe("you-allowed");
    expect(s()).toMatchObject({ imagesShown: [], recipientsOpen: false });
    await user.keyboard("i");
    expect(s().recipientsOpen).toBe(true);
  });

  it("⌥I shows the open email's images just this once", () => {
    // On a Mac ⌥I types a dead key, so only the key's position says it was I.
    const altI = () => fireEvent.keyDown(window, { key: "ˆ", code: "KeyI", altKey: true });
    altI();
    expect(s().imagesShown).toEqual([]);
    s().open("a");
    altI();
    expect(s().imagesShown).toEqual(["a-1"]);
    expect(s().threads[0].messages[0].imageTrust).toBeUndefined();
  });

  it("⇧I and Tab work on a done email you are reading", async () => {
    useStore.setState({ doneThreads: [thread("x", { doneAt: 1, replyOptions: [{ label: "A", draft: "A" }, { label: "B", draft: "B" }] })] });
    s().openDone();
    s().open("x");
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{Shift>}I{/Shift}{Tab}");
    expect(s().doneThreads[0].messages[0].imageTrust).toBe("you-allowed");
    expect(s().replyChoice).toBe(1);
  });

  it("does nothing for an email with no messages", async () => {
    resetStore([thread("a", { messages: [] })]);
    s().open("a");
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).keyboard("{Shift>}I{/Shift}");
    fireEvent.keyDown(window, { key: "ˆ", code: "KeyI", altKey: true });
    expect(s().imagesShown).toEqual([]);
  });
});

describe("composer", () => {
  it("⌘⇧↵ asks when to send it, and other keys wait while it asks", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    s().compose("reply", "a");
    s().updateDraft({ to: [lena] });
    await user.keyboard("{Meta>}{Shift>}{Enter}{/Shift}{/Meta}");
    expect(useScheduled.getState().picking).toEqual({ kind: "compose" });
    expect(s().view).toBe("compose");
    expect(s().sending).toBeFalsy();
    await user.keyboard("{Meta>}{Enter}{/Meta}");
    expect(s().sending, "Send waits too").toBeFalsy();
    useScheduled.setState({ picking: undefined });
  });

  it("⌘↵ sends and ⌘⇧B shows BCC", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    s().compose("reply", "a");
    await user.keyboard("{Meta>}{Shift>}b{/Shift}{/Meta}");
    expect(s().draft).toMatchObject({ showBcc: true, fieldsOpen: true });
    expect(await sendingDuring(() => user.keyboard("{Meta>}{Enter}{/Meta}"))).toContain(true);
  });

  it("⌥S opens and closes the signature picker", () => {
    s().compose("new");
    // A Mac types "ß" for ⌥S, so the key is read from e.code.
    fireEvent.keyDown(window, { key: "ß", code: "KeyS", altKey: true });
    expect(s().draft?.signatureOpen).toBe(true);
    fireEvent.keyDown(window, { key: "ß", code: "KeyS", altKey: true });
    expect(s().draft?.signatureOpen).toBe(false);
  });

  it("⌥0–⌥9 show all of an account's addresses or one of them, in the inbox only", () => {
    const icloud = {
      ...personal,
      addresses: [{ email: "hi@rossstudio.art", state: "added" as const, source: "you" as const, seen: 1, blocked: false }],
    };
    resetStore([thread("a", { accountId: "personal", toAddress: "hi@rossstudio.art" })], [work, icloud]);
    // A Mac types "¡" for ⌥1, so the key is read from e.code.
    fireEvent.keyDown(window, { key: "¡", code: "Digit1", altKey: true });
    expect(s().addressFilter).toBeUndefined();
    s().setAccountFilter("personal");
    fireEvent.keyDown(window, { key: "™", code: "Digit2", altKey: true });
    expect(s().addressFilter).toBe("hi@rossstudio.art");
    fireEvent.keyDown(window, { key: "£", code: "Digit3", altKey: true });
    expect(s().addressFilter).toBe("hi@rossstudio.art");
    fireEvent.keyDown(window, { key: "º", code: "Digit0", altKey: true });
    expect(s().addressFilter).toBeUndefined();
  });

  it("⌥F opens and closes the From menu", () => {
    s().compose("new");
    // A Mac types "ƒ" for ⌥F.
    fireEvent.keyDown(window, { key: "ƒ", code: "KeyF", altKey: true });
    expect(s().draft?.fromOpen).toBe(true);
    fireEvent.keyDown(window, { key: "ƒ", code: "KeyF", altKey: true });
    expect(s().draft?.fromOpen).toBe(false);
  });

  it("Ctrl works like ⌘ on Windows and Linux", async () => {
    s().compose("new");
    s().updateDraft({ subject: "Lunch" });
    expect(await sendingDuring(() => userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).keyboard("{Control>}{Enter}{/Control}"))).toContain(true);
  });
});

describe("a draft in the conversation you're reading", () => {
  const mine = () => draft("d1", { mode: "reply", threadId: "a", to: [lena], body: "I'll text you soon." });
  afterEach(() => {
    useDrafts.setState({ drafts: [] });
    useQuickReply.setState({ threadId: undefined, status: undefined });
  });

  it("⌘E opens it and ⌘↵ sends it, over the suggested replies", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    useDrafts.setState({ drafts: [mine()] });
    s().open("a");
    await user.keyboard("{Control>}e{/Control}");
    expect(s()).toMatchObject({ view: "compose", draft: { id: "d1", body: "I'll text you soon." } });
    s().back();
    s().open("a");
    await user.keyboard("{Control>}{Enter}{/Control}");
    expect(useDrafts.getState().drafts).toEqual([]);
    expect(s().toast?.text).toBe("Sending");
  });

  it("once you ask for Quick reply here, ⌘E and ⌘↵ are its again", async () => {
    useDrafts.setState({ drafts: [mine()] });
    s().open("a");
    useQuickReply.setState({ threadId: "a", status: undefined });
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).keyboard("{Control>}e{/Control}");
    expect(s().draft?.id).not.toBe("d1");
    expect(s().draft?.body).toBe("Yes.");
    // Sending Quick reply's words leaves your draft as it was.
    s().back();
    useDrafts.setState({ drafts: [mine()] });
    s().open("a");
    useQuickReply.setState({ threadId: "a", status: undefined });
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).keyboard("{Control>}{Enter}{/Control}");
    // What goes is a reply of its own: your draft stays to finish.
    expect(s().draft).toMatchObject({ body: "Yes." });
    expect(s().draft?.id).not.toBe("d1");
  });
});

describe("more single keys", () => {
  it("i shows everyone on the email (only while reading)", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("i");
    expect(s().recipientsOpen).toBe(false);
    s().open("b");
    await user.keyboard("i");
    expect(s().recipientsOpen).toBe(true);
  });

  it("z shows inbox zero once nothing waits", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    s().markDone("a");
    s().markDone("b");
    s().back();
    await user.keyboard("z");
    expect(s().view).toBe("zero");
  });

  it("o opens only from the list", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("o");
    expect(s().openThreadId).toBe("a");
    await user.keyboard("{ArrowDown}");
    await user.keyboard("o");
    expect(s().openThreadId).toBe("b");
  });

  it("⇧Tab goes back through splits and reply options", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{Shift>}{Tab}{/Shift}");
    expect(s().split).toBe("receipts");
    s().setSplit("important");
    s().open("a");
    await user.keyboard("{Shift>}{Tab}{/Shift}");
    expect(s().replyChoice).toBe(1);
  });

  it("Tab does nothing on an email without reply options", async () => {
    s().open("b");
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).keyboard("{Tab}");
    expect(s().replyChoice).toBe(0);
  });

  it("other ⌥ keys, ⌥R included, do nothing", async () => {
    const before = s();
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{Alt>}e{/Alt}");
    await user.keyboard("{Alt>}r{/Alt}");
    expect(s()).toEqual(before);
  });
});

describe("more ⌘ keys", () => {
  it("⌘W closes the window only at inbox zero", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{Meta>}w{/Meta}");
    expect(closeWindow).not.toHaveBeenCalled();
    useStore.setState({ view: "zero" });
    await user.keyboard("{Meta>}w{/Meta}");
    expect(closeWindow).toHaveBeenCalled();
  });

  it("⌘↵ sends the instant reply and ⌘E edits it", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    s().open("a");
    await user.keyboard("{Meta>}e{/Meta}");
    expect(s().draft).toMatchObject({ body: "Yes.", fieldsOpen: false });
    s().back();
    const transitions = await sendingDuring(async () => {
      await user.keyboard("{Meta>}{Enter}{/Meta}");
      act(() => vi.advanceTimersByTime(sendBeatMs));
    });
    expect(transitions).toEqual([true, false]);
    // It sends from behind the open email, which stays open and not Done.
    expect(s()).toMatchObject({ sending: false, view: "thread", openThreadId: "a", toast: { text: "Sending" } });
    expect(s().threads.map((t) => t.id)).toEqual(["a", "b"]);
  });

  it("other ⌘ keys while reading do nothing", async () => {
    s().open("a");
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).keyboard("{Meta>}g{/Meta}");
    expect(s()).toMatchObject({ view: "thread", openThreadId: "a" });
  });

  it("⌘⇧E folds the fields and ⌘⇧C shows CC in the composer", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    s().compose("new");
    await user.keyboard("{Meta>}{Shift>}e{/Shift}{/Meta}");
    expect(s().draft?.fieldsOpen).toBe(false);
    await user.keyboard("{Meta>}{Shift>}c{/Shift}{/Meta}");
    expect(s().draft).toMatchObject({ showCc: true, fieldsOpen: true });
  });

  it("⌘⇧A attaches files in the composer", async () => {
    const attach = vi.spyOn(s(), "attach").mockImplementation(() => {});
    s().compose("new");
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).keyboard("{Meta>}{Shift>}a{/Shift}{/Meta}");
    expect(attach).toHaveBeenCalledWith();
  });

  it("single keys do nothing in the composer", async () => {
    s().compose("new");
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).keyboard("e");
    expect(s().threads).toHaveLength(2);
  });

  it("Esc from inbox zero goes back to the inbox", async () => {
    useStore.setState({ view: "zero" });
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).keyboard("{Escape}");
    expect(s().view).toBe("inbox");
  });

  it("stops listening when unmounted", async () => {
    resetStore([thread("a"), thread("b")]);
    const { unmount } = renderHook(() => useShortcuts());
    unmount();
    // The hook from beforeEach is still mounted, so one move proves no double handling.
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).keyboard("{ArrowDown}");
    expect(s().cursorId).toBe("b");
  });
});

describe("edge cases", () => {
  it("Tab does nothing at inbox zero", async () => {
    useStore.setState({ view: "zero" });
    await userEvent.setup({ advanceTimers: vi.advanceTimersByTime }).keyboard("{Tab}");
    expect(s()).toMatchObject({ split: "important", replyChoice: 0 });
  });
});

describe("the Blocked list", () => {
  it("⌘⇧B opens it from the inbox or an email; in a new email it still adds Bcc", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{Meta>}{Shift>}b{/Shift}{/Meta}");
    expect(s()).toMatchObject({ view: "blocked", history: ["inbox"] });
    s().back();
    s().open("a");
    await user.keyboard("{Meta>}{Shift>}b{/Shift}{/Meta}");
    expect(s().view).toBe("blocked");
    s().compose("new");
    await user.keyboard("{Meta>}{Shift>}b{/Shift}{/Meta}");
    expect(s()).toMatchObject({ view: "compose", draft: { showBcc: true } });
  });
});

describe("sender keys", () => {
  const jordan = { name: "Jordan Blake", email: "jordan@growthleads.example", initials: "JB" };

  it("on a new sender's email, B blocks, ⇧B blocks the domain, A accepts and ⇧A accepts the domain", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const sam = { ...jordan, email: "sam@growthleads.example" };
    resetStore([thread("n", { from: jordan, sender: "new" }), thread("m", { from: sam, sender: "new" }), thread("k")]);
    s().open("n");
    await user.keyboard("{Shift>}a{/Shift}");
    expect(s().threads.map((t) => t.sender)).toEqual(["known", "known", undefined]);
    resetStore([thread("n", { from: jordan, sender: "new" }), thread("m", { from: sam, sender: "new" }), thread("k")]);
    s().open("n");
    await user.keyboard("a");
    expect(s().threads.map((t) => t.sender)).toEqual(["known", "new", undefined]);
    useStore.setState({ threads: [thread("n", { from: jordan, sender: "new" }), thread("m", { from: { ...jordan, email: "sam@growthleads.example" } }), thread("k")] });
    await user.keyboard("{Shift>}b{/Shift}");
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["n", "m"]);
    useStore.setState({ threads: [thread("n", { from: jordan, sender: "new" }), thread("k")], blockedThreads: [], view: "thread", openThreadId: "n" });
    await user.keyboard("b");
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["n"]);
  });

  it("in the list, B, ⇧B and A act on the new sender under the cursor without opening it", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const mail = () => [thread("n", { from: jordan, sender: "new" }), thread("m", { from: { ...jordan, email: "sam@growthleads.example" } }), thread("k")];
    resetStore(mail());
    await user.keyboard("a");
    expect(s().threads[0].sender).toBe("known");
    useStore.setState({ threads: mail(), cursorId: "n" });
    await user.keyboard("{Shift>}b{/Shift}");
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["n", "m"]);
    useStore.setState({ threads: mail(), blockedThreads: [], cursorId: "n" });
    await user.keyboard("b");
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["n"]);
    expect(s()).toMatchObject({ view: "inbox", cursorId: "m" });
  });

  it("B and A do nothing on someone you know, or away from the list", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("ba");
    s().open("a");
    await user.keyboard("bAu");
    useStore.setState({ threads: [thread("n", { from: jordan, sender: "new" })], cursorId: "n", view: "zero" });
    await user.keyboard("ba");
    expect(s().blockedThreads).toEqual([]);
    expect(s().threads.map((t) => t.sender)).toEqual(["new"]);
  });

  it("U marks the email under the cursor, or the open one, unread", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("u");
    expect(s().threads[0].unread).toBe(true);
    await user.keyboard("u");
    expect(s().threads[0].unread).toBe(false);
    s().open("b");
    await user.keyboard("u");
    expect(s()).toMatchObject({ view: "inbox" });
    expect(s().threads[1].unread).toBe(true);
    // Elsewhere, such as inbox zero, U does nothing.
    useStore.setState({ view: "zero" });
    await user.keyboard("u");
    expect(s().threads.map((t) => t.unread)).toEqual([false, true]);
  });

  it("U unblocks blocked mail you are reading; the Blocked list keeps its own keys", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    useStore.setState({ blockedThreads: [thread("x", { from: jordan, sender: "blocked" })] });
    s().openBlocked();
    await user.keyboard("e");
    expect(s().view).toBe("blocked");
    s().open("x");
    await user.keyboard("u");
    expect(s().threads.map((t) => t.id)).toContain("x");
  });
});

describe("Ask Moss", () => {
  const ask = () => useAskPanel.getState();

  it("⌘J opens the panel, in the reading view too; again closes it", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    await user.keyboard("{Control>}j{/Control}");
    expect(ask()).toMatchObject({ open: true, pick: -1 });
    await user.keyboard("{Control>}j{/Control}");
    expect(ask().open).toBe(false);
    s().open("a");
    await user.keyboard("{Control>}j{/Control}");
    expect(ask()).toMatchObject({ open: true, pick: -1 });
  });

  it("⌘J works on screens with keys of their own, but not in the composer, settings or a new account", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    s().openDone();
    await user.keyboard("{Control>}j{/Control}");
    expect(ask().open).toBe(true);
    for (const view of ["compose", "settings", "add-account"] as const) {
      resetAskPanel();
      useStore.setState({ view });
      await user.keyboard("{Control>}j{/Control}");
      expect(ask().open, view).toBe(false);
    }
  });

  it("Esc closes the panel before it goes back", async () => {
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    s().open("a");
    ask().show();
    await user.keyboard("{Escape}");
    expect(ask().open).toBe(false);
    expect(s().view).toBe("thread");
    await user.keyboard("{Escape}");
    expect(s().view).toBe("inbox");
  });
});

describe.each(digitFroms)("number shortcuts from the %s keys", (from) => {
  it("⌘1–⌘9 pick an account and ⌘0 shows them all (Ctrl too)", () => {
    expect(pressDigit(2, from, { meta: true })).toBe(false);
    expect(s().accountFilter).toBe("personal");
    pressDigit(9, from, { meta: true });
    expect(s().accountFilter).toBe("personal");
    pressDigit(0, from, { meta: true });
    expect(s().accountFilter).toBe("all");
    pressDigit(1, from, { ctrl: true });
    expect(s().accountFilter).toBe("work");
  });

  it("⌥0–⌥9 pick one of an account's addresses or all of them", () => {
    const icloud = {
      ...personal,
      addresses: [{ email: "hi@rossstudio.art", state: "added" as const, source: "you" as const, seen: 1, blocked: false }],
    };
    resetStore([thread("a", { accountId: "personal", toAddress: "hi@rossstudio.art" })], [work, icloud]);
    s().setAccountFilter("personal");
    expect(pressDigit(2, from, { alt: true })).toBe(false);
    expect(s().addressFilter).toBe("hi@rossstudio.art");
    pressDigit(0, from, { alt: true });
    expect(s().addressFilter).toBeUndefined();
  });

  it("digits typed in a text field stay text, even with ⌥", () => {
    const icloud = {
      ...personal,
      addresses: [{ email: "hi@rossstudio.art", state: "added" as const, source: "you" as const, seen: 1, blocked: false }],
    };
    resetStore([thread("a", { accountId: "personal", toAddress: "hi@rossstudio.art" })], [work, icloud]);
    s().setAccountFilter("personal");
    const input = document.body.appendChild(document.createElement("input"));
    expect(pressDigit(2, from, {}, input)).toBe(true);
    expect(pressDigit(2, from, { alt: true }, input)).toBe(true);
    expect(s()).toMatchObject({ accountFilter: "personal", addressFilter: undefined });
    input.remove();
  });
});

it("does not open the palette from outside the quit modal", async () => {
  const { useQuit } = await import("./quitState");renderHook(()=>useShortcuts());
  useQuit.setState({token:"quit"});fireEvent.keyDown(window,{key:"k",ctrlKey:true});
  expect(useStore.getState().paletteOpen).toBe(false);
});
