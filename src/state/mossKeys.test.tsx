import { renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { demoReplies, setReplies } from "../services/replies";
import { resetStore, thread } from "../test/fixtures";
import { resetAskPanel, useAskPanel } from "./askPanel";
import { useMossWriting } from "./mossWriting";
import { useQuickReply } from "./quickReply";
import { useStore } from "./store";
import { useShortcuts } from "./useShortcuts";

const s = () => useStore.getState();
const m = () => useMossWriting.getState();
const q = () => useQuickReply.getState();

/** Presses a key on `target` (the window by default); true when Moss's keys took it. */
function press(key: string, init: KeyboardEventInit = {}, target: EventTarget = window) {
  const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
  target.dispatchEvent(e);
  return e.defaultPrevented;
}

beforeEach(() => {
  resetStore([thread("a", { replyOptions: [{ label: "Yes", draft: "Yes." }] }), thread("b")]);
  setReplies(demoReplies());
  m().reset();
  q().close();
  resetAskPanel();
  renderHook(() => useShortcuts());
});

describe("Quick reply keys", () => {
  it("Q starts Quick reply on the open email, not elsewhere or while typing", async () => {
    expect(press("q")).toBe(false);
    s().open("b");
    const input = document.body.appendChild(document.createElement("input"));
    expect(press("q", {}, input)).toBe(false);
    input.remove();
    expect(press("q", { metaKey: true })).toBe(false);
    expect(press("q")).toBe(true);
    expect(q()).toMatchObject({ threadId: "b", status: "writing" });
    await vi.waitFor(() => expect(q().status).toBeUndefined());
  });

  it("Esc closes Quick reply while it's writing or stuck, and goes back otherwise", () => {
    s().open("b");
    useQuickReply.setState({ threadId: "b", status: "failed" });
    press("Escape");
    expect(q().threadId).toBeUndefined();
    expect(s().view).toBe("thread");
    press("Escape");
    expect(s().view).toBe("inbox");
  });

  it("Esc discards a Quick reply you opened, and Esc again goes back", () => {
    s().open("a");
    useQuickReply.setState({ threadId: "a" });
    expect(press("Escape")).toBe(true);
    expect(s().threads.find((t) => t.id === "a")?.replyOptions).toBeUndefined();
    expect(s().view).toBe("thread");
    press("Escape");
    expect(s().view).toBe("inbox");
  });

  it("while Moss is still writing the replies, ⌘E, ⌘↵ and ⌥S wait, and ⌘J is Ask Moss's", () => {
    s().open("a");
    useQuickReply.setState({ threadId: "a", status: "writing", asking: "quick-1" });
    press("j", { metaKey: true });
    expect(useAskPanel.getState().open).toBe(true);
    useAskPanel.getState().toggle();
    for (const [key, init] of [
      ["e", { metaKey: true }],
      ["Enter", { metaKey: true }],
      ["ß", { altKey: true, code: "KeyS" }],
    ] as const) {
      expect(press(key, init)).toBe(true);
    }
    expect(s().view).toBe("thread");
    expect(s().sending).toBeFalsy();
    expect(s().threads[0].replyOptions).toHaveLength(1);
    expect(q().signatureOpen).toBe(false);
    expect(useAskPanel.getState().open).toBe(false);
    useQuickReply.setState({ threadId: undefined, status: undefined, asking: undefined });
  });

  it("⌥S opens the signature picker on a Quick reply that's showing, not without one", () => {
    s().open("b");
    expect(press("ß", { altKey: true, code: "KeyS" })).toBe(false);
    s().open("a");
    expect(press("ß", { altKey: true, code: "KeyS" })).toBe(true);
    expect(q().signatureOpen).toBe(true);
    q().toggleSignature();
  });

  it("⌘J on a Quick reply you opened opens it in the composer with Write with Moss open", () => {
    s().open("a");
    useQuickReply.setState({ threadId: "a" });
    expect(press("j", { metaKey: true })).toBe(true);
    expect(s()).toMatchObject({ view: "compose", draft: { body: "Yes." } });
    expect(m().menuOpen).toBe(true);
  });

  it("⌘J without a Quick reply you opened is left for Ask Moss", () => {
    const ask = () => useAskPanel.getState().open;
    s().open("b");
    press("j", { ctrlKey: true });
    expect(ask()).toBe(true);
    // Still writing: nothing to change yet.
    useQuickReply.setState({ threadId: "b", status: "writing" });
    press("j", { metaKey: true });
    expect(ask()).toBe(false);
    q().close();
    // Replies already on the email, but you didn't press Q: still Ask's.
    s().open("a");
    press("j", { metaKey: true });
    expect(ask()).toBe(true);
    expect(s().view).toBe("thread");
    expect(m().menuOpen).toBe(false);
  });
});

describe("composer keys", () => {
  beforeEach(() => {
    s().open("b");
    s().compose("reply");
    s().updateDraft({ body: "Hi Dana,\n\nYes." });
  });

  it("⌘J opens Write with Moss, and Esc closes it without leaving", () => {
    expect(press("j", { metaKey: true })).toBe(true);
    expect(m().menuOpen).toBe(true);
    press("Escape");
    expect(m().menuOpen).toBe(false);
    expect(s().view).toBe("compose");
  });

  it("Esc stops Moss writing, without leaving", async () => {
    setReplies(demoReplies({ state: "writing" }));
    const writing = m().write({ job: { kind: "accept" }, lead: "Moss wrote a yes" });
    await Promise.resolve();
    expect(press("Escape")).toBe(true);
    await writing;
    expect(m().status).toBe("wrote");
    expect(s().view).toBe("compose");
  });

  it("⌘Z undoes what Moss wrote, even while typing in the text", async () => {
    await m().write({ job: { kind: "shorter" }, lead: "Moss made it shorter" });
    const editor = document.body.appendChild(document.createElement("textarea"));
    expect(press("z", { metaKey: true, shiftKey: true }, editor)).toBe(false);
    expect(press("z", { metaKey: true }, editor)).toBe(true);
    expect(s().draft?.body).toBe("Hi Dana,\n\nYes.");
    // Nothing of Moss's left: ⌘Z is the editor's again.
    expect(press("z", { metaKey: true }, editor)).toBe(false);
    // Nor in another draft.
    await m().write({ job: { kind: "shorter" }, lead: "Moss made it shorter" });
    s().compose("new");
    expect(press("z", { metaKey: true }, editor)).toBe(false);
    expect(press("Escape", {}, editor)).toBe(false);
    editor.remove();
  });
});
