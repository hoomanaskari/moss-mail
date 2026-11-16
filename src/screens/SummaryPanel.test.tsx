import { act, render, renderHook, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { demoAi, setAi } from "../services/ai";
import { demoSummaries, demoView, setSummaries, type DemoSummaryStart, type SummaryBackend, type SummaryView } from "../services/summaries";
import { useAiSettings } from "../state/aiSettings";
import { useStore } from "../state/store";
import { useSummary } from "../state/summary";
import { useShortcuts } from "../state/useShortcuts";
import { lena, message, person, resetStore, thread } from "../test/fixtures";
import { digitFroms, pressDigit } from "../test/keys";
import { senderName, whyText } from "./SummaryPanel";
import { ThreadScreen } from "./ThreadScreen";

const u = () => useSummary.getState();

function backend(start: DemoSummaryStart = {}): SummaryBackend {
  const demo = demoSummaries({ senderOf: () => "lena@northwind.co", ...start });
  setSummaries(demo);
  return demo;
}

/** Opens a two-message thread from Lena at northwind.co and waits for what's kept to show. */
async function openThread(from = lena) {
  resetStore([thread("t", { from, messages: [message({ id: "t-1", from }), message({ id: "t-2", from })] })]);
  useStore.getState().open("t");
  render(<ThreadScreen />);
  await act(async () => {});
}

const card = (name: string | RegExp) => screen.getByRole("region", { name });

beforeEach(() => {
  backend();
  localStorage.clear();
  useSummary.setState({ threadId: undefined, run: 0, last: undefined });
});

describe("the kind you chose last", () => {
  it("S writes it straight away; with a summary shown, S changes its kind", async () => {
    useSummary.setState({ last: { kind: "facts" } });
    await openThread();
    const user = userEvent.setup();
    await user.keyboard("s");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    await screen.findByRole("region", { name: "Key facts" });
    await user.keyboard("s");
    expect(screen.getByRole("menu", { name: "Summarise as" })).toBeInTheDocument();
  });

  it("the Summarise button does the same", async () => {
    useSummary.setState({ last: { kind: "actions" } });
    await openThread();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /Summarise/ }));
    await screen.findByRole("region", { name: "Action points" });
    await user.click(screen.getByRole("button", { name: /Summarise/ }));
    expect(screen.getByRole("menu", { name: "Summarise as" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Summarise/ }));
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  });
});

describe("choosing the kind first", () => {
  it("the arrow beside Summarise opens the kinds without writing the last one", async () => {
    const demo = backend();
    const summarize = vi.spyOn(demo, "summarize");
    useSummary.setState({ last: { kind: "actions" } });
    await openThread();
    const user = userEvent.setup();
    const arrow = screen.getByRole("button", { name: "Choose a summary kind" });
    expect(arrow).toHaveAttribute("aria-expanded", "false");
    await user.click(arrow);
    expect(arrow).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("menu", { name: "Summarise as" })).toBeInTheDocument();
    expect(summarize).not.toHaveBeenCalled();
    // Again closes it.
    await user.click(arrow);
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    await user.click(arrow);
    await user.click(screen.getByRole("menuitemradio", { name: /Key facts/ }));
    await screen.findByRole("region", { name: "Key facts" });
    expect(summarize).toHaveBeenCalledTimes(1);
  });

  it("⇧S opens the kinds where S would write the last one", async () => {
    useSummary.setState({ last: { kind: "facts" } });
    await openThread();
    const user = userEvent.setup();
    await user.keyboard("{Shift>}s{/Shift}");
    expect(screen.getByRole("menu", { name: "Summarise as" })).toBeInTheDocument();
    expect(screen.queryByRole("region")).not.toBeInTheDocument();
    await user.keyboard("{Shift>}s{/Shift}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    await user.keyboard("{Shift>}s{/Shift}");
    await user.keyboard("2");
    await screen.findByRole("region", { name: "Detailed summary" });
  });
});

