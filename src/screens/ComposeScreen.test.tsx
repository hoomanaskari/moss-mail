import { act, createEvent, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { jonas, lena, maya, message, person, personal, resetStore, thread, work } from "../test/fixtures";
import { draftsSaved, keepDraft, saveDelay, watchDrafts } from "../state/drafts";
import { demoReplies, setReplies } from "../services/replies";
import { useAiSettings } from "../state/aiSettings";
import { useMossWriting } from "../state/mossWriting";
import { contactsFrom, useStore } from "../state/store";
import { useScheduled } from "../state/scheduled";
import { useSignatures } from "../state/signatures";
import type { MailBackend } from "../services/mail";
import { ComposeScreen } from "./ComposeScreen";

// Spies on the contact list, to see when the composer builds it again.
vi.mock("../state/store", async (importOriginal) => {
  const real = await importOriginal<typeof import("../state/store")>();
  return { ...real, contactsFrom: vi.fn(real.contactsFrom) };
});

const s = () => useStore.getState();
const ines = person("Ines Duarte", "ines@duarte-studio.pt", { short: "Ines" });
const tom = person("Tom Hale", "tom@hale.dev", { short: "Tom" });

beforeEach(() =>
  resetStore([thread("q4", { subject: "Q4 budget", messages: [message({ to: [jonas], cc: [ines] })] })], [
    { ...work, org: "Northwind" },
    personal,
  ]),
);

describe("ComposeScreen", () => {
  it("typing doesn't build the contact list again, new people in your mail do", async () => {
    s().compose("new");
    render(<ComposeScreen />);
    vi.mocked(contactsFrom).mockClear();
    await userEvent.setup().type(screen.getByRole("textbox", { name: "Subject" }), "Hello");
    expect(s().draft?.subject).toBe("Hello");
    expect(contactsFrom).not.toHaveBeenCalled();
    act(() => useStore.setState({ knownPeople: [person("Priya Shah", "priya@acme.co")] }));
    expect(contactsFrom).toHaveBeenCalledOnce();
  });

  it("shows nothing without a draft", () => {
    const { container } = render(<ComposeScreen />);
    expect(container).toBeEmptyDOMElement();
  });

  it("a new email has an editable subject and says no one is added yet", async () => {
    s().compose("new");
    render(<ComposeScreen />);
    expect(screen.getByText("New email")).toBeInTheDocument();
    expect(screen.getByText("No one added yet.")).toBeInTheDocument();
    await userEvent.setup().type(screen.getByRole("textbox", { name: "Subject" }), "Hi");
    expect(s().draft?.subject).toBe("Hi");
    expect(screen.getByRole("button", { name: /Back to inbox/ })).toBeInTheDocument();
  });

  it("a new email starts in Subject, so it isn't missed", () => {
    s().compose("new");
    render(<ComposeScreen />);
    expect(screen.getByRole("textbox", { name: "Subject" })).toHaveFocus();
  });

  it("suggests people Moss knows from all your mail, like someone you wrote to before", async () => {
    useStore.setState({ knownPeople: [person("Priya Shah", "priya@acme.co")] });
    s().compose("new");
    render(<ComposeScreen />);
    await userEvent.setup().type(screen.getByRole("textbox", { name: "To recipients" }), "pri");
    expect(screen.getByRole("option", { name: /Priya Shah/ })).toBeInTheDocument();
  });

  it("puts first whoever you chose last time for what you type", async () => {
    const user = userEvent.setup();
    const kindle = person("Kindle", "ross.maya@kindle.example");
    useStore.setState({ knownPeople: [kindle, person("Maya Ross", "rossmaya@home.example")] });
    s().compose("new");
    render(<ComposeScreen />);
    const to = screen.getByRole("textbox", { name: "To recipients" });
    await user.type(to, "ross");
    expect(screen.getAllByRole("option")[0]).toHaveAccessibleName(/Kindle/);
    await user.click(screen.getByRole("option", { name: /Maya Ross/ }));
    expect(s().recipientPicks).toEqual([{ typed: "ross", email: "rossmaya@home.example" }]);
    await user.type(to, "{Backspace}ro");
    expect(screen.getAllByRole("option")[0]).toHaveAccessibleName(/Maya Ross/);
  });

  it("asks before sending a new email with no subject", async () => {
    const user = userEvent.setup();
    s().compose("new");
    s().updateDraft({ to: [lena] });
    render(<ComposeScreen />);
    await user.click(screen.getByRole("button", { name: /^SendCtrl/ }));
    expect(screen.getByText("This email has no subject.")).toBeInTheDocument();
    expect(s().sending).toBeFalsy();

    await user.click(screen.getByRole("button", { name: "Add subject" }));
    expect(screen.getByRole("textbox", { name: "Subject" })).toHaveFocus();
    expect(screen.queryByText("This email has no subject.")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /^SendCtrl/ }));
    await user.click(screen.getByRole("button", { name: "Send anyway" }));
    expect(s().sending).toBe(true);
    // Let it go before the next test, which would otherwise see it land.
    await waitFor(() => expect(s().draft).toBeUndefined());
  });

  it("a reply shows its subject as a heading and goes back to the thread", async () => {
    s().compose("reply", "q4");
    render(<ComposeScreen />);
    expect(screen.getByRole("heading", { name: "Re: Q4 budget" })).toBeInTheDocument();
    expect(screen.getByText("Goes to 1 person.")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: /Back to thread/ }));
    expect(s().view).toBe("thread");
  });

  it("a forward shows the email being forwarded under your text", () => {
    s().compose("forward", "q4");
    render(<ComposeScreen />);
    const quote = screen.getByRole("region", { name: "Forwarded message" });
    expect(quote).toHaveTextContent("Q4 budget");
    expect(quote).toHaveTextContent("Hello");
  });

  it("a forwarded email's images load just this once, or from its sender from now on", async () => {
    const user = userEvent.setup();
    const sale = `<p>Sale</p><img src="https://img.example/hero.jpg" alt="">`;
    resetStore([thread("n", { messages: [message({ id: "n-1", html: sale })] })]);
    useStore.setState({ readingSize: "smaller" });
    s().compose("forward", "n");
    const { unmount } = render(<ComposeScreen />);
    expect((screen.getByTitle("Email") as HTMLIFrameElement).srcdoc).toContain('data-reading="smaller"');
    await user.click(screen.getByRole("button", { name: /Just this once/ }));
    expect(s().imagesShown).toEqual(["n-1"]);
    expect((screen.getByTitle("Email") as HTMLIFrameElement).srcdoc).toContain("https://img.example/hero.jpg");
    unmount();

    useStore.setState({ imagesShown: [] });
    render(<ComposeScreen />);
    await user.click(screen.getByRole("button", { name: /Always for this sender/ }));
    expect(s().threads[0].messages[0].imageTrust).toBe("you-allowed");
  });

  it("a reply doesn't show the forwarded-message preview", () => {
    s().compose("reply", "q4");
    render(<ComposeScreen />);
    expect(screen.queryByRole("region", { name: "Forwarded message" })).not.toBeInTheDocument();
  });

  it("a reply to a done email goes back to it", () => {
    useStore.setState({ doneThreads: [thread("old", { subject: "Old news", doneAt: 1 })] });
    s().compose("reply", "old");
    render(<ComposeScreen />);
    expect(screen.getByRole("button", { name: /Back to thread/ })).toBeInTheDocument();
  });

  it("names one outsider", () => {
    s().compose("reply-all", "q4");
    render(<ComposeScreen />);
    expect(screen.getByText("Goes to 3 people. Ines is outside Northwind.")).toBeInTheDocument();
  });

  it("counts BCC and names several outsiders", () => {
    s().compose("reply-all", "q4");
    s().updateDraft({ bcc: [tom] });
    render(<ComposeScreen />);
    expect(screen.getByText("Goes to 3 people and 1 BCC. Ines and Tom are outside Northwind.")).toBeInTheDocument();
  });

  it("uses the full name for an outsider without a short name", () => {
    s().compose("new");
    s().updateDraft({ to: [person("Sam Reed", "sam@reed.io")] });
    render(<ComposeScreen />);
    expect(screen.getByText("Goes to 1 person. Sam Reed is outside Northwind.")).toBeInTheDocument();
  });

  it("warns when the email mentions an attachment that isn’t there", () => {
    s().compose("reply", "q4");
    s().updateDraft({ body: "I’ve attached the notes." });
    render(<ComposeScreen />);
    expect(screen.getByText("You mention an attachment, but nothing is attached.")).toBeInTheDocument();
  });

  it("toggles the no-reply reminder", async () => {
    s().compose("reply", "q4");
    render(<ComposeScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: /Remind me if no reply/ }));
    expect(s().draft?.remindIfNoReply).toBe(true);
  });

  it("discard deletes the draft and goes back", async () => {
    s().compose("reply", "q4");
    s().updateDraft({ body: "Draft", bodyHtml: "<p>Draft</p>" });
    render(<ComposeScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: /Discard/ }));
    expect(s().view).toBe("thread");
    expect(s().draft).toBeUndefined();
  });

  describe("saving the draft", () => {
    let stop = () => {};
    beforeEach(() => {
      vi.useFakeTimers();
      stop = watchDrafts(useStore);
    });
    afterEach(() => {
      stop();
      vi.useRealTimers();
    });

    const saved = async () => {
      await act(() => vi.advanceTimersByTimeAsync(saveDelay));
      await act(draftsSaved);
    };

    it("says nothing while the draft is empty, and Draft saved once it really is", async () => {
      s().compose("new");
      render(<ComposeScreen />);
      const status = screen.getByRole("status");
      await saved();
      expect(status).toHaveTextContent(/^$/);
      act(() => s().updateDraft({ subject: "Lunch" }));
      expect(status).toHaveTextContent(/^$/);
      await saved();
      expect(status).toHaveTextContent("Draft saved");
      act(() => s().updateDraft({ subject: "" }));
      await saved();
      expect(status).toHaveTextContent(/^$/);
    });

    it("says when the draft couldn’t be saved", async () => {
      vi.spyOn(console, "warn").mockImplementation(() => {});
      fakeMail({ saveDraft: vi.fn(async () => Promise.reject(new Error("disk full"))) });
      s().compose("reply", "q4");
      render(<ComposeScreen />);
      act(() => s().updateDraft({ body: "Yes" }));
      await saved();
      expect(screen.getByRole("status")).toHaveTextContent("Couldn’t save draft");
    });
  });

  it("Send shows it is sending at once, then hands the draft over and goes back to the email, still in the inbox", async () => {
    s().compose("reply", "q4");
    render(<ComposeScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: /^SendCtrl/ }));
    expect(screen.getByRole("button", { name: /^Sending…/ })).toHaveAttribute("aria-busy", "true");
    await waitFor(() => expect(s()).toMatchObject({ draft: undefined, view: "thread", openThreadId: "q4", handled: 0 }));
    expect(s().threads.map((t) => t.id)).toContain("q4");
  });

  it("says it is sending, and why a send was refused", async () => {
    let fail: (e: Error) => void = () => {};
    fakeMail({ sendAfter: vi.fn(() => new Promise<boolean>((_, reject) => (fail = reject))) });
    s().compose("reply", "q4");
    render(<ComposeScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: /^SendCtrl/ }));
    expect(screen.getByRole("status")).toHaveTextContent("Sending…");
    await act(async () => fail(new Error("can’t reach smtp.gmail.com")));
    expect(screen.getByRole("alert")).toHaveTextContent("Not sent: can’t reach smtp.gmail.com. Your email is still here");
    expect(s().draft).toBeDefined();
  });

  it("Send later asks when", async () => {
    useScheduled.setState({ picking: undefined, editing: undefined });
    s().compose("reply", "q4");
    render(<ComposeScreen />);
    await userEvent.click(screen.getByRole("button", { name: /^Send later/ }));
    expect(useScheduled.getState().picking).toEqual({ kind: "compose" });
    useScheduled.setState({ picking: undefined });
  });

  it("an email opened from Scheduled says it won’t go until you send or schedule it again", () => {
    s().compose("new");
    const id = s().draft!.id;
    useScheduled.setState({ editing: { id, sendAt: new Date(2026, 10, 7, 8, 0).getTime() } });
    const { unmount } = render(<ComposeScreen />);
    const note = screen.getByText(/^Was set to send .*8 AM\. It won’t go until you send it or pick a time again\.$/);
    // Under the subject, where you look first, not down by Send.
    const send = screen.getByRole("button", { name: /^Send(⌘|Ctrl)/ });
    expect(note.compareDocumentPosition(send) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(note.compareDocumentPosition(screen.getAllByLabelText(/recipients$/)[0]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    unmount();
    useScheduled.setState({ editing: { id: "another", sendAt: 1 } });
    render(<ComposeScreen />);
    expect(screen.queryByText(/Was set to send/)).not.toBeInTheDocument();
    useScheduled.setState({ editing: undefined });
  });

  it("CC and BCC buttons show their rows", async () => {
    s().compose("new");
    render(<ComposeScreen />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /^CC/ }));
    await user.click(screen.getByRole("button", { name: /^BCC/ }));
    expect(s().draft).toMatchObject({ showCc: true, showBcc: true });
  });

  it("adding a recipient updates the draft", async () => {
    s().compose("new");
    render(<ComposeScreen />);
    const user = userEvent.setup();
    await user.type(screen.getByRole("textbox", { name: "To recipients" }), "lena@northwind.co{Enter}");
    expect(s().draft?.to.map((p) => p.email)).toContain(lena.email);
  });

  it("an address typed into To, without picking a contact, is sent to", async () => {
    const send = vi.fn(async () => true);
    fakeMail({ sendAfter: send });
    s().compose("new");
    s().updateDraft({ subject: "Hello" });
    render(<ComposeScreen />);
    const user = userEvent.setup();
    await user.type(screen.getByRole("textbox", { name: "To recipients" }), "sam@harbour.studio");
    await user.click(screen.getByRole("button", { name: /^SendCtrl/ }));
    expect(send).toHaveBeenCalledWith(expect.any(String), expect.objectContaining({ to: [expect.objectContaining({ email: "sam@harbour.studio" })] }), expect.any(Number));
  });

  it("collapsed fields open again when clicked", async () => {
    s().compose("reply", "q4");
    s().updateDraft({ fieldsOpen: false });
    render(<ComposeScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: /^EditCtrl\+Shift\+E/ }));
    expect(s().draft?.fieldsOpen).toBe(true);
  });

  it("typing in the editor saves the body", async () => {
    s().compose("reply", "q4");
    render(<ComposeScreen />);
    const editor = await waitFor(() => {
      const el = document.querySelector<HTMLElement>("[contenteditable=true]");
      if (!el) throw new Error("editor not ready");
      return el;
    });
    await userEvent.setup().type(editor, "Thanks");
    await waitFor(() => expect(s().draft?.body).toContain("Thanks"));
  });

  it("adds the account’s signature under the body and says why", () => {
    s().compose("new");
    render(<ComposeScreen />);
    expect(screen.getByRole("button", { name: "Change signature, Work (full)" })).toBeInTheDocument();
    expect(screen.getByText("Your signature for maya@northwind.co")).toBeInTheDocument();
    expect(screen.getByText("Design lead, Northwind")).toBeInTheDocument();
  });

  it("says there's no signature when the account has none, and Add opens the picker", async () => {
    s().compose("new");
    s().updateDraft({ fromAccountId: "personal" });
    render(<ComposeScreen />);
    expect(screen.getByRole("group", { name: "Signature on this email" })).toHaveTextContent(
      "No signatureAdd signatureAlt+SNone set for maya.ross@fastmail.com",
    );
    expect(screen.queryByRole("group", { name: "Signature" })).not.toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Add signatureAlt+S" }));
    expect(s().draft?.signatureOpen).toBe(true);
  });

  it("Remove takes the signature off this email, and the row says so", async () => {
    s().compose("new");
    const { rerender } = render(<ComposeScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Remove signature" }));
    expect(s().draft).toMatchObject({ signatureId: null, signatureChosen: true });
    rerender(<ComposeScreen />);
    expect(screen.getByRole("group", { name: "Signature on this email" })).toHaveTextContent("No signatureAdd signatureAlt+SChosen for this email");
  });

  it("a draft with none, though Moss would add one now, offers one without saying why it has none", async () => {
    s().compose("reply", "q4");
    // As a draft saved before you set up a reply signature opens. Ones from the server get theirs as they come in.
    s().updateDraft({ signatureId: null });
    const { rerender } = render(<ComposeScreen />);
    expect(screen.getByRole("group", { name: "Signature on this email" })).toHaveTextContent(/^No signatureAdd signatureAlt\+S$/);
    await userEvent.setup().click(screen.getByRole("button", { name: "Add signatureAlt+S" }));
    rerender(<ComposeScreen />);
    expect(screen.queryByText(/came without/)).not.toBeInTheDocument();
    expect(screen.getByText("Work (short)")).toBeInTheDocument();
  });

  it("with no saved signatures, the row leads to Settings, Signatures", async () => {
    useSignatures.setState({ signatures: [], recent: [] });
    s().compose("new");
    render(<ComposeScreen />);
    expect(screen.getByRole("group", { name: "Signature on this email" })).toHaveTextContent("No signature");
    await userEvent.setup().click(screen.getByRole("button", { name: "Set up a signature" }));
    expect(s()).toMatchObject({ view: "settings", settingsSection: "signatures" });
  });

  it("the picker offers the suggested signature first, then recent ones, and remembers a pick", async () => {
    s().compose("new");
    s().toggleSignaturePicker();
    const { rerender } = render(<ComposeScreen />);
    const user = userEvent.setup();
    expect(screen.getAllByRole("radio").map((r) => r.closest("label")!.textContent)).toEqual([
      "Work (full)Maya Ross · Design lead, Northwind1",
      "Work (short)Maya2",
      "StudioMaya Ross · Ross Studio3",
      "NoneNo signature on this email0",
    ]);
    expect(screen.getByText("Picked for new emails from maya@northwind.co.")).toBeInTheDocument();
    await user.click(screen.getByRole("radio", { name: /^Work \(short\)/ }));
    expect(s().draft).toMatchObject({ signatureId: "work-short", signatureOpen: false });
    rerender(<ComposeScreen />);
    expect(screen.getByText("Chosen for this email")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Change signature, Work (short)" }));
    expect(screen.getByText(/You chose this one/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^Edit signatures/ }));
    expect(s()).toMatchObject({ view: "settings", settingsSection: "signatures" });
  });

  it("explains an account with no signature when the picker is opened", () => {
    s().compose("new");
    s().updateDraft({ fromAccountId: "personal", signatureOpen: true });
    render(<ComposeScreen />);
    expect(screen.getByRole("radio", { name: /^None/ })).toBeChecked();
    expect(screen.getByText(/No signature set for maya.ross@fastmail.com here/)).toBeInTheDocument();
  });

  it("in a back-and-forth says why it left the signature out, or picked a short one", () => {
    resetStore([thread("q4", { messages: [message(), message({ id: "q4-2", from: maya })] })], [work, { ...personal, id: "studio" }]);
    s().compose("reply", "q4");
    // Studio's is a full one.
    s().updateDraft({ fromAccountId: "studio" });
    const { rerender } = render(<ComposeScreen />);
    expect(screen.getByRole("button", { name: "Add signatureAlt+S" })).toBeInTheDocument();
    expect(screen.getByText("Left out: quick reply in a back-and-forth")).toBeInTheDocument();
    s().updateDraft({ fromAccountId: "work" });
    rerender(<ComposeScreen />);
    expect(screen.getByRole("button", { name: "Change signature, Work (short)" })).toBeInTheDocument();
    expect(screen.getByText("Short one: you signed earlier in this thread")).toBeInTheDocument();
  });

  it("a signature the draft came with, not Moss's pick, gets no reason", () => {
    s().compose("new");
    s().updateDraft({ signatureId: "studio" });
    render(<ComposeScreen />);
    expect(screen.getByRole("button", { name: "Change signature, Studio" })).toBeInTheDocument();
    expect(screen.queryByText(/Your signature for/)).not.toBeInTheDocument();
  });

  it("the From menu moves the email to another of your addresses, and says why it picked the first", async () => {
    const withStudio = { ...personal, addresses: [{ email: "hi@rossstudio.art", state: "added" as const, source: "you" as const, seen: 0, blocked: false }] };
    resetStore([thread("q4", { accountId: "personal", toAddress: "hi@rossstudio.art" })], [work, withStudio]);
    s().compose("reply", "q4");
    const { rerender } = render(<ComposeScreen />);
    const user = userEvent.setup();
    expect(screen.getByText("It came to hi@rossstudio.art")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Maya Ross\s+·\s+hi@rossstudio\.art/ }));
    expect(s().draft?.fromOpen).toBe(true);
    rerender(<ComposeScreen />);
    await user.click(screen.getByRole("radio", { name: /maya\.ross@fastmail\.com/ }));
    expect(s().draft).toMatchObject({ fromAccountId: "personal", fromAddress: undefined, fromOpen: false });
    rerender(<ComposeScreen />);
    expect(screen.queryByText("It came to hi@rossstudio.art")).not.toBeInTheDocument();
    s().toggleFromPicker();
    rerender(<ComposeScreen />);
    await user.click(screen.getByRole("button", { name: /Add an address/ }));
    expect(s()).toMatchObject({ view: "settings", settingsSection: "accounts" });
  });
});

