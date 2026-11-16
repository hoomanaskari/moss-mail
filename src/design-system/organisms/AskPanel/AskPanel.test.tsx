import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { startTurn, type ChatTurn } from "../../../services/ask";
import type { SavedDraft } from "../../../state/drafts";
import { message, person, thread } from "../../../test/fixtures";
import { AskPanel, type AskPanelActions, type AskPanelProps } from "./AskPanel";

const dana = person("Dana Whitfield", "dana@acme.co", { short: "Dana" });
const contract = thread("41", {
  from: dana,
  subject: "Spring campaign",
  time: "9:12 AM",
  summary: "Can you countersign it?",
  messages: [
    message({ id: "41-0", attachments: [{ name: "draft-v1.pdf", size: "280 KB" }] }),
    message({ id: "41-1", attachments: [{ name: "contract.pdf", size: "312 KB" }, { name: "terms.pdf", size: "20 KB" }] }),
    message({ id: "41-2" }),
  ],
});
const older = thread("24", { from: dana, subject: "First draft", time: "Oct 24" });

const draft: SavedDraft = {
  id: "d1",
  mode: "reply",
  threadId: "41",
  fromAccountId: "work",
  to: [dana],
  cc: [],
  bcc: [],
  subject: "Re: Spring campaign",
  body: "Hi Dana, yes.\n\nBest,",
  bodyHtml: "",
  fieldsOpen: true,
  showCc: false,
  showBcc: false,
  remindIfNoReply: false,
  signatureId: null,
  updatedAt: 1,
};

const done = (question: string, extra: Partial<ChatTurn> = {}): ChatTurn => ({ ...startTurn(question), state: "done", text: "Yes.", ...extra });

function setup(props: Partial<AskPanelProps> = {}) {
  const actions = Object.fromEntries(
    ["type", "ask", "close", "stop", "newChat", "draftReply", "move", "openEmail", "showMessage", "openFile", "editDraft", "sendDraft", "dismissDraft", "download", "settings", "search"].map((k) => [k, vi.fn()]),
  ) as unknown as { [K in keyof AskPanelActions]: ReturnType<typeof vi.fn> } & AskPanelActions;
  const all: AskPanelProps = {
    emailOpen: true,
    turns: [],
    input: "",
    pick: 0,
    intro: "Ask anything about this conversation.",
    thinking: "Reading 6 emails…",
    noModel: "Ask needs the writing model.",
    dismissed: [],
    accountOf: () => ({ letter: "W", color: "work", address: "maya@example.test" }),
    chip: (id) => (id === "41-1" ? "Dana · Oct 28" : undefined),
    draftLine: () => "Reply to Dana Whitfield",
    replyingTo: (d) => (d.threadId === "41" ? contract : undefined),
    sendsAt: () => undefined,
    actions,
    ...props,
  };
  const view = render(<AskPanel {...all} />);
  const box = within(view.container).getByRole("textbox");
  const key = (k: string, init: object = {}) => fireEvent.keyDown(box, { key: k, ...init });
  return { actions, box, key, view, user: userEvent.setup() };
}