describe("discarding a summary", () => {
  it("X or Discard takes it away, and it stays away", async () => {
    const demo = backend({ kept: { t: demoView({ kind: "facts" }) } });
    await openThread();
    const user = userEvent.setup();
    await user.keyboard("x");
    expect(screen.queryByRole("region")).not.toBeInTheDocument();
    expect(await demo.kept("t")).toEqual({});
    // Nothing to discard: X does nothing.
    await user.keyboard("x");
    await user.keyboard("s1");
    await user.click(await screen.findByRole("button", { name: "DiscardX" }));
    expect(screen.queryByRole("region")).not.toBeInTheDocument();
  });

  it("X does nothing while the menu is open", async () => {
    backend({ kept: { t: demoView({ kind: "facts" }) } });
    await openThread();
    const user = userEvent.setup();
    await user.keyboard("sx");
    expect(screen.getByRole("menu", { name: "Summarise as" })).toBeInTheDocument();
    expect(card("Key facts")).toBeInTheDocument();
  });
});

describe("asking for a summary", () => {
  it.each(digitFroms)("1–5 pick a kind from the %s keys, and not with ⌥ or ⌘ held", async (from) => {
    await openThread();
    await userEvent.setup().keyboard("s");
    pressDigit(3, from, { alt: true });
    pressDigit(3, from, { meta: true });
    expect(screen.getByRole("menu", { name: "Summarise as" })).toBeInTheDocument();
    expect(pressDigit(3, from)).toBe(false);
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(await screen.findByRole("region", { name: "Action points" })).toBeInTheDocument();
  });

  it("S opens the kinds, 1–5 picks one, and Moss offers to make it the rule", async () => {
    await openThread();
    expect(screen.queryByRole("region")).not.toBeInTheDocument();
    const user = userEvent.setup();
    await user.keyboard("s");
    expect(screen.getByRole("menu", { name: "Summarise as" })).toBeInTheDocument();
    await user.keyboard("3");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    const actions = await screen.findByRole("region", { name: "Action points" });
    expect(actions).toHaveTextContent("You asked just now");
    expect(actions).toHaveTextContent("Approve the Q4 design budget or say what to change");
    expect(actions).toHaveTextContent("Always summarise mail from lena@northwind.co like this?");
    await user.keyboard("{Enter}");
    expect(card("Action points")).toHaveTextContent("Always for Lena Vogt");
    expect(card("Action points")).not.toHaveTextContent("Always summarise");
    expect(u().rule).toMatchObject({ sender: "lena@northwind.co", kind: "actions" });
  });

  it("↑ ↓ move through the open menu and ↵ picks, instead of going to another email", async () => {
    resetStore([
      thread("t", { from: lena, messages: [message({ id: "t-1", from: lena }), message({ id: "t-2", from: lena })] }),
      thread("next", { from: lena }),
    ]);
    useStore.getState().open("t");
    render(<ThreadScreen />);
    renderHook(() => useShortcuts());
    await act(async () => {});
    const user = userEvent.setup();
    await user.keyboard("s{ArrowDown}{ArrowDown}{ArrowUp}");
    expect(useStore.getState().openThreadId).toBe("t");
    expect(screen.getByRole("menu", { name: "Summarise as" })).toBeInTheDocument();
    await user.keyboard("{Enter}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    await screen.findByRole("region", { name: "Detailed summary" });
    // With the menu shut, ↓ goes to the next email again.
    await user.keyboard("{ArrowDown}");
    expect(useStore.getState().openThreadId).toBe("next");
  });

  it("⇧↵ makes it the rule for everyone at the domain, Esc says not now", async () => {
    await openThread();
    const user = userEvent.setup();
    await user.keyboard("s4");
    await screen.findByRole("region", { name: "Key facts" });
    await user.keyboard("{Escape}");
    expect(card("Key facts")).not.toHaveTextContent("Always summarise");
    expect(useStore.getState().view).toBe("thread");
    await user.keyboard("s1");
    await screen.findByRole("region", { name: "Short summary" });
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    expect(card("Short summary")).toHaveTextContent("Always for everyone at northwind.co");
  });

  it("a public mail service gets no domain choice, and ⇧↵ does nothing", async () => {
    const sam = person("Sam Lee", "sam@gmail.com");
    backend({ senderOf: () => "sam@gmail.com" });
    await openThread(sam);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /Summarise/ }));
    await user.click(screen.getByRole("menuitemradio", { name: /Detailed/ }));
    const detailed = await screen.findByRole("region", { name: "Detailed summary" });
    expect(detailed).toHaveTextContent("You asked just now · 2 emails");
    // Just the words: no column of dates beside them.
    expect(detailed).toHaveTextContent("Daniel updated the Q4 design budget");
    expect(detailed).not.toHaveTextContent(/Oct 27|Today/);
    expect(screen.queryByRole("button", { name: /Everyone at/ })).not.toBeInTheDocument();
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    expect(u().rule).toBeUndefined();
    await user.click(screen.getByRole("button", { name: "Always for Sam Lee↵" }));
    expect(u().rule?.sender).toBe("sam@gmail.com");
  });

  it("the offer's buttons work by click too", async () => {
    await openThread();
    const user = userEvent.setup();
    await user.keyboard("s2");
    await user.click(await screen.findByRole("button", { name: "Not nowEsc" }));
    expect(u().offer).toBe(false);
    await user.keyboard("s1");
    await user.click(await screen.findByRole("button", { name: /Everyone at northwind.co/ }));
    expect(u().rule?.sender).toBe("northwind.co");
  });

  it("Custom asks how, then writes it; ↵ in the box writes it again", async () => {
    await openThread();
    const user = userEvent.setup();
    await user.keyboard("s5");
    const box = screen.getByRole("textbox", { name: "How to summarise it" });
    expect(box).toHaveFocus();
    expect(card("Custom summary")).toHaveTextContent("Say how you want it");
    // The menu marks Custom while you write the instruction.
    await user.click(screen.getByRole("button", { name: /Summarise/ }));
    expect(screen.getByRole("menuitemradio", { name: /Custom/ })).toHaveAttribute("aria-checked", "true");
    await user.click(screen.getByRole("button", { name: /Summarise/ }));
    // Typing in the box is just typing.
    await user.type(box, "s1 Only the amount{Enter}");
    const custom = await screen.findByRole("region", { name: "Custom summary" });
    expect(custom).toHaveTextContent("Always summarise mail from lena@northwind.co this way?");
    expect(u().shown).toMatchObject({ kind: "custom", instruction: "s1 Only the amount" });
    expect(screen.getByRole("textbox", { name: "How to summarise it" })).not.toHaveFocus();
    await user.click(screen.getByRole("textbox", { name: "How to summarise it" }));
    await user.keyboard("{Enter}");
    expect(u().shown?.instruction).toBe("s1 Only the amount");
  });

  it("the Summarise button opens and closes the menu; a click outside or S closes it", async () => {
    await openThread();
    const user = userEvent.setup();
    const button = screen.getByRole("button", { name: /Summarise/ });
    await user.click(button);
    expect(button).toHaveAttribute("aria-expanded", "true");
    await user.click(button);
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    await user.click(button);
    await user.click(screen.getByRole("heading"));
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    await user.keyboard("s");
    await user.keyboard("s");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    await user.keyboard("s");
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    expect(useStore.getState().view).toBe("thread");
    await user.click(button);
    await user.click(screen.getByRole("button", { name: "EscClose" }));
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    // Other keys pass through the open menu.
    await user.keyboard("s");
    await user.keyboard("x");
    expect(screen.getByRole("menu")).toBeInTheDocument();
  });

  it("keys do nothing while the palette is open, or with ⌥", async () => {
    await openThread();
    const user = userEvent.setup();
    await user.keyboard("{Alt>}s{/Alt}");
    expect(screen.queryByRole("menu")).not.toBeInTheDocument();
    useStore.setState({ paletteOpen: true });
    await user.keyboard("s");
    expect(u().menuOpen).toBe(false);
  });
});

