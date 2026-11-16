import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { draft, lena, message, person, resetStore, thread } from "../test/fixtures";
import { useDrafts } from "../state/drafts";
import { demoReplies, setReplies } from "../services/replies";
import { useAiSettings } from "../state/aiSettings";
import { useMossWriting } from "../state/mossWriting";
import { useQuickReply } from "../state/quickReply";
import { useSignatures } from "../state/signatures";
import { useAskPanel } from "../state/askPanel";
import { useStore, type SendStep } from "../state/store";
import { ThreadScreen } from "./ThreadScreen";
import { Profiler } from "react";

// Counts how often an email's frame is drawn, newest or earlier.
const frames = vi.hoisted(() => ({ renders: 0 }));
vi.mock("../design-system/organisms/EmailBody/EmailBody", async (importOriginal) => {
  const real = await importOriginal<typeof import("../design-system/organisms/EmailBody/EmailBody")>();
  const { createElement } = await import("react");
  return {
    ...real,
    EmailBody: (props: Parameters<typeof real.EmailBody>[0]) => {
      frames.renders++;
      return createElement(real.EmailBody, props);
    },
  };
});

const s = () => useStore.getState();
const options = [{ label: "Approve it", draft: "Approved." }, { label: "Need more time", draft: "Later." }];
const demoReplyText = "Hi,\n\nYes, that works for me. Thanks for sorting it out.";

beforeEach(() =>
  resetStore([thread("a", { replyOptions: options }), thread("b", { subject: "Plain" }), thread("c", { section: "info" })]),
);

describe("a reply of yours on its way", () => {
  const held = (overrides: Partial<SendStep["draft"]> = {}, step: Partial<SendStep> = {}): SendStep => ({
    kind: "send", hold: "h1", until: Date.now() + 6000,
    draft: { ...draft("held", { mode: "reply", threadId: "a", to: [lena], subject: "Re: a", body: "Yes, Friday works.\n\nMaya", bodyHtml: "" }), ...overrides },
    ...step,
  });

  it("takes the newest message's place while it waits to go, as it will read once sent, marked Sending", () => {
    useStore.setState({ undoSteps: [held()] });
    s().open("a");
    render(<ThreadScreen />);
    const yours = screen.getByRole("group", { name: "Your reply, sending" });
    expect(within(yours).getByText("Sending…")).toBeInTheDocument();
    expect(within(yours).getByText("Yes, Friday works.")).toBeInTheDocument();
    expect(within(yours).getByText("Maya")).toBeInTheDocument();
    expect(within(yours).getByText("Maya Ross")).toBeInTheDocument();
    // The email it answers folds to a row above it, as it does once your reply has gone.
    expect(screen.getByRole("button", { name: /Lena Vogt\s*Hello\s*9:12 AM/ })).toBeInTheDocument();
  });

  it("ends with its signature, as the engine adds it", () => {
    useStore.setState({ undoSteps: [held({ signatureId: "work-full" })] });
    s().open("a");
    render(<ThreadScreen />);
    const yours = screen.getByRole("group", { name: "Your reply, sending" });
    expect(yours).toHaveTextContent(/Yes, Friday works\.\s*Maya\s*-- Maya Ross\s*Design lead, Northwind\s*northwind\.co/);
  });

  it("opens its recipients like the newest message's", async () => {
    useStore.setState({ undoSteps: [held()] });
    s().open("a");
    render(<ThreadScreen />);
    const yours = screen.getByRole("group", { name: "Your reply, sending" });
    await userEvent.click(within(yours).getByRole("button", { name: /to Lena/ }));
    expect(s().recipientsOpen).toBe(true);
  });

  it("of two on their way, the first folds to a row marked Sending", () => {
    useStore.setState({ undoSteps: [held({ body: "First go" }), held({ id: "again", body: "Second go" }, { hold: "h2" })] });
    s().open("a");
    render(<ThreadScreen />);
    expect(screen.getByRole("button", { name: /You\s*First go\s*Sending…/ })).toBeInTheDocument();
    expect(within(screen.getByRole("group", { name: "Your reply, sending" })).getByText("Second go")).toBeInTheDocument();
  });

  it("shows its formatting, signature and files as they will go; its files can't be opened yet", async () => {
    const openAttachment = vi.fn();
    useStore.setState({ openAttachment, undoSteps: [held({ signatureId: "work-short", bodyHtml: "<p>Yes, <b>Friday</b> works.</p>", attachments: [{ id: "f1", name: "plan.pdf", bytes: 2048, contentType: "application/pdf" }] })] });
    s().open("a");
    render(<ThreadScreen />);
    const yours = screen.getByRole("group", { name: "Your reply, sending" });
    const frame = within(yours).getByTitle("Email") as HTMLIFrameElement;
    expect(frame.getAttribute("srcdoc")).toContain('<div class="gmail_signature">Maya</div>');
    await userEvent.click(within(yours).getByText("plan.pdf"));
    expect(openAttachment).not.toHaveBeenCalled();
  });

  it("adds nothing for a blank signature, as the engine doesn't", () => {
    useSignatures.setState({ signatures: [{ id: "blank", name: "Blank", lines: ["  ", ""] }] });
    useStore.setState({ undoSteps: [held({ signatureId: "blank" })] });
    s().open("a");
    render(<ThreadScreen />);
    const yours = screen.getByRole("group", { name: "Your reply, sending" });
    expect(yours).toHaveTextContent("Yes, Friday works.");
    expect(yours).not.toHaveTextContent("--");
  });

  it("puts Moss's suggested replies away while it waits: you've answered", () => {
    useStore.setState({ undoSteps: [held()] });
    s().open("a");
    render(<ThreadScreen />);
    expect(screen.queryByText("Approve it")).not.toBeInTheDocument();
    expect(screen.getByText("2 suggested replies ready")).toBeInTheDocument();
  });

  it.each([
    ["a forward, which starts a conversation of its own", held({ mode: "forward" })],
    ["a reply that couldn't go", held({}, { failed: "No connection" })],
    ["a reply to another email", held({ threadId: "b" })],
  ])("leaves out %s", (_, step) => {
    useStore.setState({ undoSteps: [step] });
    s().open("a");
    render(<ThreadScreen />);
    expect(screen.queryByRole("group", { name: "Your reply, sending" })).not.toBeInTheDocument();
  });
});

