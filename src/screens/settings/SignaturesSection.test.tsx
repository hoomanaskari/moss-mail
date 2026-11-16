import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { gmailDemoAccount } from "../../data/demo";
import { useSignatures } from "../../state/signatures";
import { useStore } from "../../state/store";
import { resetDemoStore } from "../../test/fixtures";
import { SignaturesSection } from "./SignaturesSection";

const sig = () => useSignatures.getState();
const named = (name: string) => sig().signatures.find((x) => x.name === name)!;
const list = () => within(screen.getByRole("listbox", { name: "Your signatures" }));
const selected = () => list().getAllByRole("option").find((o) => o.getAttribute("aria-selected") === "true")!;

beforeEach(() => {
  resetDemoStore();
  useStore.getState().addAccount(gmailDemoAccount);
  useStore.getState().openSettings("signatures");
});

describe("Signatures settings", () => {
  it("lists signatures with who uses them, and opens on Studio", () => {
    render(<SignaturesSection />);
    expect(screen.getByText("Used by Work · new emails")).toBeInTheDocument();
    expect(screen.getByText("Used by Gmail · new emails")).toBeInTheDocument();
    expect(selected()).toHaveTextContent("Studio");
    expect(screen.getByLabelText("Name")).toHaveValue("Studio");
    expect(screen.getByLabelText("Line 3")).toHaveValue("rossstudio.art");
  });

  it("turns “Let Moss choose” off and on, and says what it does", async () => {
    render(<SignaturesSection />);
    expect(screen.getByText(/Later replies in a thread get your one-line one, or none/)).toBeInTheDocument();
    expect(screen.getByText(/Your pick in an email always wins/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("switch", { name: "Let Moss choose" }));
    expect(sig().letMossChoose).toBe(false);
  });

  it("↓ and ↑ move through the list (J and K do nothing), ↵ edits the name, clicking picks one", async () => {
    const user = userEvent.setup();
    render(<SignaturesSection />);
    const start = selected()!.textContent;
    await user.keyboard("xjJkK");
    expect(selected()!.textContent).toBe(start);
    await user.keyboard("{ArrowDown}");
    expect(selected()).toHaveTextContent("Personal");
    await user.keyboard("{ArrowDown}{ArrowUp}{ArrowUp}{ArrowUp}");
    expect(selected()).toHaveTextContent("Work (full)");
    await user.keyboard("{ArrowUp}");
    expect(selected()).toHaveTextContent("Work (full)");
    await user.keyboard("{Enter}");
    expect(screen.getByLabelText("Name")).toHaveFocus();
    // Typing in a field doesn't move the list.
    await user.keyboard("j{ArrowDown}");
    expect(selected()).toHaveTextContent("Work (full)j");
    await user.click(list().getByRole("option", { name: /^Personal/ }));
    expect(selected()).toHaveTextContent("Personal");
  });

  it("edits lines: ↵ adds one (up to four), ⌫ on an empty line removes it", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<SignaturesSection />);
    await user.click(screen.getByLabelText("Line 2"));
    await user.keyboard("{Enter}");
    await act(() => vi.advanceTimersByTimeAsync(10));
    expect(named("Studio").lines).toEqual(["Maya Ross", "Ross Studio · Photography and set design", "", "rossstudio.art"]);
    expect(screen.getByLabelText("Line 3")).toHaveFocus();
    await user.keyboard("+44 20 1234");
    expect(named("Studio").lines[2]).toBe("+44 20 1234");
    // Four lines is the most.
    await user.keyboard("{Enter}");
    expect(named("Studio").lines).toHaveLength(4);
    await user.clear(screen.getByLabelText("Line 3"));
    await user.keyboard("{Backspace}");
    await act(() => vi.advanceTimersByTimeAsync(10));
    expect(named("Studio").lines).toEqual(["Maya Ross", "Ross Studio · Photography and set design", "rossstudio.art"]);
    expect(screen.getByLabelText("Line 2")).toHaveFocus();
    // ⌫ inside text just deletes a letter.
    await user.keyboard("{Backspace}");
    expect(named("Studio").lines[1]).toBe("Ross Studio · Photography and set desig");
    vi.useRealTimers();
  });

  it("keeps the last line, and a one-line signature can’t lose it", async () => {
    const user = userEvent.setup();
    render(<SignaturesSection />);
    await user.click(list().getByRole("option", { name: /^Work \(short\)/ }));
    await user.clear(screen.getByLabelText("Line 1"));
    await user.keyboard("{Backspace}");
    expect(named("Work (short)").lines).toEqual([""]);
  });

  it("renames, adds with ⌘N or the hint, and deletes with ⌘⌫ or the button", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    render(<SignaturesSection />);
    await user.type(screen.getByLabelText("Name"), " art");
    expect(sig().signatures.some((x) => x.name === "Studio art")).toBe(true);
    await user.keyboard("{Meta>}n{/Meta}");
    await act(() => vi.advanceTimersByTimeAsync(10));
    expect(selected()).toHaveTextContent("New signature");
    expect(screen.getByLabelText("Line 1")).toHaveValue("Maya Ross");
    await user.click(screen.getByRole("button", { name: /New signature/ }));
    expect(selected()).toHaveTextContent("New signature 2");
    // ⌘⌫ in a field is left to the field.
    screen.getByLabelText("Name").focus();
    await user.keyboard("{Meta>}{Backspace}{/Meta}");
    expect(sig().signatures).toHaveLength(6);
    screen.getByLabelText("Name").blur();
    await user.keyboard("{Meta>}{Backspace}{/Meta}");
    expect(sig().signatures).toHaveLength(5);
    expect(selected()).toHaveTextContent("New signature");
    await user.click(screen.getByRole("button", { name: /Delete/ }));
    expect(selected()).toHaveTextContent("Personal");
    vi.useRealTimers();
  });

  it("with no signatures left there is nothing to edit", async () => {
    useSignatures.setState({ signatures: [] });
    const user = userEvent.setup();
    render(<SignaturesSection />);
    expect(screen.queryByLabelText("Name")).not.toBeInTheDocument();
    await user.keyboard("{ArrowDown}{Meta>}{Backspace}{/Meta}");
    expect(useStore.getState().view).toBe("settings");
  });

  it("with no signatures, says what they’re for and starts the first one", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    useSignatures.setState({ signatures: [] });
    render(<SignaturesSection />);
    const empty = screen.getByRole("status");
    expect(empty).toHaveTextContent("No signatures yet");
    expect(empty).toHaveTextContent(/Moss adds it to the end of your emails/);
    expect(screen.queryByRole("listbox", { name: "Your signatures" })).not.toBeInTheDocument();
    // One way to start, not two side by side.
    await user.click(screen.getByRole("button", { name: /New signature/ }));
    await act(() => vi.advanceTimersByTimeAsync(10));
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
    expect(selected()).toHaveTextContent("New signature");
    expect(screen.getByLabelText("Name")).toHaveValue("New signature");
    vi.useRealTimers();
  });

  it("picks each account’s signature for new emails and replies, or none", async () => {
    const user = userEvent.setup();
    render(<SignaturesSection />);
    expect(screen.getByRole("combobox", { name: "Work: new emails" })).toHaveValue("work-full");
    expect(screen.getByRole("combobox", { name: "Personal: replies" })).toHaveValue("");
    await user.selectOptions(screen.getByRole("combobox", { name: "Personal: replies" }), "studio");
    expect(sig().byAccount.personal.replies).toBe("studio");
    await user.selectOptions(screen.getByRole("combobox", { name: "Work: new emails" }), "");
    expect(sig().byAccount.work.newEmails).toBeNull();
  });

  it("each address you send from follows its account until you pick one for it", async () => {
    const user = userEvent.setup();
    const personal = useStore.getState().accounts.find((a) => a.id === "personal")!;
    useStore.setState((s) => ({
      accounts: s.accounts.map((a) =>
        a.id === "personal"
          ? { ...personal, addresses: [{ email: "hi@mayaross.me", state: "added", source: "you", seen: 0, blocked: false }] }
          : a,
      ),
    }));
    const { rerender } = render(<SignaturesSection />);
    const forNew = () => screen.getByRole("combobox", { name: "hi@mayaross.me: new emails" });
    expect(forNew()).toHaveValue("same");
    await user.selectOptions(forNew(), "studio");
    expect(sig().byAddress["hi@mayaross.me"]).toEqual({ newEmails: "studio", replies: null });
    rerender(<SignaturesSection />);
    expect(forNew()).toHaveValue("studio");
    expect(screen.getByRole("combobox", { name: "hi@mayaross.me: replies" })).toHaveValue("");
    await user.selectOptions(screen.getByRole("combobox", { name: "hi@mayaross.me: replies" }), "work-short");
    await user.selectOptions(forNew(), "");
    expect(sig().byAddress["hi@mayaross.me"]).toEqual({ newEmails: null, replies: "work-short" });
    await user.selectOptions(screen.getByRole("combobox", { name: "hi@mayaross.me: replies" }), "same");
    expect(sig().byAddress).toEqual({});
    rerender(<SignaturesSection />);
    expect(forNew()).toHaveValue("same");
  });

  it("shows web addresses as links, and an account with no choices yet as none", () => {
    useSignatures.setState({ byAccount: {} });
    render(<SignaturesSection />);
    expect(screen.getByRole("combobox", { name: "Gmail: replies" })).toHaveValue("");
    expect(screen.getByLabelText("Line 3").className).not.toBe(screen.getByLabelText("Line 2").className);
  });
});

it("a new signature starts empty when no account has a sender name", async () => {
  useStore.setState({ accounts: [] });
  render(<SignaturesSection />);
  await userEvent.setup().keyboard("{Meta>}n{/Meta}");
  expect(useSignatures.getState().signatures.at(-1)!.lines).toEqual([""]);
});