describe("states", () => {
  it("while it writes, Esc stops", async () => {
    setSummaries({ ...demoSummaries(), summarize: () => new Promise(() => {}) });
    await openThread();
    const user = userEvent.setup();
    await user.keyboard("s1");
    expect(card("Summarising 2 emails…")).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("region")).not.toBeInTheDocument();
    expect(useStore.getState().view).toBe("thread");
    await user.keyboard("s1");
    await user.click(screen.getByRole("button", { name: "EscStop" }));
    expect(screen.queryByRole("region")).not.toBeInTheDocument();
  });

  it("the summary shows as it's written, and Esc still stops it", async () => {
    backend({ state: "writing" });
    await openThread();
    const user = userEvent.setup();
    await user.keyboard("s3");
    expect(within(card("Summarising 2 emails…")).getByText("Approve the Q4 design budget or say what to change")).toBeInTheDocument();
    expect(screen.queryByText("Check the new user testing line")).not.toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("region")).not.toBeInTheDocument();
  });

  it("one email says so", async () => {
    setSummaries({ ...demoSummaries(), summarize: () => new Promise(() => {}) });
    resetStore([thread("t")]);
    useStore.getState().open("t");
    render(<ThreadScreen />);
    await userEvent.setup().keyboard("s1");
    expect(card("Summarising this email…")).toBeInTheDocument();
  });

  it("without the writing model it offers to download it, or AI settings", async () => {
    backend({ state: "notReady" });
    setAi(demoAi());
    const download = vi.spyOn(useAiSettings.getState(), "download").mockResolvedValue();
    await openThread();
    const user = userEvent.setup();
    await user.keyboard("s2");
    const state = await screen.findByRole("region", { name: "Summaries need the writing model" });
    await vi.waitFor(() => expect(state).toHaveTextContent("Gemma 4 E4B runs on this Mac. It’s about 5 GB and downloads in the background."));
    await user.keyboard("{Enter}");
    await user.click(screen.getByRole("button", { name: "Download↵" }));
    expect(download).toHaveBeenCalledTimes(2);
    await user.click(screen.getByRole("button", { name: "AI settingsCtrl+," }));
    expect(useStore.getState().settingsSection).toBe("ai");
  });

  it("before the AI's status is known, it says so plainly", async () => {
    backend({ state: "notReady" });
    setAi({ ...demoAi(), status: () => new Promise(() => {}) });
    await openThread();
    await userEvent.setup().keyboard("s2");
    expect(await screen.findByText("The writing model runs on this Mac and downloads in the background.")).toBeInTheDocument();
  });

  it("when it fails: ↵ tries again, S changes kind", async () => {
    const demo = backend({ state: "failed" });
    await openThread();
    const user = userEvent.setup();
    await user.keyboard("s4");
    expect(await screen.findByRole("region", { name: "Couldn’t summarise this one" })).toHaveTextContent(
      "The writing model stopped before it finished. Your email is fine.",
    );
    const attempts = vi.spyOn(demo, "summarize");
    await user.keyboard("{Enter}");
    await user.click(screen.getByRole("button", { name: "Try again↵" }));
    expect(attempts).toHaveBeenCalledTimes(2);
    await user.click(screen.getByRole("button", { name: "Change kindS" }));
    expect(screen.getByRole("menu")).toBeInTheDocument();
  });
});

