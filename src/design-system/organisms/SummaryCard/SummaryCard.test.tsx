import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Repeat } from "lucide-react";
import { describe, expect, it, vi } from "vitest";
import { SummaryAction, SummaryBody, SummaryCard, SummaryInstruction, SummaryNote, SummaryOffer, SummarySkeleton } from "./SummaryCard";

const open = vi.hoisted(() => vi.fn());
vi.mock("../../../lib/links", () => ({ openLink: open }));

describe("SummaryCard", () => {
  it("says what it is and why, with Change kind and Stop rule", async () => {
    const onChangeKind = vi.fn();
    const onStopRule = vi.fn();
    render(
      <SummaryCard label="Action points" why="Always for Dana" whyIcon={Repeat} onChangeKind={onChangeKind} onStopRule={onStopRule}>
        <SummarySkeleton />
      </SummaryCard>,
    );
    const card = screen.getByRole("region", { name: "Action points" });
    expect(card).toHaveTextContent("Action pointsAlways for DanaChange kindSStop rule");
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Change kindS" }));
    await user.click(screen.getByRole("button", { name: "Stop rule" }));
    expect(onChangeKind).toHaveBeenCalled();
    expect(onStopRule).toHaveBeenCalled();
  });

  it("explains an automatic summary, and can be stopped while it's written", async () => {
    const onStop = vi.fn();
    render(
      <SummaryCard label="Auto summary" why="Because it's in Important" info={{ title: "Why is there a summary?", text: "Moss writes one line.", hint: "Turn it off in Settings → AI." }} onStop={onStop} footer={<p>Below</p>}>
        <p>Line</p>
      </SummaryCard>,
    );
    expect(screen.getByRole("tooltip")).toHaveTextContent("Why is there a summary?Moss writes one line.Turn it off in Settings → AI.");
    expect(screen.getByLabelText("Why is there a summary?")).toHaveAttribute("tabindex", "0");
    expect(screen.getByText("Below")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "EscStop" }));
    expect(onStop).toHaveBeenCalled();
  });

  it("states have words and actions with keys", async () => {
    const onClick = vi.fn();
    render(
      <SummaryNote text="Your email is fine.">
        <SummaryAction keys="↵" onClick={onClick}>
          Try again
        </SummaryAction>
      </SummaryNote>,
    );
    expect(screen.getByText("Your email is fine.")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Try again↵" }));
    expect(onClick).toHaveBeenCalled();
  });
});

describe("the always offer", () => {
  const props = { email: "dana@brightlane.com", name: "Dana", onAlways: vi.fn(), onDomain: vi.fn(), onNotNow: vi.fn() };

  it("asks about the sender's address, with the domain unless it's public", async () => {
    const { rerender } = render(<SummaryOffer {...props} domain="brightlane.com" custom={false} />);
    expect(screen.getByText(/Always summarise mail from dana@brightlane.com like this\?/)).toBeInTheDocument();
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Always for Dana↵" }));
    await user.click(screen.getByRole("button", { name: "Everyone at brightlane.comShift+↵" }));
    await user.click(screen.getByRole("button", { name: "Not nowEsc" }));
    expect([props.onAlways, props.onDomain, props.onNotNow].map((f) => f.mock.calls.length)).toEqual([1, 1, 1]);
    rerender(<SummaryOffer {...props} custom problem="Couldn’t save it." />);
    expect(screen.getByText(/this way\?/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Everyone at/ })).not.toBeInTheDocument();
    expect(screen.getByText("Couldn’t save it.")).toBeInTheDocument();
  });
});

it("Custom's instruction box writes it again on ↵", async () => {
  const onChange = vi.fn();
  const onRedo = vi.fn();
  const { unmount } = render(<SummaryInstruction value="Only dates" onChange={onChange} onRedo={onRedo} />);
  expect(screen.getByRole("textbox")).not.toHaveFocus();
  unmount();
  render(<SummaryInstruction value="Only dates" focus onChange={onChange} onRedo={onRedo} />);
  const box = screen.getByRole("textbox", { name: "How to summarise it" });
  expect(box).toHaveFocus();
  const user = userEvent.setup();
  await user.type(box, "!");
  expect(onChange).toHaveBeenCalledWith("Only dates!");
  await user.keyboard("{Enter}");
  expect(onRedo).toHaveBeenCalledTimes(1);
});