describe("Attaching files", () => {
  const plan = { id: "1", name: "Plan.pdf", bytes: 2048, contentType: "application/pdf" };
  const notes = { id: "2", name: "notes.txt", bytes: 2, contentType: "text/plain" };

  it("Attach, and the warning's Attach file, open the file picker", async () => {
    const { backend } = fakeMail();
    s().compose("reply", "q4");
    s().updateDraft({ body: "see attachment" });
    render(<ComposeScreen />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /^AttachCtrl\+Shift\+A/ }));
    await user.click(screen.getByRole("button", { name: "Attach file" }));
    expect(backend.pickFiles).toHaveBeenCalledTimes(2);
  });

  it("attached files show their name, kind and size, with a thumbnail, and come off by mouse or ⌫", async () => {
    fakeMail();
    s().compose("new");
    s().updateDraft({ attachments: [plan, notes] });
    useStore.setState({ previews: { [`draft:${s().draft!.id}:1`]: "data:image/jpeg;base64,AA==" } });
    const { container } = render(<ComposeScreen />);
    const files = screen.getByRole("group", { name: "Attached files" });
    expect(within(files).getByRole("button", { name: "Plan.pdf, PDF document, 2 KB" })).toBeInTheDocument();
    expect(container.querySelector("img[src='data:image/jpeg;base64,AA==']")).toBeInTheDocument();
    const user = userEvent.setup();
    await user.click(within(files).getByRole("button", { name: "Remove notes.txt" }));
    expect(s().draft!.attachments).toEqual([plan]);
    within(files).getByRole("button", { name: /^Plan\.pdf/ }).focus();
    await user.keyboard("{Backspace}");
    expect(s().draft!.attachments).toEqual([]);
    expect(screen.queryByRole("group", { name: "Attached files" })).not.toBeInTheDocument();
  });

  it("the missing-attachment warning goes once something is attached", () => {
    s().compose("reply", "q4");
    s().updateDraft({ body: "I’ve attached the notes.", attachments: [notes] });
    render(<ComposeScreen />);
    expect(screen.queryByText("You mention an attachment, but nothing is attached.")).not.toBeInTheDocument();
  });

  it("says before Send when the files are too big for the account", () => {
    resetStore([thread("q4")], [{ ...work, connection: { provider: "gmail", summary: "Gmail" } }]);
    s().compose("new");
    s().updateDraft({ attachments: [{ ...plan, bytes: 26 * 1024 * 1024 }] });
    render(<ComposeScreen />);
    expect(screen.getByText("These files are 26 MB. Gmail sends up to 25 MB.")).toBeInTheDocument();
  });

  it("says why a file couldn't be attached", () => {
    s().compose("new");
    useStore.setState({ attachProblem: "Couldn’t attach: can’t read gone.pdf" });
    render(<ComposeScreen />);
    expect(screen.getByText("Couldn’t attach: can’t read gone.pdf")).toBeInTheDocument();
  });

  it("files dragged over the composer can be dropped to attach them", async () => {
    const { backend } = fakeMail();
    s().compose("new");
    render(<ComposeScreen />);
    const zone = screen.getByTestId("compose-drop");
    const file = new File(["hi"], "notes.txt");
    const dataTransfer = { types: ["Files"], files: [file] };
    fireEvent.dragEnter(zone, { dataTransfer });
    fireEvent.dragOver(zone, { dataTransfer });
    expect(screen.getByText("Drop to attach")).toBeInTheDocument();
    fireEvent.dragLeave(zone, { dataTransfer, relatedTarget: null });
    expect(screen.queryByText("Drop to attach")).not.toBeInTheDocument();
    fireEvent.dragOver(zone, { dataTransfer });
    fireEvent.drop(zone, { dataTransfer });
    expect(screen.queryByText("Drop to attach")).not.toBeInTheDocument();
    await waitFor(() => expect(backend.attachFiles).toHaveBeenCalledWith(s().draft!.id, [file]));
    // Text dragged around inside the email isn't a file.
    fireEvent.dragOver(zone, { dataTransfer: { types: ["text/plain"], files: [] } });
    expect(screen.queryByText("Drop to attach")).not.toBeInTheDocument();
    fireEvent.drop(zone, { dataTransfer: { types: ["text/plain"], files: [] } });
    // Files with nothing in the list (a folder, say) add nothing.
    fireEvent.drop(zone, { dataTransfer: { types: ["Files"], files: [] } });
    await act(async () => {});
    expect(backend.attachFiles).toHaveBeenCalledTimes(1);
    // Onto something inside the composer isn't leaving it.
    fireEvent.dragEnter(zone, { dataTransfer });
    // jsdom's drag events drop relatedTarget, so set it on the event itself.
    const inside = createEvent.dragLeave(zone, { dataTransfer });
    Object.defineProperty(inside, "relatedTarget", { value: screen.getByText("New email") });
    fireEvent(zone, inside);
    expect(screen.getByText("Drop to attach")).toBeInTheDocument();
  });

  it("a forwarded file whose email is gone shows no size", () => {
    s().compose("new");
    s().updateDraft({ attachments: [{ id: "x:1", name: "Q4.pdf", bytes: 0, contentType: "", forwarded: { messageId: "x", index: 1 } }] });
    render(<ComposeScreen />);
    expect(screen.getByRole("button", { name: /^Q4\.pdf, PDF document,$/ })).toBeInTheDocument();
  });

  it("a forward lists the files it carries, which can come off", async () => {
    resetStore([
      thread("q4", { subject: "Q4 budget", messages: [message({ id: "m1", attachments: [{ index: 2, name: "Q4.pdf", size: "2.0 MB", bytes: 2_097_152 }, { name: "logo.png", size: "3 KB" }] })] }),
    ]);
    s().compose("forward", "q4");
    render(<ComposeScreen />);
    const files = screen.getByRole("group", { name: "Attached files" });
    expect(within(files).getByRole("button", { name: "Q4.pdf, PDF document, 2.0 MB" })).toBeInTheDocument();
    // Demo mail's files have no byte count: the size they show stands in.
    expect(within(files).getByRole("button", { name: "logo.png, image, 3 KB" })).toBeInTheDocument();
    await userEvent.setup().click(within(files).getByRole("button", { name: "Remove Q4.pdf" }));
    expect(s().draft!.attachments!.map((f) => f.name)).toEqual(["logo.png"]);
  });
});