describe("what's there as the email opens", () => {
  it("a rule's summary, with Stop rule; ⌘Z brings the rule back", async () => {
    const demo = backend({
      rules: [{ id: 1, sender: "lena@northwind.co", kind: "facts", createdAt: 0 }],
      kept: { t: demoView({ kind: "facts" }, { why: "rule" }) },
    });
    await openThread();
    const facts = card("Key facts");
    expect(facts).toHaveTextContent("Always for Lena Vogt");
    expect(facts).toHaveTextContent("€18,400");
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Stop rule" }));
    expect(card("Key facts")).toHaveTextContent("Rule stopped · ⌘Z brings it back");
    expect(await demo.rules()).toEqual([]);
    expect(screen.queryByRole("button", { name: "Stop rule" })).not.toBeInTheDocument();
    await user.keyboard("{Control>}z{/Control}");
    await vi.waitFor(() => expect(card("Key facts")).toHaveTextContent("Always for Lena Vogt"));
    expect((await demo.rules()).map((r) => r.sender)).toEqual(["lena@northwind.co"]);
    // Nothing to bring back: ⌘Z is the app's again.
    await user.keyboard("{Control>}z{/Control}");
  });

  it("the automatic one-liner says why, and explains itself", async () => {
    backend({ kept: { t: demoView({ kind: "short" }, { why: "replied" }) } });
    await openThread();
    const auto = card("Auto summary");
    expect(auto).toHaveTextContent("Because you replied in this conversation");
    expect(screen.getByRole("tooltip")).toHaveTextContent("Turn it off in Settings → AI.");
  });

  it("a summary a rule writes in the background appears", async () => {
    let written: (id: string) => void = () => {};
    const demo = demoSummaries({ senderOf: () => "lena@northwind.co", rules: [{ id: 1, sender: "northwind.co", kind: "actions", createdAt: 0 }] });
    setSummaries({ ...demo, onWritten: (h) => ((written = h), () => {}) });
    await openThread();
    expect(screen.queryByRole("region")).not.toBeInTheDocument();
    await demo.summarize("t", { kind: "actions" });
    await act(async () => written("t"));
    expect(card("Action points")).toHaveTextContent("Always for everyone at northwind.co");
  });

  it("a kept custom summary shows its instruction, without taking the keys", async () => {
    backend({ kept: { t: demoView({ kind: "custom", instruction: "As a list" }) } });
    await openThread();
    expect(screen.getByRole("textbox", { name: "How to summarise it" })).toHaveValue("As a list");
    await userEvent.setup().keyboard("s");
    expect(screen.getByRole("menu")).toBeInTheDocument();
  });
});

