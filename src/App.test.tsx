import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { scenes } from "./dev/scenes";
import { resetAskPanel, useAskPanel } from "./state/askPanel";
import { openScheduled, useScheduled } from "./state/scheduled";
import { start } from "./state/startup";
import { useStore } from "./state/store";
import { resetDemoStore } from "./test/fixtures";
import { renderCounted } from "./test/renders";

vi.mock("./lib/window", () => ({ closeWindow: vi.fn() }));

const s = () => useStore.getState();

beforeEach(() => resetDemoStore());
afterEach(() => history.replaceState(null, "", "/"));

/** Opens the app at a dev scene, as `?scene=` does in the browser. */
async function renderScene(scene: string, theme?: string) {
  history.replaceState(null, "", `/?scene=${scene}${theme ? `&theme=${theme}` : ""}`);
  render(<App />);
  // Scenes load lazily after the first render.
  await act(() => import("./dev/scenes"));
}

// Every state drawn in design.pen must render without falling into the error screen.
describe("design scenes", () => {
  it.each(Object.entries(scenes).map(([name, { shows }]) => [name, shows] as const))("%s", async (scene, text) => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    await renderScene(scene, "light");
    await act(() => vi.advanceTimersByTimeAsync(200));
    vi.useRealTimers();
    expect(screen.queryByText(/Something went wrong on this screen/)).not.toBeInTheDocument();
    expect((await screen.findAllByText(text)).length).toBeGreaterThan(0);
    expect(s().theme).toBe("light");
  });

  it("an unknown scene leaves the app as it is", async () => {
    await renderScene("nope");
    expect(screen.getByText(/^Inbox$/)).toBeInTheDocument();
  });
});

describe("App: mail that couldn't be loaded", () => {
  it("says why in place of the mailbox, with no mail behind it, and shows the mail once Try again works", async () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    let works = false;
    await start(async () => {
      if (!works) throw new Error("database is locked");
    });
    render(<App />);
    expect(screen.getByText("Moss couldn’t open your mail")).toBeInTheDocument();
    expect(screen.getByText("database is locked")).toBeInTheDocument();
    expect(screen.queryByText(/^Inbox$/)).not.toBeInTheDocument();
    expect(screen.queryByText(s().threads[0].subject)).not.toBeInTheDocument();
    works = true;
    await userEvent.setup().click(screen.getByRole("button", { name: /Try again/ }));
    expect(await screen.findByText(/^Inbox$/)).toBeInTheDocument();
    vi.restoreAllMocks();
  });
});

