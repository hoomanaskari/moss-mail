import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoAsk, setAsk, type AskBackend } from "../services/ask";
import { demoMail, setMail } from "../services/mail";
import { message, person, resetStore, thread, work } from "../test/fixtures";
import { emailThread, openEmail, resetAskPanel, turnsOf, useAskPanel, watchAsk, editDraft, sendDraft } from "./askPanel";
import { draftsSaved, useDrafts } from "./drafts";
import { useScheduled } from "./scheduled";
import { useSignatures } from "./signatures";
import { useStore } from "./store";

const dana = person("Dana Whitfield", "dana@acme.co", { short: "Dana" });
const budget = thread("41", { from: dana, subject: "Q4 budget", messages: [message({ id: "41-1", from: dana }), message({ id: "41-2" })] });
const lunch = thread("57", { subject: "Lunch" });

const panel = () => useAskPanel.getState();
const settle = () => new Promise((r) => setTimeout(r, 0));

let stopWatching: () => void;
beforeEach(() => {
  resetStore([budget, lunch]);
  resetAskPanel();
  setAsk(demoAsk({ threads: [budget, lunch] }));
  stopWatching = watchAsk();
});
afterEach(() => stopWatching());

describe("opening", () => {
  it("opens with nothing picked, an email open or not", async () => {
    useStore.setState({ view: "thread", openThreadId: "41" });
    panel().show();
    expect(panel()).toMatchObject({ open: true, pick: -1 });
    expect(emailThread()?.id).toBe("41");
    panel().hide();
    useStore.setState({ view: "inbox", cursorId: "57" });
    panel().show();
    expect(panel()).toMatchObject({ open: true, pick: -1 });
    await settle();
    expect(panel().mailCount).toBe(2431);
  });

  it("opened again on a chat already going, nothing is picked", async () => {
    useStore.setState({ view: "thread", openThreadId: "41" });
    panel().show();
    await panel().ask("When is it due?");
    panel().hide();
    panel().show();
    expect(panel().pick).toBe(-1);
  });

  it("⌘J again closes it", () => {
    panel().toggle();
    expect(panel().open).toBe(true);
    panel().toggle();
    expect(panel().open).toBe(false);
  });

  it("only an open email is the context, not the one under the cursor", () => {
    useStore.setState({ view: "inbox", cursorId: "57" });
    expect(emailThread()).toBeUndefined();
  });

  it("a mail count that can't be read stays unknown", async () => {
    setAsk({ ...demoAsk(), mailCount: () => Promise.reject(new Error("no")) });
    panel().show();
    await settle();
    expect(panel().mailCount).toBeUndefined();
  });
});