describe("a draft of yours in the conversation", () => {
  it("shows files and all recipients beside the inline Send button", () => {
    const saved = draft("d1", { mode: "reply", threadId: "a", to: [lena], cc: [person("Copy", "copy@example.test")], bcc: [person("Private", "private@example.test")], attachments: [{ id: "f1", name: "contract.pdf", bytes: 2048, contentType: "application/pdf" }] });
    useDrafts.setState({ drafts: [saved] });
    s().open("a");
    render(<ThreadScreen />);
    const card = screen.getByRole("region", { name: "Your draft" });
    expect(within(card).getByRole("list", { name: "Attachments" })).toHaveTextContent("contract.pdf");
    expect(card).toHaveTextContent("copy@example.test");
    expect(card).toHaveTextContent("private@example.test");
    expect(card).toHaveTextContent(lena.email);
  });

  const mine = () => draft("d1", { mode: "reply", threadId: "a", to: [lena], subject: "Re: Subject a", body: "Hi Lena,\n\nYes, I'll text you soon.", updatedAt: Date.now() - 3 * 60_000 });
  beforeEach(() => useDrafts.setState({ drafts: [mine(), draft("other", { mode: "reply", threadId: "b", body: "Not this one" })] }));
  afterEach(() => useDrafts.setState({ drafts: [] }));

  it("shows at the end of it, and the suggested replies step aside for it", () => {
    s().open("a");
    render(<ThreadScreen />);
    const card = screen.getByRole("region", { name: "Your draft" });
    expect(card).toHaveTextContent("To Lena Vogt · from maya@northwind.co");
    expect(card).toHaveTextContent(/Not sent yet · Saved (at|on) /);
    expect(card).toHaveTextContent("Yes, I'll text you soon.");
    expect(screen.queryByText("Not this one")).not.toBeInTheDocument();
    expect(screen.queryByText("Approve it")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reply allShift+R" })).toBeInTheDocument();
  });

  it("one saved just now says so; a conversation without one shows none", () => {
    useDrafts.setState({ drafts: [{ ...mine(), updatedAt: Date.now() }] });
    s().open("a");
    const { unmount } = render(<ThreadScreen />);
    expect(screen.getByRole("region", { name: "Your draft" })).toHaveTextContent("Saved just now");
    unmount();
    s().open("c");
    render(<ThreadScreen />);
    expect(screen.queryByRole("region", { name: "Your draft" })).not.toBeInTheDocument();
  });

  it("of two there, the newer shows; one to a bare address from another of yours, saved days ago, says so", () => {
    const old = Date.now() - 3 * 86_400_000;
    useDrafts.setState({
      drafts: [
        { ...mine(), body: "The older one", updatedAt: old - 1000 },
        draft("d2", { mode: "reply", threadId: "a", to: [person("", "sam@acme.co")], fromAddress: "hello@northwind.co", body: "The newer one", updatedAt: old }),
      ],
    });
    s().open("a");
    render(<ThreadScreen />);
    const card = screen.getByRole("region", { name: "Your draft" });
    expect(card).toHaveTextContent("To sam@acme.co · from hello@northwind.co");
    expect(card).toHaveTextContent(/Saved on /);
    expect(card).toHaveTextContent("The newer one");
    expect(card).not.toHaveTextContent("The older one");
  });

  it("Edit opens it in the composer, Delete throws it away, Send sends it", async () => {
    const user = userEvent.setup();
    s().open("a");
    const { unmount } = render(<ThreadScreen />);
    await user.click(screen.getByRole("button", { name: "EditCtrl+E" }));
    expect(s()).toMatchObject({ view: "compose", draft: { id: "d1" } });
    unmount();
    s().open("a");
    const second = render(<ThreadScreen />);
    await user.click(screen.getByRole("button", { name: "Delete" }));
    expect(useDrafts.getState().drafts.map((d) => d.id)).toEqual(["other"]);
    expect(s().toast?.text).toBe("Draft deleted");
    second.unmount();
    useDrafts.setState({ drafts: [mine()] });
    s().open("a");
    render(<ThreadScreen />);
    await user.click(screen.getByRole("button", { name: "SendCtrl+↵" }));
    expect(useDrafts.getState().drafts).toEqual([]);
    expect(s().toast?.text).toBe("Sending");
  });
});

describe("ThreadScreen: re-rendering", () => {
  const conversation = () =>
    resetStore([
      thread("a", {
        replyOptions: options,
        messages: [message({ id: "a-1", paragraphs: ["Can you estimate the cabinet?"], time: "Sep 24" }), message({ id: "a-2", paragraphs: ["The sample is ready."] })],
      }),
    ]);
  const pristineAsk = useAskPanel.getState();
  const pristineQuick = useQuickReply.getState();
  afterEach(() => {
    useAskPanel.setState(pristineAsk, true);
    useQuickReply.setState(pristineQuick, true);
  });
  const askToken = (text: string) =>
    act(() => useAskPanel.setState((a) => ({ input: text, chats: { ...a.chats, [a.chat]: [{ question: "Summarise", answer: text } as never] } })));

  it("an Ask answer streaming in doesn't redraw the email", async () => {
    conversation();
    s().open("a");
    const renders = vi.fn();
    render(
      <Profiler id="thread" onRender={renders}>
        <ThreadScreen />
      </Profiler>,
    );
    await userEvent.setup().click(screen.getByRole("button", { name: /Can you estimate the cabinet\?Sep 24/ }));
    expect(screen.getAllByRole("article")).toHaveLength(2);
    renders.mockClear();
    frames.renders = 0;
    askToken("The");
    askToken("The sample");
    expect(renders).not.toHaveBeenCalled();
    expect(frames.renders).toBe(0);
  });

  it("typing a Quick reply doesn't redraw the email, only the reply", async () => {
    conversation();
    s().open("a");
    render(<ThreadScreen />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /Can you estimate the cabinet\?Sep 24/ }));
    frames.renders = 0;
    await user.type(screen.getByRole("textbox", { name: "Reply" }), " Thanks");
    await user.click(screen.getByRole("radio", { name: "Need more time" }));
    expect(screen.getByRole("textbox", { name: "Reply" })).toHaveValue("Later.");
    expect(s().replyChoice).toBe(1);
    expect(frames.renders).toBe(0);
  });

  it("Quick reply starting, writing and stopping doesn't redraw the email", () => {
    conversation();
    s().open("a");
    render(<ThreadScreen />);
    frames.renders = 0;
    act(() => useQuickReply.setState({ threadId: "a", status: "writing" }));
    expect(screen.getByRole("button", { name: /Stop/ })).toBeInTheDocument();
    act(() => useQuickReply.setState({ threadId: undefined, status: undefined }));
    expect(frames.renders).toBe(0);
  });
});

