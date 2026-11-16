import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoAi, setAi, type AiStatus } from "../services/ai";
import { demoAsk, setAsk, startTurn } from "../services/ask";
import { fakeMail } from "../test/fakeMail";
import { message, person, personal, resetStore, thread, work } from "../test/fixtures";
import { resetAskPanel, useAskPanel, watchAsk } from "../state/askPanel";
import { useAiSettings } from "../state/aiSettings";
import { useScheduled } from "../state/scheduled";
import { useSearch } from "../state/search";
import { useStore } from "../state/store";
import { AskSide } from "./AskSide";

const dana = person("Dana Whitfield", "dana@acme.co", { short: "Dana" });
const budget = thread("41", {
  from: dana,
  subject: "Q4 budget",
  messages: [message({ id: "41-1", from: dana, time: "Oct 28" }), message({ id: "41-2", from: dana, time: "9:12 AM" })],
});
const lunch = thread("57", { subject: "Lunch", from: person("Priya Nair", "priya@nair.co") });

const panel = () => useAskPanel.getState();

let stop: () => void;
beforeEach(() => {
  resetStore([budget, lunch]);
  resetAskPanel();
  setAsk(demoAsk({ threads: [budget, lunch] }));
  stop = watchAsk();
});
afterEach(() => stop());

const open = async () => {
  render(<AskSide />);
  await act(async () => panel().show());
};

const thinking = (question: string) => ({ ...startTurn(question) });