describe("asking", () => {
  it("asks about the open email, streaming into the turn, and cites its messages", async () => {
    useStore.setState({ view: "thread", openThreadId: "41" });
    panel().show();
    panel().type("When is it due?");
    expect(panel().pick, "typing goes back to the box").toBe(-1);
    const asked = panel().ask();
    expect(panel().input).toBe("");
    expect(turnsOf(panel())).toMatchObject([{ question: "When is it due?", state: "thinking" }]);
    await asked;
    expect(turnsOf(panel())).toMatchObject([{ state: "done", messages: ["41-1", "41-2"] }]);
    expect(turnsOf(panel())[0].text).toMatch(/Friday/);
  });

  it("with no email open it asks all mail; the first email card is picked when the answer lands", async () => {
    panel().show();
    await panel().ask("Did Dana send the contract?");
    const [turn] = turnsOf(panel());
    expect(turn.sources.map((t) => t.id)).toEqual(["41", "57"]);
    expect(panel().pick).toBe(0);
    panel().move(1, 2);
    expect(panel().pick).toBe(1);
    panel().move(1, 2);
    expect(panel().pick, "stays on the last").toBe(1);
    panel().move(-1, 2);
    panel().move(-1, 2);
    expect(panel().pick).toBe(0);
    panel().move(1, 0);
    expect(panel().pick, "nothing to pick").toBe(-1);
  });

  it("is one chat that follows you from email to email, telling Moss which one is open", async () => {
    const demo = demoAsk({ threads: [budget, lunch] });
    const asked = vi.fn(demo.askMoss);
    setAsk({ ...demo, askMoss: asked });
    useStore.setState({ view: "thread", openThreadId: "41" });
    panel().show();
    await panel().ask("When is it due?");
    useStore.setState({ view: "thread", openThreadId: "57" });
    await panel().ask("And this one?");
    useStore.setState({ view: "inbox" });
    await panel().ask("Anything else from Dana?");
    expect(asked.mock.calls).toEqual([
      ["moss-1", "When is it due?", "41"],
      ["moss-1", "And this one?", "57"],
      ["moss-1", "Anything else from Dana?", undefined],
    ]);
    expect(turnsOf(panel()).map((t) => t.question)).toEqual(["When is it due?", "And this one?", "Anything else from Dana?"]);
  });

  it("does what you ask with mail, and says what it did", async () => {
    panel().show();
    await panel().ask("Mark the budget emails done");
    expect(turnsOf(panel())[0].did).toEqual(["Marked done: Q4 budget", "Marked done: Lunch"]);
  });

  it("nothing to ask, or an answer still coming, asks nothing", async () => {
    const asked = vi.fn(demoAsk().askMoss);
    setAsk({ ...demoAsk({ answer: "thinking" }), askMoss: asked });
    panel().show();
    await panel().ask("   ");
    expect(asked).not.toHaveBeenCalled();
    void panel().ask("one");
    await panel().ask("two");
    expect(asked).toHaveBeenCalledTimes(1);
  });

  it("a failure, or a missing model, lands in the turn", async () => {
    setAsk(demoAsk({ answer: "missing" }));
    panel().show();
    await panel().ask("q");
    expect(turnsOf(panel())[0]).toMatchObject({ state: "failed", error: { kind: "notReady" } });
    setAsk({ ...demoAsk(), askMoss: () => Promise.reject(new Error("socket closed")) });
    await panel().ask("again");
    expect(turnsOf(panel())[1]).toMatchObject({ state: "failed", error: { kind: "model", message: "socket closed" } });
  });

  it("Esc stops the answer and keeps what it wrote", async () => {
    setAsk(demoAsk({ answer: "thinking", threads: [budget] }));
    stopWatching();
    stopWatching = watchAsk();
    panel().show();
    const asked = panel().ask("q");
    await settle();
    expect(turnsOf(panel())[0].activity).toMatch(/Searching mail/);
    panel().stop();
    await asked;
    expect(turnsOf(panel())[0].state).toBe("stopped");
  });

  it("events for another chat, or a finished turn, change nothing", async () => {
    panel().show();
    await panel().ask("q");
    const before = turnsOf(panel());
    let emit: (e: Parameters<Parameters<AskBackend["onEvent"]>[0]>[0]) => void = () => {};
    stopWatching();
    setAsk({ ...demoAsk(), onEvent: (h) => ((emit = h), () => {}) });
    stopWatching = watchAsk();
    emit({ type: "text", chat: "moss-1", delta: "late" });
    emit({ type: "text", chat: "other", delta: "x" });
    emit({ type: "done", chat: "moss-1", answer: { text: "", sources: [], messages: [], drafts: [], stopped: false } });
    expect(turnsOf(panel())).toEqual(before);
    expect(panel().chats.other).toBeUndefined();
  });

  it("New chat starts over, and forgets the old one", async () => {
    const forget = vi.fn(async () => {});
    setAsk({ ...demoAsk(), forget });
    panel().show();
    await panel().ask("q");
    panel().newChat();
    expect(panel()).toMatchObject({ chat: "moss-2", pick: -1 });
    expect(turnsOf(panel())).toEqual([]);
    expect(Object.keys(panel().chats)).toEqual([]);
    expect(forget).toHaveBeenCalledWith("moss-1");
    // With an email open too, a new chat starts with nothing picked.
    useStore.setState({ view: "thread", openThreadId: "41" });
    await panel().ask("q");
    panel().newChat();
    expect(panel()).toMatchObject({ chat: "moss-3", pick: -1 });
    expect(forget).toHaveBeenLastCalledWith("moss-2");
  });

  it("⌘R drafts a reply from the chat", async () => {
    useStore.setState({ view: "thread", openThreadId: "41" });
    panel().show();
    await panel().draftReply();
    const [turn] = turnsOf(panel());
    expect(turn.question).toBe("Draft a reply based on this chat.");
    expect(turn.drafts).toMatchObject([{ threadId: "41" }]);
  });
});