describe("SummaryBody", () => {
  it("text in paragraphs", () => {
    render(<SummaryBody content={{ format: "text", text: "One.\n\nTwo." }} />);
    expect(screen.getAllByText(/One\.|Two\./).map((t) => t.parentElement?.tagName)).toEqual(["P", "P"]);
  });

  it("lays text out to scan: separate lines as a list, a line ending in a colon as its heading", () => {
    const text =
      "OpenAI appointed Dan Driscoll.\nAnthropic uses Claude to design chips.\n\n" +
      "Anthropic filed an amended S-1.\n\n" +
      "Other updates include:\nWizard gives players finer control.\n- Grok Bot compares prices.\n\n" +
      "Also:\n\n* Voice in ChatGPT Finances\n• Plugins for Claude Team";
    render(<SummaryBody content={{ format: "text", text }} />);
    const lists = screen.getAllByRole("list");
    expect(lists.map((l) => within(l).getAllByRole("listitem").map((li) => li.textContent))).toEqual([
      ["OpenAI appointed Dan Driscoll.", "Anthropic uses Claude to design chips."],
      ["Wizard gives players finer control.", "Grok Bot compares prices."],
      ["Voice in ChatGPT Finances", "Plugins for Claude Team"],
    ]);
    expect(screen.getByText("Anthropic filed an amended S-1.").parentElement?.tagName).toBe("P");
    // A heading names the list under it, even across a blank line.
    expect(screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent)).toEqual(["Other updates include", "Also"]);
    expect(lists[1]).toHaveAccessibleName("Other updates include");
  });

  it("shows the Markdown a model writes: headings, numbered or bulleted points, emphasis", () => {
    const text = "The email is from **The Tinker Weekly**, *issue 28*.\n\n**What matters:**\n1. Paper Studio loses its home\n2) Brightboard **restructures**\n\n## Also\n+ Framely grows";
    const { container } = render(<SummaryBody content={{ format: "text", text }} />);
    expect(container).not.toHaveTextContent("**");
    expect(screen.getByText("The Tinker Weekly").tagName).toBe("STRONG");
    expect(screen.getByText("issue 28").tagName).toBe("EM");
    expect(screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent)).toEqual(["What matters", "Also"]);
    const lists = screen.getAllByRole("list");
    expect(within(lists[0]).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["Paper Studio loses its home", "Brightboard restructures"]);
    expect(screen.getByText("restructures").tagName).toBe("STRONG");
    expect(screen.getByText("Framely grows").closest("p")).not.toBeNull();
  });

  it("a lone line ending in a colon with nothing after it stays text", () => {
    render(<SummaryBody content={{ format: "text", text: "Prices:" }} />);
    expect(screen.getByText("Prices:").parentElement?.tagName).toBe("P");
    expect(screen.queryByRole("heading")).not.toBeInTheDocument();
  });

  it("action points tick off here only, with their dates", async () => {
    const { rerender } = render(<SummaryBody content={{ format: "actions", items: [{ text: "Countersign", due: "Thu 6 Nov" }, { text: "Be ready" }] }} />);
    expect(screen.getAllByRole("listitem").map((li) => li.textContent)).toEqual(["CountersignThu 6 Nov", "Be ready"]);
    // A model's emphasis shows as emphasis; the box is named in plain words.
    rerender(<SummaryBody content={{ format: "actions", items: [{ text: "Send the **Q4** numbers" }] }} />);
    expect(screen.getByText("Q4").tagName).toBe("STRONG");
    expect(screen.getByRole("checkbox", { name: "Send the Q4 numbers" })).toBeInTheDocument();
    rerender(<SummaryBody content={{ format: "actions", items: [{ text: "Countersign", due: "Thu 6 Nov" }, { text: "Be ready" }] }} />);
    const user = userEvent.setup();
    const box = screen.getByRole("checkbox", { name: "Countersign" });
    await user.click(box);
    expect(box).toBeChecked();
    await user.click(box);
    expect(box).not.toBeChecked();
    rerender(<SummaryBody content={{ format: "actions", items: [] }} />);
    expect(screen.getByText("Nothing for you to do.")).toBeInTheDocument();
  });

  it("key facts in labelled rows, one per line with what each is; web links open in the browser", async () => {
    const { rerender } = render(
      <SummaryBody
        content={{
          format: "facts",
          items: [
            { kind: "person", value: "Dana Whitfield", detail: "Brightlane's lead" },
            { kind: "person", value: "Jonas Weber" },
            { kind: "amount", value: "€24,000" },
            { kind: "link", value: "https://www.brightlane.com/spring", detail: "the plan" },
            { kind: "link", value: "https://brightlane.com/terms" },
            { kind: "link", value: "contract-final.pdf" },
            { kind: "other", value: "Room 4" },
          ],
        }}
      />,
    );
    const rows = screen.getAllByRole("term").map((t) => t.textContent);
    expect(rows).toEqual(["People", "Amounts", "Links", "Other"]);
    const people = within(screen.getByText("Dana Whitfield").closest("dd")!).getAllByRole("listitem");
    expect(people.map((p) => p.textContent)).toEqual(["Dana Whitfield Brightlane's lead", "Jonas Weber"]);
    // A link reads as what it is, with the site it's on; one without a name shows where it goes.
    const plan = screen.getByRole("link", { name: "the plan" });
    expect(plan.closest("li")).toHaveTextContent("the plan brightlane.com");
    await userEvent.setup().click(plan);
    expect(open).toHaveBeenCalledWith("https://www.brightlane.com/spring");
    expect(screen.getByRole("link", { name: "brightlane.com/terms" })).toHaveAttribute("href", "https://brightlane.com/terms");
    expect(within(plan.closest("dd")!).getByText("contract-final.pdf").tagName).toBe("SPAN");
    rerender(<SummaryBody content={{ format: "facts", items: [] }} />);
    expect(screen.getByText("No people, dates, amounts or links in it.")).toBeInTheDocument();
  });

  it("a long list of key facts shows the first five, and the rest on asking", async () => {
    const links = Array.from({ length: 7 }, (_, i) => ({ kind: "link" as const, value: `https://keyward.example/changelog/${i}`, detail: `Change ${i}` }));
    render(<SummaryBody content={{ format: "facts", items: [...links, { kind: "person", value: "Emily" }] }} />);
    expect(screen.getAllByRole("link").map((l) => l.textContent)).toEqual(["Change 0", "Change 1", "Change 2", "Change 3", "Change 4"]);
    expect(screen.getAllByRole("button")).toHaveLength(1);
    await userEvent.setup().click(screen.getByRole("button", { name: "2 more" }));
    expect(screen.getAllByRole("link")).toHaveLength(7);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});