describe("AskPanel", () => {
  it("is a labelled panel with the box focused, and how to close it: no tabs, no scope to pick", async () => {
    const { actions, box, key, user } = setup();
    expect(screen.getByRole("complementary", { name: "Ask Moss" })).toBeInTheDocument();
    expect(box).toHaveFocus();
    expect(box).toHaveAttribute("placeholder", "Ask about this email, or any other…");
    expect(screen.queryByRole("radiogroup")).not.toBeInTheDocument();
    expect(screen.queryByRole("radio")).not.toBeInTheDocument();
    expect(screen.getByText(/Runs on this .*Answers come only from your mail\./)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "EscClose" }));
    expect(actions.close).toHaveBeenCalled();
    const outer = vi.fn();
    window.addEventListener("keydown", outer);
    key("Tab");
    window.removeEventListener("keydown", outer);
    expect(outer, "Tab is the page's again").toHaveBeenCalled();
  });

  it("with an email open, says what it can do, with no ready-made questions to pick", () => {
    const { actions, key } = setup({ pick: -1 });
    expect(screen.getByText("Ask anything about this conversation.")).toBeInTheDocument();
    expect(screen.queryByText("Try")).not.toBeInTheDocument();
    expect(screen.queryByText(/to pick/)).not.toBeInTheDocument();
    key("ArrowDown");
    expect(actions.move).toHaveBeenCalledWith(1, 0);
    key("ArrowUp");
    expect(actions.move).toHaveBeenLastCalledWith(-1, 0);
    key("Enter");
    expect(actions.ask, "an empty box asks nothing").not.toHaveBeenCalled();
  });

  it("typing goes to the box; ↵ asks what you typed", async () => {
    const { actions, box, key, user } = setup({ input: "When is it due?", pick: -1 });
    await user.type(box, "!");
    expect(actions.type).toHaveBeenCalledWith("When is it due?!");
    key("Enter");
    expect(actions.ask).toHaveBeenCalledWith();
    key("Enter", { shiftKey: true });
    expect(actions.ask).toHaveBeenCalledTimes(1);
  });

  it("with no email open, starts with what it can do, and no suggestions", () => {
    const { actions, key } = setup({ emailOpen: false, pick: -1 });
    expect(screen.getByText(/Ask about any email, or tell Moss what to do with your mail/)).toBeInTheDocument();
    expect(screen.queryByText("Try")).not.toBeInTheDocument();
    expect(screen.getByRole("textbox")).toHaveAttribute("placeholder", "Ask about any email…");
    key("Enter");
    expect(actions.ask).not.toHaveBeenCalled();
  });

  it("while thinking it says what it's doing, and Esc stops it", async () => {
    const thinking = startTurn("Find the PDF");
    const { actions, key, view, user } = setup({ turns: [thinking] });
    expect(screen.getByText("Find the PDF")).toBeInTheDocument();
    expect(screen.getByText("Reading 6 emails…")).toBeInTheDocument();
    key("Escape");
    expect(actions.stop).toHaveBeenCalled();
    expect(actions.close).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "EscStop" }));
    expect(actions.stop).toHaveBeenCalledTimes(2);
    view.unmount();
    setup({ turns: [{ ...thinking, activity: "Searching mail for “PDF”" }] });
    expect(screen.getByText("Searching mail for “PDF”")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Draft a reply from this chat/ })).not.toBeInTheDocument();
  });

  it("an answer about this email shows chips for the messages it used", async () => {
    const { actions, box, user } = setup({ turns: [done("Agreed?", { messages: ["41-1", "gone"] })] });
    expect(box).toHaveAttribute("placeholder", "Ask a follow-up…");
    await user.click(screen.getByRole("button", { name: "Dana · Oct 28" }));
    expect(actions.showMessage).toHaveBeenCalledWith("41-1");
    expect(screen.getAllByRole("button", { name: /·/ })).toHaveLength(1);
  });

  it("an answer shows a chip for each file it took facts from, which opens the file", async () => {
    const file = { threadId: "41", messageId: "41-1", index: 2, name: "P2026-0457.pdf" };
    const all = setup({ emailOpen: false, turns: [done("When does my permit start?", { files: [file] })] });
    const chip = screen.getByRole("button", { name: "Open P2026-0457.pdf" });
    expect(chip).toHaveAttribute("title", "P2026-0457.pdf");
    await all.user.click(chip);
    expect(all.actions.openFile).toHaveBeenCalledWith(file);
    all.view.unmount();
    // With the email open, next to the message chips.
    setup({ turns: [done("When?", { messages: ["41-1"], files: [file] })] });
    expect(screen.getByRole("button", { name: "Open P2026-0457.pdf" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Dana · Oct 28" })).toBeInTheDocument();
  });

  it("an answer shows the emails it used as cards: ↵ opens the picked one", async () => {
    const { actions, key, user } = setup({ emailOpen: false, pick: 0, turns: [done("Did Dana send it?", { sources: [contract, older] })] });
    const first = screen.getByRole("button", { name: /Dana Whitfield9:12 AM.*Spring campaign/ });
    expect(first).toHaveAttribute("aria-current", "true");
    expect(within(first).getByText("contract.pdf")).toBeInTheDocument();
    expect(within(first).queryByText("terms.pdf"), "the first file only").not.toBeInTheDocument();
    expect(within(first).queryByText("draft-v1.pdf"), "of the newest email with files").not.toBeInTheDocument();
    expect(screen.getByText(/↑ ↓ to pick an email/)).toBeInTheDocument();
    key("ArrowDown");
    expect(actions.move).toHaveBeenCalledWith(1, 2);
    key("Enter");
    expect(actions.openEmail).toHaveBeenCalledWith("41");
    await user.click(screen.getByRole("button", { name: /First draft/ }));
    expect(actions.openEmail).toHaveBeenLastCalledWith("24");
    expect(screen.queryByText("Dana · Oct 28")).not.toBeInTheDocument();
  });

  it("a reply to the open email: Edit ⌘E, Send ⌘↵, no Not now", async () => {
    const { actions, key, user } = setup({ replyingTo: () => undefined, turns: [done("Draft a reply", { drafts: [draft] })] });
    const card = screen.getByRole("group", { name: "Draft reply" });
    expect(within(card).getByText("Reply to Dana Whitfield")).toBeInTheDocument();
    // Whatever the answer says, the card says plainly that nothing has gone.
    expect(within(card).getByText("Not sent yet")).toBeInTheDocument();
    expect(within(card).getByText(/Hi Dana, yes\./)).toBeInTheDocument();
    expect(within(card).queryByRole("button", { name: "Not now" })).not.toBeInTheDocument();
    key("e", { metaKey: true });
    expect(actions.editDraft).toHaveBeenCalledWith(draft);
    key("Enter", { metaKey: true });
    expect(actions.sendDraft).toHaveBeenCalledWith(draft);
    key("o", { metaKey: true });
    expect(actions.openEmail).not.toHaveBeenCalled();
    await user.click(within(card).getByRole("button", { name: "EditCtrl+E" }));
    await user.click(within(card).getByRole("button", { name: "SendCtrl+↵" }));
    expect(actions.editDraft).toHaveBeenCalledTimes(2);
    expect(actions.sendDraft).toHaveBeenCalledTimes(2);
  });

  it("a reply to another email shows what it replies to: ⌘O opens it, Not now hides the card", async () => {
    const { actions, key, user } = setup({ turns: [done("Tell Dana yes", { sources: [contract], drafts: [draft] })] });
    const card = screen.getByRole("group", { name: "Draft reply" });
    expect(within(card).getByText(/Spring campaign · Can you countersign it\?/)).toBeInTheDocument();
    expect(screen.getByText("Sending doesn’t mark Dana’s email Done. You decide that.")).toBeInTheDocument();
    expect(screen.queryByText(/↑ ↓ to pick an email/)).not.toBeInTheDocument();
    key("o", { metaKey: true });
    expect(actions.openEmail).toHaveBeenCalledWith("41");
    await user.click(within(card).getByRole("button", { name: "Ctrl+OOpen" }));
    await user.click(within(card).getByRole("button", { name: "Not now" }));
    expect(actions.dismissDraft).toHaveBeenCalledWith(draft);
  });

  it("names a sender with no short name by their first name", () => {
    const noShort = { ...contract, from: person("Priya Nair", "priya@nair.co") };
    setup({ replyingTo: () => noShort, turns: [done("q", { drafts: [draft] })] });
    expect(screen.getByText("Sending doesn’t mark Priya’s email Done. You decide that.")).toBeInTheDocument();
  });

  it("a dismissed draft's card is gone; a reply to mail it can't show has no box", () => {
    const first = setup({ dismissed: ["d1"], turns: [done("q", { drafts: [draft] })] });
    expect(screen.queryByRole("group", { name: "Draft reply" })).not.toBeInTheDocument();
    first.view.unmount();
    setup({ turns: [done("q", { drafts: [{ ...draft, id: "d2", threadId: "99" }] })] });
    expect(screen.getByRole("group", { name: "Draft reply" })).toBeInTheDocument();
    expect(screen.queryByText(/You decide that/)).not.toBeInTheDocument();
  });

  it("after an answer: ⌘R drafts a reply from the chat, and New chat starts over", async () => {
    const { actions, key, user } = setup({ turns: [done("Agreed?")] });
    key("r", { metaKey: true });
    expect(actions.draftReply).toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Draft a reply from this chatCtrl+R" }));
    await user.click(screen.getByRole("button", { name: "New chat" }));
    expect(actions.draftReply).toHaveBeenCalledTimes(2);
    expect(actions.newChat).toHaveBeenCalled();
    key("Escape");
    expect(actions.close).toHaveBeenCalled();
  });

  it("⌘R before anything is asked does nothing; other keys reach the screen", () => {
    const { actions, key } = setup();
    key("r", { metaKey: true });
    expect(actions.draftReply).not.toHaveBeenCalled();
    const outer = vi.fn();
    window.addEventListener("keydown", outer);
    key("ArrowDown");
    key("Escape");
    expect(outer, "keys the panel handles stay in it").not.toHaveBeenCalled();
    key("x");
    window.removeEventListener("keydown", outer);
    expect(outer).toHaveBeenCalledTimes(1);
  });

  it("nothing found: says so, and offers search instead (/)", async () => {
    const { actions, key, user } = setup({ emailOpen: false, pick: -1, turns: [done("Find the bank PDF", { text: "I couldn’t find one." })] });
    expect(screen.getByText("I couldn’t find one.")).toBeInTheDocument();
    key("/");
    expect(actions.search).toHaveBeenCalledWith("Find the bank PDF");
    await user.click(screen.getByRole("button", { name: "Search instead/" }));
    expect(actions.search).toHaveBeenCalledTimes(2);
  });

  it("the model not downloaded: Download ↵ and AI settings", async () => {
    const failed: ChatTurn = { ...startTurn("q"), state: "failed", error: { kind: "notReady", message: "x" } };
    const first = setup({ pick: -1, turns: [failed, failed] });
    const [older, latest] = screen.getAllByRole("button", { name: /^Download/ });
    expect([older.textContent, latest.textContent], "only the latest takes ↵").toEqual(["Download", "Download↵"]);
    first.view.unmount();
    const { actions, key, user } = setup({ pick: -1, turns: [failed] });
    expect(screen.getByRole("alert")).toHaveTextContent("Ask needs the writing model.");
    key("Enter");
    expect(actions.download).toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "AI settingsCtrl+," }));
    expect(actions.settings).toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Download↵" }));
    expect(actions.download).toHaveBeenCalledTimes(2);
  });

  it("failed: says so, and Try again ↵ asks again", async () => {
    const failed: ChatTurn = { ...startTurn("When?"), state: "failed", error: { kind: "model", message: "x" } };
    const older: ChatTurn = { ...failed, question: "Before?" };
    const { actions, key, user } = setup({ pick: -1, turns: [older, failed] });
    expect(screen.getAllByText("Something went wrong before I could answer.")).toHaveLength(2);
    key("Enter");
    expect(actions.ask).toHaveBeenCalledWith("When?");
    await user.click(screen.getByRole("button", { name: "Try again" }));
    expect(actions.ask).toHaveBeenLastCalledWith("Before?");
  });

  it("with an email open, an answer can point at its messages and at other emails too", () => {
    setup({ pick: -1, turns: [done("Is this the latest draft?", { messages: ["41-1"], sources: [older] })] });
    expect(screen.getByRole("button", { name: "Dana · Oct 28" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /First draft/ })).toBeInTheDocument();
  });

  it("lists what Moss did; something done is not nothing found", () => {
    setup({ emailOpen: false, pick: -1, turns: [done("Mark the newsletters done", { text: "Done.", did: ["Marked done: Design weekly", "Marked done: Studio notes"] })] });
    const did = screen.getByRole("list", { name: "What Moss did" });
    expect(within(did).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["Marked done: Design weekly", "Marked done: Studio notes"]);
    expect(screen.queryByRole("button", { name: /Search instead/ })).not.toBeInTheDocument();
  });

  it("an email you asked to send later says when on its Send", async () => {
    const { actions, user } = setup({ sendsAt: () => "Sat 8 AM", turns: [done("Send it Saturday", { drafts: [draft] })] });
    await user.click(screen.getByRole("button", { name: "Send Sat 8 AMCtrl+↵" }));
    expect(actions.sendDraft).toHaveBeenCalledWith(draft);
  });

  it("with an email open, an answer with nothing to show offers no search", () => {
    setup({ pick: -1, turns: [done("Did they say when?", { text: "They didn't say." })] });
    expect(screen.queryByRole("button", { name: /Search instead/ })).not.toBeInTheDocument();
  });

  it("an answer shows the Markdown the model wrote", () => {
    const text = "Dana wants **two** things:\n\n- Sign the contract\n- Send `Q4.pdf`\n\n| When | What |\n| --- | --- |\n| Fri | Sign |";
    setup({ pick: -1, turns: [done("What does Dana want?", { text })] });
    expect(screen.getByText("two").tagName).toBe("STRONG");
    expect(screen.getByText("Sign the contract").tagName).toBe("LI");
    expect(screen.getByText("Q4.pdf").tagName).toBe("CODE");
    expect(screen.getByRole("cell", { name: "Fri" })).toBeInTheDocument();
    expect(screen.queryByText(/\*\*/)).not.toBeInTheDocument();
  });

  it("a stopped answer says so", () => {
    setup({ turns: [done("q", { state: "stopped", text: "Dana" })] });
    expect(screen.getByText("Stopped.")).toBeInTheDocument();
  });
});