describe("App", () => {
  it("⌘J opens Ask Moss beside the inbox, and answers stream into it; not in Settings", async () => {
    resetAskPanel();
    render(<App />);
    const user = userEvent.setup();
    await user.keyboard("{Control>}j{/Control}");
    const panel = screen.getByRole("complementary", { name: "Ask Moss" });
    expect(screen.getByRole("main").nextElementSibling).toBe(panel);
    await user.type(screen.getByRole("textbox", { name: "Ask about any email…" }), "Did Dana send it?{Enter}");
    expect(await screen.findByText(/Dana confirmed the Q4 budget/)).toBeInTheDocument();
    act(() => s().openSettings());
    expect(screen.queryByRole("complementary", { name: "Ask Moss" })).not.toBeInTheDocument();
  });

  it("the top bar is lined only while Ask Moss is open beside it", async () => {
    resetAskPanel();
    render(<App />);
    const frame = () => screen.getByRole("main").parentElement!.parentElement!;
    expect(frame()).not.toHaveAttribute("data-side");
    act(() => useAskPanel.getState().show());
    expect(frame()).toHaveAttribute("data-side");
    act(() => useAskPanel.getState().hide());
    expect(frame()).not.toHaveAttribute("data-side");
  });

  it("falls back to the inbox when the open email is gone", async () => {
    useStore.setState({ view: "thread", openThreadId: "gone" });
    render(<App />);
    await waitFor(() => expect(s().view).toBe("inbox"));
  });

  it("D shows the Done list; E there brings an email back, with a note to undo it", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.keyboard("d");
    expect(screen.getByRole("heading", { name: "Done" })).toBeInTheDocument();
    await user.keyboard("e");
    expect(screen.getByRole("status")).toHaveTextContent("Back in your inbox");
    await user.keyboard("{Escape}");
    expect(screen.getByRole("heading", { name: "Inbox" })).toBeInTheDocument();
  });

  it("names the window at inbox zero", async () => {
    useStore.setState({ view: "zero" });
    render(<App />);
    await waitFor(() => expect(document.title).toBe("Moss · Inbox zero"));
    act(() => useStore.setState({ view: "inbox" }));
    await waitFor(() => expect(document.title).toBe("Moss"));
  });

  it("opens and closes the command palette from the keyboard", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.keyboard("{Meta>}k{/Meta}");
    expect(await screen.findByRole("dialog")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  });

  it("H in the list asks when to remind you, and ↵ snoozes until tomorrow", async () => {
    const user = userEvent.setup();
    render(<App />);
    const id = s().cursorId!;
    await user.keyboard("h");
    const dialog = screen.getByRole("dialog", { name: "Remind me" });
    expect(dialog).toHaveTextContent(s().threads.find((t) => t.id === id)!.subject);
    await user.keyboard("{Enter}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(s().threads.find((t) => t.id === id)).toMatchObject({ section: "later", time: "Tomorrow" });
    await user.keyboard("h{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(s().remindingId).toBeUndefined();
  });

  it("⌘⇧↵ in the composer asks when to send it, and ↵ sends it tomorrow at 8 AM", async () => {
    const user = userEvent.setup();
    render(<App />);
    act(() => s().compose("new"));
    act(() => s().updateDraft({ to: [s().threads[0].from], subject: "Lunch?" }));
    await user.keyboard("{Meta>}{Shift>}{Enter}{/Shift}{/Meta}");
    const dialog = screen.getByRole("dialog", { name: "Send later" });
    expect(dialog).toHaveTextContent("Lunch?");
    await user.keyboard("{Enter}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(useScheduled.getState().emails).toMatchObject([{ draft: { subject: "Lunch?" } }]);
    expect(s().toast?.text).toMatch(/^Scheduled for .*8 AM$/);

    // Its row in Scheduled can get a new time; Esc closes the picker.
    act(() => openScheduled());
    expect(screen.getByRole("heading", { name: "Scheduled" })).toBeInTheDocument();
    await user.keyboard("h");
    expect(screen.getByRole("dialog", { name: "Send later" })).toHaveTextContent("Lunch?");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(s().view).toBe("scheduled");
    useScheduled.setState({ emails: [] });
  });

  it("H in an open email drops the menu from the Remind button instead", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.keyboard("{Enter}h");
    expect(screen.getByRole("button", { name: "RemindH" })).toHaveAttribute("aria-expanded", "true");
    expect(screen.queryByRole("dialog", { name: "Remind me" })).not.toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(s()).toMatchObject({ remindingId: undefined, view: "thread" });
  });

  it("M in the list asks which tab, and ↵ moves it to Important for good", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.keyboard("{Tab}");
    const id = s().cursorId!;
    const { subject, split } = s().threads.find((t) => t.id === id)!;
    expect(split).not.toBe("important");
    await user.keyboard("m");
    const dialog = screen.getByRole("dialog", { name: "Move to" });
    expect(dialog).toHaveTextContent(subject);
    await user.keyboard("{Enter}");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(s().threads.find((t) => t.id === id)).toMatchObject({ split: "important", movedByYou: true });
    expect(screen.getByText("Moved to Important")).toBeInTheDocument();
    await user.keyboard("m{Escape}");
    expect(s().movingId).toBeUndefined();
  });

  it("moving your own email to Important asks when to bring it back if no one replies, even while reading it", async () => {
    const user = userEvent.setup();
    render(<App />);
    const id = s().cursorId!;
    useStore.setState((st) => ({ threads: st.threads.map((t) => (t.id === id ? { ...t, youWroteLast: true } : t)) }));
    await user.keyboard("{Enter}m2");
    expect(s().threads.find((t) => t.id === id)?.split).toBe("other");
    await user.keyboard("m1");
    expect(screen.getByRole("dialog", { name: "Remind me if no reply" })).toBeInTheDocument();
    await user.keyboard("{Enter}");
    expect(s().threads.find((t) => t.id === id)).toMatchObject({ section: "later", time: "Tomorrow" });
    expect(s().toast?.text).toMatch(/if no one replies$/);
  });

  it("recovers from a broken screen back to the inbox", async () => {
    const quiet = vi.spyOn(console, "error").mockImplementation(() => {});
    const user = userEvent.setup();
    // An email with no sender breaks the thread screen.
    useStore.setState({
      view: "thread",
      openThreadId: "q4-budget",
      threads: s().threads.map((t) => (t.id === "q4-budget" ? { ...t, from: null as never } : t)),
    });
    render(<App />);
    expect(await screen.findByText(/Something went wrong/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Back to inbox/ }));
    expect(s().view).toBe("inbox");
    quiet.mockRestore();
  });

  it("keeps its shortcuts after you click into an email: keys pressed inside it still reach the app", async () => {
    useStore.setState({
      threads: s().threads.map((t) =>
        t.id === "q4-budget" ? { ...t, messages: t.messages.map((m) => ({ ...m, html: "<p>Numbers attached.</p>" })) } : t,
      ),
    });
    s().open("q4-budget");
    render(<App />);
    const frame = (await screen.findAllByTitle("Email")).at(-1) as HTMLIFrameElement;
    const doc = frame.contentDocument!;
    doc.open();
    doc.write(frame.srcdoc);
    doc.close();
    fireEvent.load(frame);
    const press = (key: string) =>
      act(() => {
        doc.body.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
      });
    press("i");
    expect(s().recipientsOpen).toBe(true);
    press("Escape");
    expect(s().recipientsOpen).toBe(false);
    press("Escape");
    expect(s().view).toBe("inbox");
  });

  it("keeps an email you leave half-written in Drafts, where it opens again", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.keyboard("c");
    await user.type(await screen.findByRole("textbox", { name: "Subject" }), "Lunch on Friday?");
    await user.keyboard("{Escape}");
    expect(await screen.findByRole("button", { name: "Drafts1" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Drafts/ }));
    expect(screen.getByRole("heading", { name: "Drafts" })).toBeInTheDocument();
    await user.keyboard("{Enter}");
    expect(screen.getByRole("textbox", { name: "Subject" })).toHaveValue("Lunch on Friday?");
  });

  it("a new email closed empty leaves no draft, and the inbox shows no Drafts link", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.keyboard("c");
    await screen.findByRole("textbox", { name: "Subject" });
    await user.keyboard("{Escape}");
    expect(screen.getByRole("heading", { name: "Inbox" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Drafts/ })).not.toBeInTheDocument();
  });

  it("the inbox footer shows C for a new email, and C starts one", async () => {
    render(<App />);
    expect(screen.getByRole("contentinfo")).toHaveTextContent("CNew email");
    await userEvent.keyboard("c");
    expect(s().view).toBe("compose");
  });

  it("goes from the inbox to an email and back with the keyboard", async () => {
    const user = userEvent.setup();
    render(<App />);
    await user.keyboard("{Enter}");
    expect(s().view).toBe("thread");
    expect(screen.getAllByText(/of \d+/).length).toBeGreaterThan(0);
    await user.keyboard("r");
    expect(await screen.findByText("Reply")).toBeInTheDocument();
    await user.keyboard("{Escape}{Escape}");
    expect(s().view).toBe("inbox");
  });
});

it("typing in a draft doesn't redraw the whole app, only what shows the draft", async () => {
  act(() => s().compose("new"));
  act(() => useStore.setState({ view: "inbox" }));
  const { renders } = renderCounted(<App />);
  await act(() => import("./dev/scenes"));
  renders.mockClear();
  act(() => useStore.setState((x) => ({ draft: x.draft && { ...x.draft, body: "Hello", bodyHtml: "Hello" } })));
  expect(renders).not.toHaveBeenCalled();
});

it("ignores development scene URLs in production",async()=>{
 vi.stubEnv("DEV",false);
 try {
  await renderScene("quit-unsaved-drafts");
  expect(screen.queryByRole("dialog",{name:"Drafts are not confirmed saved"})).not.toBeInTheDocument();
  expect(screen.getByText(/^Inbox$/)).toBeInTheDocument();
 } finally {vi.unstubAllEnvs();}
});