describe("drafts", () => {
  const studio = {
    ...work,
    id: "studio",
    address: "maya@studio.co",
    addresses: [{ email: "hello@studio.co", state: "added" as const, source: "you" as const, seen: 3, blocked: false }],
  };
  const priya = thread("63", { accountId: "studio", toAddress: "hello@studio.co", from: person("Priya Nair", "priya@nair.co") });

  it("a reply goes from the address the email came to, with that address's signature, and is in Drafts", async () => {
    resetStore([priya], [studio]);
    setAsk(demoAsk({ threads: [priya] }));
    const saved = vi.spyOn(demoMail, "saveDraft");
    panel().show();
    await panel().ask("Reply to Priya saying yes");
    const [draft] = turnsOf(panel())[0].drafts;
    expect(draft).toMatchObject({ threadId: "63", fromAccountId: "studio", fromAddress: "hello@studio.co" });
    await draftsSaved();
    expect(useDrafts.getState().drafts[0]).toMatchObject({ ...draft, updatedAt: expect.any(Number) });
    expect(saved).toHaveBeenCalledWith(expect.objectContaining({ ...draft, updatedAt: expect.any(Number) }));
    saved.mockRestore();
  });

  it("keeps the acknowledged revision when preparing an Ask reply", async () => {
    resetStore([priya], [studio]);
    setAsk(demoAsk({ threads: [priya] }));
    const save = vi.spyOn(demoMail, "saveDraft").mockImplementation(async (draft) => ({ ...draft, revision: "ask-ack" }));
    try {
      panel().show();
      await panel().ask("Reply to Priya saying yes");
      await draftsSaved();
      expect(useDrafts.getState().drafts[0]).toMatchObject({ revision: "ask-ack", fromAddress: "hello@studio.co" });
    } finally { save.mockRestore(); }
  });

  it("a drafted reply in a back-and-forth gets the signature the composer would pick", async () => {
    const back = thread("64", { accountId: "studio", messages: [message(), message({ id: "64-2", from: person("Maya", "maya@studio.co") })] });
    resetStore([back], [studio]);
    useSignatures.setState({ byAccount: { studio: { newEmails: "studio", replies: "studio" } }, byAddress: {} });
    setAsk(demoAsk({ threads: [back] }));
    panel().show();
    await panel().ask("Reply saying yes");
    expect(turnsOf(panel())[0].drafts[0]).toMatchObject({ threadId: "64", signatureId: null });
    // Without the back-and-forth, the one for replies.
    useSignatures.setState({ letMossChoose: false });
    await panel().ask("Reply saying yes");
    expect(turnsOf(panel())[1].drafts[0]).toMatchObject({ threadId: "64", signatureId: "studio" });
  });

  it("a reply to mail that's gone stays as it came", async () => {
    const gone = thread("99", { subject: "Gone" });
    setAsk(demoAsk({ threads: [gone] }));
    panel().show();
    await panel().ask("draft a reply");
    expect(turnsOf(panel())[0].drafts[0]).not.toHaveProperty("fromAddress");
  });

  it("Not now hides a draft's card", () => {
    panel().dismiss("d1");
    expect(panel().dismissed).toEqual(["d1"]);
  });

  it("Edit opens the draft in the composer and closes the panel", async () => {
    panel().show();
    await panel().ask("draft a reply");
    const [draft] = turnsOf(panel())[0].drafts;
    editDraft(draft);
    expect(panel().open).toBe(false);
    expect(useStore.getState()).toMatchObject({ view: "compose", draft: { id: draft.id } });
  });

  it("Send sends it with Undo, where you are, and hides the card", async () => {
    const sent = vi.spyOn(demoMail, "sendAfter");
    panel().show();
    await panel().ask("draft a reply");
    const [draft] = turnsOf(panel())[0].drafts;
    sendDraft(draft);
    expect(sent).toHaveBeenCalledWith(expect.any(String), draft, expect.any(Number));
    expect(useStore.getState()).toMatchObject({ view: "inbox", toast: { text: "Sending" } });
    expect(panel().dismissed).toContain(draft.id);
    sent.mockRestore();
  });

  it("Send on one you asked to go later sets it to go then, and hides the card", async () => {
    panel().show();
    await panel().ask("draft a reply");
    const [draft] = turnsOf(panel())[0].drafts;
    const at = Date.now() + 86_400_000;
    sendDraft(draft, at);
    expect(useScheduled.getState().emails).toMatchObject([{ draft: { id: draft.id }, sendAt: at }]);
    expect(useStore.getState().toast?.text).toMatch(/^Scheduled for /);
    expect(panel().dismissed).toContain(draft.id);
  });
});

describe("a source chip", () => {
  it("shows the message it names in the open email and scrolls to it", async () => {
    useStore.setState({ view: "thread", openThreadId: "41" });
    panel().show();
    const target = document.createElement("div");
    target.dataset.message = "41-1";
    target.scrollIntoView = vi.fn();
    document.body.append(target);
    panel().showMessage("41-1");
    expect(panel().reveal).toBe("41-1");
    await settle();
    expect(target.scrollIntoView).toHaveBeenCalledWith({ block: "start", behavior: "smooth" });
    // A message not drawn is no problem.
    panel().showMessage("41-2");
    expect(useStore.getState().openThreadId).toBe("41");
    await settle();
    target.remove();
  });
});

describe("opening an email from a card", () => {
  it("opens one in the mailbox, or fetches it first", async () => {
    openEmail("57");
    expect(useStore.getState()).toMatchObject({ view: "thread", openThreadId: "57" });
    const old = thread("12", { subject: "Old" });
    setMail({ ...demoMail, thread: async (id) => (id === "12" ? old : undefined) });
    await openEmail("12");
    expect(useStore.getState()).toMatchObject({ view: "thread", openThreadId: "12" });
    await openEmail("404");
    expect(useStore.getState().openThreadId, "nothing to open").toBe("12");
    setMail(demoMail);
  });
});