it("shows every attachment and copied recipient before an AI draft can be sent", () => {
  const withFiles = { ...draft, cc: [person("Copy", "copy@example.test")], bcc: [person("Private", "private@example.test")], attachments: [{ id: "f1", name: "contract.pdf", bytes: 2048, contentType: "application/pdf" }] };
  setup({ turns: [done("Draft a reply", { drafts: [withFiles] })] });
  const card = screen.getByRole("group", { name: "Draft reply" });
  expect(within(card).getByRole("list", { name: "Attachments" })).toHaveTextContent("contract.pdf");
  expect(card).toHaveTextContent("2 KB");
  expect(card).toHaveTextContent("copy@example.test");
  expect(card).toHaveTextContent("private@example.test");
  expect(card).toHaveTextContent(dana.email);
});

it.each(["\u061c", "\u200e", "\u200f", "\u202a", "\u202b", "\u202c", "\u202d", "\u202e", "\u2066", "\u2067", "\u2068", "\u2069"])("keeps direction controls out of Ask's send header: %s", control => {
  setup({ turns: [done("Write a reply", { drafts: [draft] })], draftLine: () => `To Dana${control} · from maya@example.test` });
  expect(screen.getByText("To Dana · from maya@example.test").closest("bdi")).toHaveAttribute("dir", "ltr");
});