describe("AskSide", () => {
  it("draws nothing while the panel is closed", () => {
    const { container } = render(<AskSide />);
    expect(container).toBeEmptyDOMElement();
  });

  it("with an email open: what it reads, no ready-made questions, and no tabs or scope to pick", async () => {
    useStore.setState({ view: "thread", openThreadId: "41" });
    await open();
    expect(screen.getByText("Ask about this conversation or any other email, or tell Moss what to do with your mail. Moss reads all 2 emails here first.")).toBeInTheDocument();
    expect(screen.queryByText("Try")).not.toBeInTheDocument();
    expect(screen.queryByRole("radiogroup")).not.toBeInTheDocument();
    expect(screen.queryByText("Q4 budget · 2 emails")).not.toBeInTheDocument();
    expect(screen.getByPlaceholderText("Ask about this email, or any other…")).toBeInTheDocument();
  });

  it("an email with one message reads it", async () => {
    useStore.setState({ view: "thread", openThreadId: "57" });
    await open();
    expect(screen.getByText(/Moss reads this email first\./)).toBeInTheDocument();
  });

  it("follows what's on screen: opening an email makes it what Moss reads, going back asks all mail", async () => {
    useStore.setState({ view: "inbox", cursorId: "41" });
    await open();
    expect(screen.getByPlaceholderText("Ask about any email…")).toBeInTheDocument();
    act(() => useStore.getState().open("41"));
    expect(screen.getByPlaceholderText("Ask about this email, or any other…")).toBeInTheDocument();
    act(() => useStore.setState({ view: "inbox" }));
    expect(screen.getByPlaceholderText("Ask about any email…")).toBeInTheDocument();
  });

  it("a chip names a sender with no short name by their first name", async () => {
    const priya = person("Priya Nair", "priya@nair.co");
    resetStore([thread("63", { from: priya, messages: [message({ id: "63-1", from: priya, time: "Oct 30" })] })]);
    setAsk({ ...demoAsk(), askMoss: async () => ({ text: "Yes.", sources: [], messages: ["63-1"], drafts: [], stopped: false }) });
    useStore.setState({ view: "thread", openThreadId: "63" });
    await open();
    await act(async () => void panel().ask("Did she agree?"));
    expect(screen.getByRole("button", { name: "Priya · Oct 30" })).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: /Draft a reply from this chat/ }));
    expect(panel().chats["moss-1"].at(-1)!.question).toBe("Draft a reply based on this chat.");
  });

  it("lists what Moss did to your mail", async () => {
    await open();
    await act(async () => void panel().ask("Mark the budget emails done"));
    const did = screen.getByRole("list", { name: "What Moss did" });
    expect(within(did).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["Marked done: Q4 budget", "Marked done: Lunch"]);
    expect(screen.queryByRole("button", { name: /Search instead/ }), "it did something").not.toBeInTheDocument();
  });

  it("asks, shows the answer with its source chips, and they open the message", async () => {
    useStore.setState({ view: "thread", openThreadId: "41" });
    await open();
    const user = userEvent.setup();
    await user.type(screen.getByRole("textbox"), "When is it due?{Enter}");
    await waitFor(() => expect(screen.getByText(/Friday/)).toBeInTheDocument());
    await user.click(screen.getByRole("button", { name: "Dana · Oct 28" }));
    expect(panel().reveal).toBe("41-1");
    expect(screen.getByRole("button", { name: "Dana · today" })).toBeInTheDocument();
  });

  it("a file chip opens the file the answer came from", async () => {
    const { backend } = fakeMail();
    await open();
    const file = { threadId: "41", messageId: "41-1", index: 2, name: "P2026-0457.pdf" };
    act(() => useAskPanel.setState({ chats: { "moss-1": [{ ...startTurn("When?"), state: "done", text: "On 10 December.", files: [file] }] } }));
    await userEvent.setup().click(screen.getByRole("button", { name: "Open P2026-0457.pdf" }));
    expect(backend.openAttachment).toHaveBeenCalledWith("41-1", 2);
  });

  it("while thinking about all mail, says how many emails it looks through", async () => {
    setAsk(demoAsk({ answer: "thinking" }));
    stop();
    stop = watchAsk();
    await open();
    await act(async () => void panel().ask("q"));
    await waitFor(() => expect(screen.getByText(/Searching mail for “q”/)).toBeInTheDocument());
    act(() => useAskPanel.setState((s) => ({ chats: { ...s.chats, "moss-1": [{ ...s.chats["moss-1"][0], activity: undefined }] } })));
    expect(screen.getByText("Looking through 2,431 emails…")).toBeInTheDocument();
    act(() => useAskPanel.setState({ mailCount: undefined }));
    expect(screen.getByText("Looking through your mail…")).toBeInTheDocument();
    act(() => panel().stop());
  });

  it("thinking about one email reads it", async () => {
    setAsk(demoAsk({ answer: "thinking" }));
    useStore.setState({ view: "thread", openThreadId: "41" });
    await open();
    act(() => useAskPanel.setState({ chats: { "moss-1": [thinking("q")] } }));
    expect(screen.getByText("Reading 2 emails…")).toBeInTheDocument();
    act(() => useStore.setState({ openThreadId: "57" }));
    expect(screen.getByText("Reading the email…")).toBeInTheDocument();
  });

  it("no model: names the writing model and its size; Download and AI settings work", async () => {
    setAi(demoAi({ download: { state: "missing", done: 0, total: 1 }, features: { search: true, sorting: true, writing: false } }));
    await act(async () => void useAiSettings.getState().loadStatus());
    setAsk(demoAsk({ answer: "missing" }));
    await open();
    await act(async () => void panel().ask("q"));
    expect(screen.getByRole("alert")).toHaveTextContent("Ask needs the writing model. Gemma 4 E4B runs on this");
    expect(screen.getByRole("alert")).toHaveTextContent(", about 5 GB.");
    const download = vi.spyOn(useAiSettings.getState(), "download");
    useAiSettings.setState({ download });
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /^Download/ }));
    expect(download).toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: /^AI settings/ }));
    expect(useStore.getState()).toMatchObject({ view: "settings", settingsSection: "ai" });
  });

  it("no model, before the AI status is known, still says what's needed", async () => {
    useAiSettings.setState({ status: undefined });
    setAsk(demoAsk({ answer: "missing" }));
    await open();
    await act(async () => void panel().ask("q"));
    expect(screen.getByRole("alert")).toHaveTextContent("Ask needs the writing model. It runs on this");
  });

  it("drafts: a reply to the open email, or to another one, and Edit, Send, Not now and Open", async () => {
    useStore.setState({ view: "thread", openThreadId: "41" });
    await open();
    await act(async () => void panel().ask("Draft a reply that says yes"));
    expect(screen.getByText(`Reply to Dana Whitfield · from ${work.address}`)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Not now" }), "it replies to what's open").not.toBeInTheDocument();
    const user = userEvent.setup();
    act(() => useStore.setState({ view: "inbox" }));
    await act(async () => void panel().ask("draft a reply to Dana"));
    const card = screen.getAllByRole("group", { name: "Draft reply" }).at(-1)!;
    expect(within(card).getByText(`To Dana Whitfield · from ${work.address}`)).toBeInTheDocument();
    await user.click(within(card).getByRole("button", { name: "Not now" }));
    expect(screen.getAllByRole("group", { name: "Draft reply" })).toHaveLength(1);
    await act(async () => void panel().ask("draft another reply"));
    const another = () => screen.getAllByRole("group", { name: "Draft reply" }).at(-1)!;
    await user.click(within(another()).getByRole("button", { name: /Open/ }));
    await waitFor(() => expect(useStore.getState()).toMatchObject({ view: "thread", openThreadId: "41" }));
    act(() => useStore.setState({ view: "inbox" }));
    await user.click(within(another()).getByRole("button", { name: /^Send/ }));
    expect(useStore.getState().toast).toMatchObject({ text: "Sending" });
    await act(async () => void panel().ask("one more reply"));
    await user.click(within(another()).getByRole("button", { name: /^Edit/ }));
    expect(useStore.getState().view).toBe("compose");
  });

  it("an email you asked to go later says when on its Send, and goes then", async () => {
    fakeMail();
    const at = Date.now() + 3 * 86_400_000;
    const draft = { ...(await demoAsk({ threads: [budget] }).askMoss("x", "reply")).drafts[0], id: "later" };
    setAsk({ ...demoAsk(), askMoss: async () => ({ text: "Ready.", sources: [], messages: [], drafts: [draft], sendAt: { later: at }, stopped: false }) });
    await open();
    await act(async () => void panel().ask("Reply yes, send it in three days"));
    const send = within(screen.getByRole("group", { name: "Draft reply" })).getByRole("button", { name: /^Send/ });
    expect(send.textContent).toMatch(/^Send \w{3} /);
    await userEvent.setup().click(send);
    expect(useStore.getState().toast?.text).toMatch(/^Scheduled for /);
    expect(useScheduled.getState().emails).toMatchObject([{ draft: { id: "later" }, sendAt: at }]);
  });

  it("a draft from another account says which address it goes from", async () => {
    const other = thread("70", { accountId: "personal", from: dana, subject: "Trip" });
    resetStore([other]);
    setAsk(demoAsk({ threads: [other] }));
    await open();
    await act(async () => void panel().ask("reply yes"));
    expect(screen.getByText(`To Dana Whitfield · from ${personal.address}`)).toBeInTheDocument();
  });

  it("cards, Close and New chat reach the panel", async () => {
    await open();
    const user = userEvent.setup();
    await act(async () => void panel().ask("Did Dana write?"));
    await user.click(screen.getByRole("button", { name: /^Dana Whitfield/ }));
    expect(useStore.getState().openThreadId).toBe("41");
    await user.click(screen.getByRole("button", { name: "New chat" }));
    expect(panel().chat).toBe("moss-2");
    await user.keyboard("{ArrowDown}");
    await user.click(screen.getByRole("button", { name: /Close/ }));
    expect(panel().open).toBe(false);
  });

  it("nothing found: Search instead runs the question as a search", async () => {
    setAsk(demoAsk({ answer: "nothing" }));
    await open();
    await act(async () => void panel().ask("bank pdf"));
    await userEvent.setup().click(screen.getByRole("button", { name: /Search instead/ }));
    expect(useStore.getState().view).toBe("search");
    expect(useSearch.getState().query).toBe("bank pdf");
    expect(panel().open).toBe(false);
  });
});

it("an AI status with no such writing model falls back to the plain line", async () => {
  const status = { ...(await demoAi().status()), writer: "gone" } as AiStatus;
  useAiSettings.setState({ status });
  resetStore([budget]);
  resetAskPanel();
  setAsk(demoAsk({ answer: "missing" }));
  render(<AskSide />);
  await act(async () => panel().show());
  await act(async () => void panel().ask("q"));
  expect(screen.getByRole("alert")).toHaveTextContent("It runs on this");
});