describe("Write with Moss", () => {
  const m = () => useMossWriting.getState();
  beforeEach(() => {
    setReplies(demoReplies());
    m().reset();
    s().open("q4");
    s().compose("reply");
    s().updateDraft({ body: "Hi Lena,\n\nYes." });
  });

  it("the toolbar button opens the menu, named for who you're writing to", async () => {
    render(<ComposeScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: /^Write with Moss/ }));
    expect(screen.getByRole("dialog", { name: "Write with Moss" })).toHaveTextContent("Accept what Lena asks");
  });

  it("someone without a short name goes by their first name", () => {
    s().updateDraft({ to: [person("Sam Reed", "sam@reed.co")] });
    m().openMenu();
    render(<ComposeScreen />);
    expect(screen.getByRole("dialog")).toHaveTextContent("Accept what Sam asks");
  });

  it("a new email with no one in it yet asks about 'they'", () => {
    s().compose("new");
    m().openMenu();
    render(<ComposeScreen />);
    expect(screen.getByRole("dialog")).toHaveTextContent("Accept what they ask");
    expect(screen.getByRole("option", { name: /^Shorter/ })).toHaveAttribute("aria-disabled", "true");
  });

  it("writes from your note: the strip says so, the text lands, Undo puts yours back", async () => {
    m().openMenu();
    render(<ComposeScreen />);
    const user = userEvent.setup();
    await user.keyboard("say Friday works{Enter}");
    await waitFor(() => expect(screen.getByText("Moss wrote this from your note")).toBeInTheDocument());
    expect(screen.getByText("“say Friday works”")).toBeInTheDocument();
    expect(s().draft?.body).toBe("Hi,\n\nThanks for this. Say Friday works.");
    expect(await screen.findByText(/Thanks for this\. Say Friday works\./)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "UndoCtrl+Z" }));
    expect(s().draft?.body).toBe("Hi Lena,\n\nYes.");
    expect(screen.queryByText("Moss wrote this from your note")).not.toBeInTheDocument();
  });

  it("a menu action's strip names it; typing in the text puts the strip away", async () => {
    await m().write({ job: { kind: "shorter" }, lead: "Moss made it shorter" });
    render(<ComposeScreen />);
    expect(screen.getByText("Moss made it shorter")).toBeInTheDocument();
    const editor = await screen.findByRole("textbox", { name: "Email body" });
    await userEvent.setup().type(editor, "!");
    expect(screen.queryByText("Moss made it shorter")).not.toBeInTheDocument();
  });

  it("Change it on the strip opens the menu again", async () => {
    await m().write({ job: { kind: "shorter" }, lead: "Moss made it shorter" });
    render(<ComposeScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Change itCtrl+J" }));
    expect(screen.getByRole("dialog", { name: "Write with Moss" })).toBeInTheDocument();
  });

  it("while writing, the text streams in with Stop; your draft stays until it's done", async () => {
    setReplies(demoReplies({ state: "writing" }));
    const writing = m().write({ job: { kind: "accept" }, lead: "Moss wrote a yes" });
    render(<ComposeScreen />);
    expect(await screen.findByText("Hi,\n\nYes, happy to.", { normalizer: (t) => t })).toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: "Email body" })).not.toBeInTheDocument();
    expect(s().draft?.body).toBe("Hi Lena,\n\nYes.");
    await userEvent.setup().click(screen.getByRole("button", { name: "StopEsc" }));
    await act(() => writing);
    expect(screen.getByText("Moss wrote a yes")).toBeInTheDocument();
  });

  it("without the model or after a failure, the strip says so and offers the way on", async () => {
    setReplies(demoReplies({ state: "missing" }));
    await m().write({ job: { kind: "accept" }, lead: "Moss wrote a yes" });
    const { unmount } = render(<ComposeScreen />);
    expect(screen.getByText("Writing needs the writing model")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "AI settingsCtrl+," }));
    expect(s().view).toBe("settings");
    unmount();
    // Another draft doesn't show it.
    s().compose("new");
    render(<ComposeScreen />);
    expect(screen.queryByText("Writing needs the writing model")).not.toBeInTheDocument();
  });

  it("Try again after a failure writes it", async () => {
    setReplies(demoReplies({ state: "failed" }));
    await m().write({ job: { kind: "accept" }, lead: "Moss wrote a yes" });
    render(<ComposeScreen />);
    expect(screen.getByText("Couldn’t write this one")).toBeInTheDocument();
    setReplies(demoReplies());
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again↵" }));
    await waitFor(() => expect(screen.getByText("Moss wrote a yes")).toBeInTheDocument());
  });

  it("Download starts getting the model", async () => {
    const download = vi.fn(async () => {});
    useAiSettings.setState({ download });
    setReplies(demoReplies({ state: "missing" }));
    await m().write({ job: { kind: "accept" }, lead: "Moss wrote a yes" });
    render(<ComposeScreen />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Download↵" }));
    expect(download).toHaveBeenCalledOnce();
  });
});

it("offers recovery review from a composer whose local edit conflicted", async () => {
  fakeMail({ saveDraft: vi.fn(async () => { throw { kind: "draftConflict" }; }),
    keepLocalDraftRecovery: vi.fn<MailBackend["keepLocalDraftRecovery"]>(async (id, _previous, draft) => ({ status: "saved", recovery: { id, version: "v1", draft, missingFiles: [], updatedAt: 1 } })) });
  const stop = watchDrafts(useStore);
  try {
    s().compose("new");s().updateDraft({ body: "Conflicting local work" });
    keepDraft(s().draft!);await draftsSaved();
    render(<ComposeScreen />);
    expect(screen.getByText(/Your local changes are kept separately/)).toBeVisible();
    await userEvent.setup().click(screen.getByRole("button", { name: "Review in Drafts" }));
    expect(s().view).toBe("drafts");
  } finally { stop(); }
});