it("does not choose a draft implicitly when a turn has multiple sendable drafts", () => {
  const other = { ...draft, id: "second-draft", to: [person("Other", "other@example.test")] };
  const { actions, key } = setup({ turns: [done("Write two emails", { drafts: [draft, other] })] });
  key("Enter", { metaKey: true });
  key("Enter", { ctrlKey: true });
  expect(actions.sendDraft).not.toHaveBeenCalled();
  expect(screen.getAllByRole("button", {name: "Send"})).toHaveLength(2);
});

it("allows the shortcut when only one draft remains after dismissal", () => {
  const other = { ...draft, id: "second-draft" };
  const { actions, key } = setup({ dismissed: [draft.id], turns: [done("Write two emails", { drafts: [draft, other] })] });
  key("Enter", { metaKey: true });
  expect(actions.sendDraft).toHaveBeenCalledExactlyOnceWith(other);
});

it("restores email keyboard navigation after every draft has been dismissed", () => {
  const { actions, key } = setup({ emailOpen: false, pick: 0, dismissed: [draft.id], turns: [done("Find the contract", { sources: [contract, older], drafts: [draft] })] });
  key("ArrowDown");
  expect(actions.move).toHaveBeenCalledWith(1, 2);
  key("Enter");
  expect(actions.openEmail).toHaveBeenCalledWith(contract.id);
  expect(actions.sendDraft).not.toHaveBeenCalled();
});