describe("ThreadScreen", () => {
  it("has an Ask ⌘J button that opens Ask Moss on this email, raised while it's open", async () => {
    s().open("b");
    render(<ThreadScreen />);
    const ask = screen.getByRole("button", { name: "AskCtrl+J" });
    expect(ask).toHaveAttribute("aria-pressed", "false");
    await userEvent.setup().click(ask);
    expect(useAskPanel.getState()).toMatchObject({ open: true, pick: -1 });
    expect(ask).toHaveAttribute("aria-pressed", "true");
  });

  it("opens a message Ask Moss cited, earlier or the newest, where the panel can scroll to it", () => {
    resetStore([thread("t", { messages: [message({ id: "t-1", paragraphs: ["First"] }), message({ id: "t-2", paragraphs: ["Second"] })] })]);
    s().open("t");
    const { container } = render(<ThreadScreen />);
    expect(container.querySelector('[data-message="t-2"]')).toHaveTextContent("Second");
    act(() => useAskPanel.setState({ reveal: "t-1" }));
    expect(container.querySelector('[data-message="t-1"]')).toHaveTextContent("First");
  });

  it("says when the AI sorted the email, and how sure it was", () => {
    resetStore([
      thread("r", { section: "reply", aiSure: 95 }),
      thread("i", { section: "info", aiSure: 88 }),
      thread("l", { section: "later", aiSure: 90 }),
    ]);
    s().open("r");
    const { rerender } = render(<ThreadScreen />);
    expect(screen.getByText("Needs your reply · AI, 95% sure")).toBeInTheDocument();
    s().open("i");
    rerender(<ThreadScreen />);
    expect(screen.getByText("For your information · AI, 88% sure")).toBeInTheDocument();
    s().open("l");
    rerender(<ThreadScreen />);
    expect(screen.queryByText(/% sure/)).not.toBeInTheDocument();
  });

  it("says each decision Moss made, and who made it", () => {
    resetStore([
      thread("d", {
        split: "receipts",
        section: "info",
        splitWhy: { by: "ai", sure: 88 },
        due: "Due Fri",
        dueDate: "2099-11-06",
        dueWhy: { by: "rule", evidence: "by Friday" },
      }),
    ]);
    s().open("d");
    render(<ThreadScreen />);
    const why = screen.getByRole("list", { name: "Why Moss put it here" });
    expect(within(why).getByText("Moved to Receipts · AI, 88% sure")).toBeInTheDocument();
    expect(within(why).getByText(/^Due .* · says “by Friday”$/)).toBeInTheDocument();
  });

  it("Remind me if no reply waits on the email you sent", async () => {
    resetStore([thread("w", { youWroteLast: true, lastMessageAt: Date.now() })]);
    s().open("w");
    render(<ThreadScreen />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "RemindH" }));
    await user.click(screen.getByRole("button", { name: "Remind me if no replyShift+H" }));
    expect(s().threads[0]).toMatchObject({ section: "later", waiting: { why: { by: "you" } } });
  });

  it("shows an HTML email's images when asked, in the app's theme", async () => {
    const html = `<p>Sale</p><img src="https://img.example/hero.jpg" alt="Hero">`;
    resetStore([thread("n", { messages: [message({ id: "n-1", html })] })]);
    useStore.setState({ theme: "light", readingSize: "larger" });
    s().open("n");
    render(<ThreadScreen />);
    const frame = screen.getByTitle("Email") as HTMLIFrameElement;
    expect(frame.srcdoc).toContain('data-theme="light" data-reading="larger"');
    expect(frame.srcdoc).not.toContain("img.example");
    await userEvent.setup().click(screen.getByRole("button", { name: /Just this once/ }));
    expect(s().imagesShown).toEqual(["n-1"]);
    expect((screen.getByTitle("Email") as HTMLIFrameElement).srcdoc).toContain("https://img.example/hero.jpg");
  });

  it("says you moved it to Important, and Move opens the menu for it", async () => {
    resetStore([thread("m", { movedByYou: true })]);
    s().open("m");
    render(<ThreadScreen />);
    expect(screen.getByText("You moved this to Important")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "MoveM" }));
    expect(s().movingId).toBe("m");
  });

  it("shows images from the newest message's sender from now on", async () => {
    resetStore([thread("n", { messages: [message({ id: "n-1", html: `<p>Sale</p><img src="https://img.example/hero.jpg" alt="">` })] })]);
    s().open("n");
    render(<ThreadScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: /Always for this sender/ }));
    expect(s().threads[0].messages[0].imageTrust).toBe("you-allowed");
    expect((screen.getByTitle("Email") as HTMLIFrameElement).srcdoc).toContain("https://img.example/hero.jpg");
  });

  it("shows the earlier messages above the newest one, and opens one in full when asked", async () => {
    resetStore([
      thread("t", {
        messages: [
          message({ id: "t-1", paragraphs: ["Can you estimate the display cabinet?"], time: "Sep 24" }),
          message({ id: "t-2", paragraphs: ["The sample is ready."] }),
        ],
      }),
    ]);
    s().open("t");
    render(<ThreadScreen />);
    expect(screen.getByText("The sample is ready.")).toBeInTheDocument();
    const row = screen.getByRole("button", { name: /Can you estimate the display cabinet\?Sep 24/ });
    await userEvent.setup().click(row);
    expect(screen.getByText("Can you estimate the display cabinet?").tagName).toBe("P");
    expect(screen.getAllByRole("article")).toHaveLength(2);
  });

  it("an opened earlier message shows its own recipients and images, apart from the newest one", async () => {
    const html = `<p>Photos</p><img src="https://img.example/tiles.jpg" alt="Tiles">`;
    resetStore([thread("t", { messages: [message({ id: "t-1", html, paragraphs: ["Photos"] }), message({ id: "t-2" })] })]);
    s().open("t");
    render(<ThreadScreen />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /Photos/ }));
    const [earlier] = screen.getAllByRole("article");
    await user.click(within(earlier).getByRole("button", { name: /to you/ }));
    expect(within(earlier).getByRole("button", { name: /to you/ })).toHaveAttribute("aria-expanded", "true");
    expect(s().recipientsOpen).toBe(false);
    await user.click(within(earlier).getByRole("button", { name: /to you/ }));
    expect(within(earlier).getByRole("button", { name: /to you/ })).toHaveAttribute("aria-expanded", "false");
    await user.click(within(earlier).getByRole("button", { name: /Just this once/ }));
    expect(s().imagesShown).toEqual(["t-1"]);
  });

  it("showing images from a sender loads them in the newest and earlier messages", async () => {
    const html = `<p>Photos</p><img src="https://img.example/tiles.jpg" alt="Tiles">`;
    resetStore([thread("t", { messages: [message({ id: "t-1", html, paragraphs: ["Photos"] }), message({ id: "t-2", html })] })]);
    s().open("t");
    render(<ThreadScreen />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /Photos/ }));
    const [earlier] = screen.getAllByRole("article");
    await user.click(within(earlier).getByRole("button", { name: /Always for this sender/ }));
    expect(s().threads[0].messages.map((m) => m.imageTrust)).toEqual(["you-allowed", "you-allowed"]);
    expect(screen.queryByRole("button", { name: /Always for this sender/ })).not.toBeInTheDocument();
    expect(screen.getAllByTitle("Email")).toHaveLength(2);
    expect(screen.getAllByTitle("Email").every((f) => (f as HTMLIFrameElement).srcdoc.includes("img.example/tiles.jpg"))).toBe(true);
  });

  it("clicking an attachment opens it", async () => {
    const { backend } = fakeMail();
    resetStore([thread("n", { messages: [message({ id: "n-1", attachments: [{ index: 1, name: "Q4.pdf", size: "7 B" }] })] })]);
    s().open("n");
    render(<ThreadScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: /Q4\.pdf/ }));
    expect(backend.openAttachment).toHaveBeenCalledWith("n-1", 1);
  });

  it("shows the thumbnails the mail engine makes for an email's files", async () => {
    fakeMail({ attachmentPreview: async () => "data:,page" });
    resetStore([thread("n", { messages: [message({ id: "n-1", attachments: [{ index: 1, name: "Q4.pdf", size: "7 B" }] })] })]);
    s().open("n");
    render(<ThreadScreen />);
    const file = screen.getByRole("button", { name: "Q4.pdf, PDF document, 7 B" });
    await vi.waitFor(() => expect(file.querySelector("img")).toHaveAttribute("src", "data:,page"));
  });

  it("shows nothing without an open email", () => {
    const { container } = render(<ThreadScreen />);
    expect(container).toBeEmptyDOMElement();
  });

  it("shows where you are among emails in the same section", () => {
    s().open("b");
    render(<ThreadScreen />);
    expect(screen.getByText("2 of 2")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Important/ })).toBeInTheDocument();
  });

  it("the position hint shows the arrow keys, not J and K", () => {
    s().open("b");
    const { container } = render(<ThreadScreen />);
    const keys = [...container.querySelectorAll("kbd")].map((k) => k.textContent);
    expect(keys).toEqual(expect.arrayContaining(["↑", "↓"]));
    expect(keys).not.toContain("J");
    expect(keys).not.toContain("K");
  });

  it("offers Quick replies you can change in place, then open in the full editor", async () => {
    s().open("a");
    render(<ThreadScreen />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("radio", { name: "Need more time" }));
    expect(s().replyChoice).toBe(1);
    await user.type(screen.getByRole("textbox", { name: "Reply" }), " Friday?");
    expect(s().replyDraft).toBe("Later. Friday?");
    await user.click(screen.getByRole("button", { name: /^Full editor/ }));
    expect(s().draft).toMatchObject({ body: "Later. Friday?" });
  });

  it("Change it opens the reply in the composer with Write with Moss open", async () => {
    s().open("a");
    render(<ThreadScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: /^Change it/ }));
    expect(s()).toMatchObject({ view: "compose", draft: { body: "Approved." } });
    expect(useMossWriting.getState().menuOpen).toBe(true);
    useMossWriting.getState().reset();
  });

  it("send from a Quick reply sends it and leaves the email in the inbox", async () => {
    s().open("a");
    render(<ThreadScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: /^Send/ }));
    expect(s()).toMatchObject({ sending: true, view: "thread" });
    expect(s().threads.map((t) => t.id)).toContain("a");
  });

  it("Quick reply in the reply bar asks Moss, and the replies land in the card", async () => {
    setReplies(demoReplies());
    s().open("b");
    const { rerender } = render(<ThreadScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: "QQuick reply" }));
    rerender(<ThreadScreen />);
    await vi.waitFor(() => expect(screen.getAllByRole("radio")).toHaveLength(3));
    expect(screen.getByRole("textbox", { name: "Reply" })).toHaveValue(demoReplyText);
    useQuickReply.getState().close();
  });

  it("the Quick reply shows the signature it sends with, and Discard brings the reply bar back", async () => {
    useSignatures.setState({
      signatures: [{ id: "short", name: "Short", lines: ["Best", "Maya"] }],
      byAccount: { work: { newEmails: null, replies: "short" } },
      byAddress: {},
    });
    s().open("a");
    render(<ThreadScreen />);
    const user = userEvent.setup();
    expect(screen.getByRole("group", { name: "Signature" })).toHaveTextContent("BestMaya");
    expect(screen.getByText("Your reply signature for maya@northwind.co")).toBeInTheDocument();
    // The chip picks another one, or none.
    await user.click(screen.getByRole("button", { name: "Change signature, Short" }));
    expect(screen.getByText("Picked for replies from maya@northwind.co.")).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: /None/ }));
    expect(screen.getByText("Chosen for this email")).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "Signature" })).not.toBeInTheDocument();
    expect(screen.getByRole("group", { name: "Signature on this email" })).toHaveTextContent("No signature");
    // Add brings it back; Remove takes it off again.
    await user.click(screen.getByRole("button", { name: "Add signatureAlt+S" }));
    await user.click(screen.getByRole("radio", { name: /^Short/ }));
    expect(screen.getByRole("group", { name: "Signature" })).toHaveTextContent("BestMaya");
    await user.click(screen.getByRole("button", { name: "Remove signature" }));
    expect(useQuickReply.getState().signature).toEqual({ threadId: "a", id: null });
    // Edit signatures goes to Settings.
    await user.click(screen.getByRole("button", { name: "Add signatureAlt+S" }));
    await user.click(screen.getByRole("button", { name: /^Edit signatures/ }));
    expect(s().view).toBe("settings");
    useQuickReply.getState().toggleSignature();
    s().open("a");
    await user.click(screen.getByRole("button", { name: /^Discard/ }));
    expect(screen.queryByRole("region", { name: "Quick reply" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "QQuick reply" })).toBeInTheDocument();
  });

  it("a Quick reply in a back-and-forth says why it has no signature", () => {
    resetStore([thread("a", { replyOptions: options, messages: [message(), message({ id: "a-2", from: person("Maya Ross", "maya@northwind.co") })] })]);
    useSignatures.setState({
      signatures: [{ id: "full", name: "Full", lines: ["Maya Ross", "Northwind"] }],
      byAccount: { work: { newEmails: "full", replies: "full" } },
      byAddress: {},
    });
    s().open("a");
    render(<ThreadScreen />);
    expect(screen.getByRole("group", { name: "Signature on this email" })).toHaveTextContent(
      "No signatureAdd signatureAlt+SLeft out: quick reply in a back-and-forth",
    );
  });

  it("a Quick reply whose replies are gone shows the reply bar, not an empty card", () => {
    s().open("b");
    useQuickReply.setState({ threadId: "b", status: undefined });
    render(<ThreadScreen />);
    expect(screen.queryByRole("region", { name: "Quick reply" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "QQuick reply" })).toBeInTheDocument();
    useQuickReply.getState().close();
  });

  it("while Moss writes, Stop closes the card", async () => {
    s().open("b");
    useQuickReply.setState({ threadId: "b", status: "writing" });
    render(<ThreadScreen />);
    expect(screen.getByRole("status")).toHaveTextContent("Moss is writing…");
    await userEvent.setup().click(screen.getByRole("button", { name: "StopEsc" }));
    expect(useQuickReply.getState().threadId).toBeUndefined();
  });

  it("without the writing model, offers the download and AI settings", async () => {
    const download = vi.fn(async () => {});
    useAiSettings.setState({ download });
    s().open("b");
    useQuickReply.setState({ threadId: "b", status: "missing" });
    const user = userEvent.setup();
    const { unmount } = render(<ThreadScreen />);
    await user.click(screen.getByRole("button", { name: "Download↵" }));
    expect(download).toHaveBeenCalledOnce();
    expect(useQuickReply.getState().threadId).toBeUndefined();
    unmount();
    useQuickReply.setState({ threadId: "b", status: "missing" });
    render(<ThreadScreen />);
    await user.click(screen.getByRole("button", { name: "AI settingsCtrl+," }));
    expect(s()).toMatchObject({ view: "settings", settingsSection: "ai" });
    useQuickReply.getState().close();
  });

  it("after a failure, Try again asks again", async () => {
    setReplies(demoReplies());
    s().open("b");
    useQuickReply.setState({ threadId: "b", status: "failed" });
    render(<ThreadScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again↵" }));
    await vi.waitFor(() => expect(s().threads.find((t) => t.id === "b")?.replyOptions).toHaveLength(3));
    useQuickReply.getState().close();
  });

  it("swaps instant replies for the reply bar while recipients are open", () => {
    s().open("a");
    s().toggleRecipients();
    render(<ThreadScreen />);
    expect(screen.queryByRole("radiogroup")).not.toBeInTheDocument();
    expect(screen.getByText("2 suggested replies ready")).toBeInTheDocument();
  });

  it("the reply field starts a reply, reply all or forward", async () => {
    const user = userEvent.setup();
    for (const [name, mode] of [
      [/^Reply to Lena…R$/, "reply"],
      [/Reply all$/, "reply-all"],
      [/Forward$/, "forward"],
    ] as const) {
      s().open("b");
      const { unmount } = render(<ThreadScreen />);
      await user.click(screen.getByRole("button", { name }));
      expect(s().draft?.mode).toBe(mode);
      unmount();
    }
  });

  it("shows the summary when the email has no message body yet", () => {
    resetStore([thread("x", { messages: [], summary: "Booking confirmed", split: "receipts" })]);
    s().setSplit("receipts");
    s().open("x");
    render(<ThreadScreen />);
    expect(screen.getByText("Booking confirmed")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Receipts/ })).toBeInTheDocument();
  });

  it("falls back to “Inbox” for a split it doesn’t know", () => {
    resetStore([thread("x", { split: "unknown" as never })]);
    useStore.setState({ split: "unknown" as never });
    s().open("x");
    render(<ThreadScreen />);
    expect(screen.getByRole("button", { name: /^Inbox/ })).toBeInTheDocument();
  });

  it("a done email says where it is, and its quick key brings it back", async () => {
    useStore.setState({ doneThreads: [thread("x", { doneAt: 2 }), thread("y", { doneAt: 1 })] });
    s().openDone();
    s().open("y");
    render(<ThreadScreen />);
    expect(screen.getByText("2 of 2")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: /Not done/ }));
    expect(s().threads.map((t) => t.id)).toContain("y");
    await userEvent.click(screen.getByRole("button", { name: "DoneEsc" }));
    expect(s().view).toBe("done");
  });

  it("back returns to the list", async () => {
    s().open("b");
    render(<ThreadScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: /Important/ }));
    expect(s().view).toBe("inbox");
  });

  it("back from an email opened in Search says Search", () => {
    s().openSearch();
    s().open("b");
    render(<ThreadScreen />);
    expect(screen.getByRole("button", { name: "SearchEsc" })).toBeInTheDocument();
  });

  it("the top bar reminds you, and marks the email done", async () => {
    resetStore([thread("p"), thread("q")]);
    s().open("p");
    render(<ThreadScreen />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "RemindH" }));
    await user.click(screen.getByRole("button", { name: "Remind me if no replyShift+H" }));
    expect(s().threads.find((t) => t.id === "p")?.section).not.toBe("later");
    await user.click(screen.getByRole("button", { name: "RemindH" }));
    expect(s().remindingId).toBe("p");
    await user.click(screen.getByRole("button", { name: /^Tomorrow/ }));
    expect(s().threads.find((t) => t.id === "p")?.section).toBe("later");
    expect(s().remindingId).toBeUndefined();
    s().open("q");
    await user.click(screen.getByRole("button", { name: "DoneE" }));
    expect(s().threads.map((t) => t.id)).not.toContain("q");
  });

  it("there is no side panel: the sender's name opens what you share and what the AI noticed", async () => {
    resetStore([thread("p", { between: { emails: 6, since: "March", youReply: "Same day" }, insight: "Prefers short replies." })]);
    s().open("p");
    render(<ThreadScreen />);
    expect(screen.queryByRole("complementary")).not.toBeInTheDocument();
    const sender = s().threads[0].messages.at(-1)!.from.name;
    await userEvent.setup().click(screen.getByRole("button", { name: sender }));
    const card = within(screen.getByRole("dialog", { name: `About ${sender}` }));
    expect(card.getByRole("heading", { name: "Between you" })).toBeInTheDocument();
    expect(card.getByText("Prefers short replies.")).toBeInTheDocument();
  });

  it("the top bar can move the email to Trash", async () => {
    resetStore([thread("p"), thread("q")]);
    s().open("p");
    render(<ThreadScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: "DeleteBackspace" }));
    expect(s().threads.map((t) => t.id)).toEqual(["q"]);
    expect(s().toast).toMatchObject({ text: "Moved to Trash" });
  });

  it("replies use the full name when there is no short one", () => {
    resetStore([thread("y", { messages: [], from: person("Sam Reed", "sam@reed.io") })]);
    s().open("y");
    render(<ThreadScreen />);
    expect(screen.getByRole("button", { name: "Reply to Sam Reed…R" })).toBeInTheDocument();
  });

});