describe("why a summary is there", () => {
  const from = person("Dana Whitfield", "Dana@brightlane.com");
  const now = Date.UTC(2026, 10, 4, 12);
  const view = (extra: Partial<SummaryView>) => demoView({ kind: "short" }, { madeAt: now, ...extra });

  it("in plain words", () => {
    expect(whyText(view({}), undefined, from, 1, now)).toBe("You asked just now");
    expect(whyText(view({ madeAt: now - 3_600_000 }), undefined, from, 1, now)).toMatch(/^You asked at \d+:\d\d/);
    expect(whyText(view({ madeAt: now - 9 * 86_400_000 }), undefined, from, 1, now)).toMatch(/^You asked on Oct 26/);
    expect(whyText(view({ why: "important" }), undefined, from, 1, now)).toBe("Because it’s in Important");
    expect(whyText(view({ why: "rule" }), undefined, from, 1, now)).toBe("Written by a rule");
    const rule = { id: 1, kind: "short" as const, createdAt: 0 };
    expect(whyText(view({ why: "rule" }), { ...rule, sender: "dana@brightlane.com" }, from, 1, now)).toBe("Always for Dana Whitfield");
    expect(whyText(view({ why: "rule" }), { ...rule, sender: "jo@brightlane.com", name: "Jo" }, from, 1, now)).toBe("Always for Jo");
    expect(whyText(view({ why: "rule" }), { ...rule, sender: "jo@brightlane.com" }, from, 1, now)).toBe("Always for jo@brightlane.com");
    expect(whyText(view({ kind: "detailed", stale: true }), undefined, from, 6, now)).toBe("You asked just now · 6 emails · before the newest email");
  });

  it("a sender by the whole name they go by: a first word isn't one for a newsletter or a company", () => {
    expect(senderName(person("The Tinker Weekly", "tinker@tinkerweekly.example", { short: "The" }))).toBe("The Tinker Weekly");
    expect(senderName(person("Dana Whitfield", "d@x.co"))).toBe("Dana Whitfield");
    expect(senderName(person(" ", "d@x.co"))).toBe("d@x.co");
  });
});