describe("ThreadScreen: senders", () => {
  const jordan = person("Jordan Blake", "jordan@growthleads.example");
  const pat = person("Pat Kim", "pat@gmail.com");

  it("asks about a new sender on the email itself; Block reads on to the next email", async () => {
    resetStore([thread("n", { from: jordan, sender: "new", accountId: "personal" }), thread("b", { accountId: "personal" })]);
    s().open("n");
    const { rerender } = render(<ThreadScreen />);
    const banner = screen.getByRole("region", { name: "Block this sender?" });
    expect(banner).toHaveTextContent("From jordan@growthleads.example");
    await userEvent.click(within(banner).getByRole("button", { name: "BlockB" }));
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["n"]);
    expect(s().openThreadId).toBe("b");
    rerender(<ThreadScreen />);
    expect(screen.queryByRole("region", { name: "Block this sender?" })).not.toBeInTheDocument();
  });

  it("Accept and the whole domain's Block and Accept work from the banner", async () => {
    resetStore([thread("n", { from: jordan, sender: "new" }), thread("m", { from: jordan, sender: "new" })]);
    s().open("n");
    const { rerender } = render(<ThreadScreen />);
    await userEvent.click(screen.getByRole("button", { name: "AcceptA" }));
    expect(s().threads.every((t) => t.sender === "known")).toBe(true);
    useStore.setState({ threads: [thread("n", { from: jordan, sender: "new" }), thread("m", { from: jordan, sender: "new" })] });
    rerender(<ThreadScreen />);
    await userEvent.click(screen.getByRole("button", { name: "Shift+BBlock" }));
    expect(s().blockedThreads).toHaveLength(2);
    const sam = person("Sam Lee", "sam@growthleads.example");
    resetStore([thread("n", { from: jordan, sender: "new" }), thread("m", { from: sam, sender: "new" })]);
    s().open("n");
    rerender(<ThreadScreen />);
    await userEvent.click(screen.getByRole("button", { name: "Shift+AAccept" }));
    expect(s().threads.map((t) => t.sender)).toEqual(["known", "known"]);
  });

  it("never offers to block a public domain", () => {
    resetStore([thread("n", { from: pat, sender: "new" })]);
    s().open("n");
    render(<ThreadScreen />);
    expect(screen.queryByRole("button", { name: /everyone|domain/ })).not.toBeInTheDocument();
  });

  it("asks nothing about someone you know", () => {
    resetStore([thread("n", { sender: "known" })]);
    s().open("n");
    render(<ThreadScreen />);
    expect(screen.queryByRole("region", { name: /sender/ })).not.toBeInTheDocument();
  });

  it("blocked mail says where it is, and Unblock brings it back", async () => {
    resetStore([]);
    useStore.setState({ blockedThreads: [thread("x", { from: jordan, sender: "blocked", accountId: "personal" }), thread("y", { sender: "blocked" })] });
    s().openBlocked();
    s().open("x");
    render(<ThreadScreen />);
    expect(screen.getByRole("button", { name: /Blocked/ })).toBeInTheDocument();
    expect(screen.getByText("1 of 2")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "You blocked this sender" })).toHaveTextContent("Personal’s Blocked folder");
    expect(screen.queryByRole("button", { name: "DeleteBackspace" })).not.toBeInTheDocument();
    await userEvent.click(within(screen.getByRole("region", { name: "You blocked this sender" })).getByRole("button", { name: "UnblockU" }));
    expect(s().threads.map((t) => t.id)).toEqual(["x"]);
    expect(s().blockedThreads.map((t) => t.id)).toEqual(["y"]);
  });

  it("an email you forwarded says so, and opens the forward from there or from an earlier message", async () => {
    const rebecca = person("Rebecca Vale", "rebecca@vale.example", { short: "Rebecca" });
    const forward = { threadId: "fwd", to: [rebecca], time: "Saturday, 31 October, 4:12 PM" };
    resetStore([
      thread("n", { messages: [message({ id: "n-1", paragraphs: ["Older note"], forwards: [forward] }), message({ id: "n-2", forwards: [forward] })] }),
      thread("fwd", { subject: "Fwd: Hello" }),
    ]);
    s().open("n");
    const user = userEvent.setup();
    render(<ThreadScreen />);
    await user.click(screen.getByRole("button", { name: /You forwarded this to Rebecca/ }));
    expect(s().openThreadId).toBe("fwd");

    act(() => s().open("n"));
    await user.click(screen.getByRole("button", { name: /Older note/ }));
    const [earlier] = screen.getAllByRole("article");
    await user.click(within(earlier).getByRole("button", { name: /You forwarded this to Rebecca/ }));
    expect(s().openThreadId).toBe("fwd");
  });

  describe("the address it came to", () => {
    const icloud = {
      id: "icloud",
      name: "iCloud",
      letter: "I",
      address: "maya@icloud.com",
      senderName: "Maya Ross",
      color: "sky" as const,
      addresses: [{ email: "hi@rossstudio.art", state: "added" as const, source: "you" as const, seen: 1, blocked: false }],
    };

    it("shows it beside the account, and an instant reply goes from it", () => {
      resetStore([thread("x", { accountId: "icloud", toAddress: "hi@rossstudio.art", replyOptions: options })], [icloud]);
      s().open("x");
      render(<ThreadScreen />);
      expect(screen.getByText(/iCloud\s+·\s+hi@rossstudio\.art/)).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Fromhi@rossstudio.art" })).toBeInTheDocument();
      expect(screen.queryByText(/you don’t send from yet/)).not.toBeInTheDocument();
    });

    it("mail to the account's own address shows it as before", () => {
      resetStore([thread("x", { accountId: "icloud", toAddress: "maya@icloud.com" })], [icloud]);
      s().open("x");
      render(<ThreadScreen />);
      expect(screen.getByText(/iCloud\s+·\s+maya@icloud\.com/)).toBeInTheDocument();
    });

    it("an address you don't send from yet can be added, or blocked", async () => {
      resetStore([thread("x", { accountId: "icloud", toAddress: "shop@mayaross.me" }), thread("y", { accountId: "icloud" })], [icloud]);
      s().open("x");
      const { rerender } = render(<ThreadScreen />);
      expect(screen.getByText("It came to shop@mayaross.me, which you don’t send from yet.")).toBeInTheDocument();
      await userEvent.click(screen.getByRole("button", { name: "Send from it" }));
      expect(s().accounts[0].addresses?.at(-1)).toMatchObject({ email: "shop@mayaross.me", state: "added" });
      rerender(<ThreadScreen />);
      expect(screen.queryByText(/you don’t send from yet/)).not.toBeInTheDocument();

      resetStore([thread("x", { accountId: "icloud", toAddress: "shop@mayaross.me" }), thread("y", { accountId: "icloud" })], [icloud]);
      s().open("x");
      rerender(<ThreadScreen />);
      await userEvent.click(screen.getByRole("button", { name: "Block it" }));
      expect(s().blockedThreads.map((t) => t.id)).toEqual(["x"]);
    });

    it("mail blocked for its address says so on the email", () => {
      const blocked = { ...icloud, addresses: [{ email: "shop@mayaross.me", state: "dismissed" as const, source: "mail" as const, seen: 1, blocked: true }] };
      resetStore([], [blocked]);
      useStore.setState({ blockedThreads: [thread("x", { accountId: "icloud", toAddress: "shop@mayaross.me", sender: "blocked" })] });
      s().openBlocked();
      s().open("x");
      render(<ThreadScreen />);
      expect(screen.getByRole("region", { name: "You blocked mail to shop@mayaross.me" })).toBeInTheDocument();
      expect(screen.queryByText(/you don’t send from yet/)).not.toBeInTheDocument();
    });
  });
});
