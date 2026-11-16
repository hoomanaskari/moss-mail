import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import type { Person } from "../../../data/types";
import { jonas, lena, maya, person, personal, work } from "../../../test/fixtures";
import { ComposerFields, type ComposerFieldsProps } from "./ComposerFields";

const ines = person("Ines Duarte", "ines@studio-ines.pt", { short: "Ines" });
const priya = person("Priya Nair", "priya@northwind.co");
const daniel = person("Daniel Park", "daniel@northwind.co", { short: "Daniel" });
const joanna = person("Joanna Lee", "joanna@northwind.co");
const jo = person("Jo March", "jo@elsewhere.org");
const contacts = [lena, jonas, ines, priya, daniel, joanna, jo, maya];

function props(overrides: Partial<ComposerFieldsProps> = {}): ComposerFieldsProps {
  return {
    from: work,
    to: [],
    cc: [],
    bcc: [],
    showCc: false,
    showBcc: false,
    open: true,
    contacts,
    onChange: vi.fn(),
    onShowCc: vi.fn(),
    onShowBcc: vi.fn(),
    onOpen: vi.fn(),
    ...overrides,
  };
}

/** Holds the recipients in state, the way the composer does, so chips appear and disappear. */
function Stateful(initial: Partial<ComposerFieldsProps>) {
  const [people, setPeople] = useState<Record<"to" | "cc" | "bcc", Person[]>>({
    to: initial.to ?? [],
    cc: initial.cc ?? [],
    bcc: initial.bcc ?? [],
  });
  return (
    <ComposerFields
      {...props(initial)}
      {...people}
      onChange={(field, list) => setPeople((p) => ({ ...p, [field]: list }))}
    />
  );
}

const chips = () => screen.queryAllByRole("button", { name: /^Remove / }).map((b) => b.getAttribute("aria-label")!.slice(7));
const options = () => screen.queryAllByRole("option");
/** Names in the suggestion list, top to bottom. */
const suggested = () => options().map((o) => contacts.find((c) => o.textContent?.includes(c.email))?.name);
const highlighted = () => options().find((o) => o.getAttribute("aria-selected") === "true");

describe("summary", () => {
  it("folds everything into one line: account, short names, cc and a bcc count", () => {
    render(<ComposerFields {...props({ open: false, to: [lena, priya], cc: [jonas, ines, daniel], bcc: [maya] })} />);
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    expect(screen.getByText("Work")).toBeInTheDocument();
    expect(screen.getByText("Lena, Priya Nair")).toBeInTheDocument();
    expect(screen.getByText("cc")).toBeInTheDocument();
    expect(screen.getByText("Jonas,")).toBeInTheDocument();
    // An outsider is a pill and gets no comma; the last cc gets none either.
    expect(screen.getByText("Ines")).toBeInTheDocument();
    expect(screen.getByText("Daniel")).toBeInTheDocument();
    expect(screen.getByText("1 bcc")).toBeInTheDocument();
  });

  it("names the address it goes from when it isn't the account's own", () => {
    const sending = { accountId: "work", email: "hi@rossstudio.art", name: "Maya Ross", primary: false };
    const { rerender } = render(<ComposerFields {...props({ open: false, to: [lena], sending })} />);
    expect(screen.getByText("hi@rossstudio.art")).toBeInTheDocument();
    rerender(<ComposerFields {...props({ open: false, to: [lena], sending: { ...sending, email: work.address, primary: true } })} />);
    expect(screen.queryByText(work.address)).not.toBeInTheDocument();
  });

  it("leaves out cc and bcc when there are none", () => {
    render(<ComposerFields {...props({ open: false, to: [lena] })} />);
    expect(screen.getByText("Lena")).toBeInTheDocument();
    expect(screen.queryByText("cc")).not.toBeInTheDocument();
    expect(screen.queryByText(/bcc/)).not.toBeInTheDocument();
  });

  it("Edit opens the fields", async () => {
    const p = props({ open: false, to: [lena] });
    render(<ComposerFields {...p} />);
    await userEvent.setup().click(screen.getByRole("button", { name: /Edit/ }));
    expect(p.onOpen).toHaveBeenCalledOnce();
  });
});

describe("open fields", () => {
  it("shows the From account with sender name and address", () => {
    render(<ComposerFields {...props()} />);
    expect(screen.getByRole("button", { name: /Maya Ross\s+·\s+maya@northwind\.co/ })).toBeInTheDocument();
  });

  it("shows the address it goes from, and opens the From menu to change it", async () => {
    const studio = { ...work, addresses: [{ email: "hi@rossstudio.art", name: "Ross Studio", state: "added" as const, source: "you" as const, seen: 0, blocked: false }] };
    const onToggleFrom = vi.fn();
    const onChooseFrom = vi.fn();
    const onEditAddresses = vi.fn();
    const sending = { accountId: "work", email: "hi@rossstudio.art", name: "Ross Studio", primary: false };
    const shown = (fromOpen: boolean) => (
      <ComposerFields
        {...props({ from: studio, sending, accounts: [studio], fromOpen, onToggleFrom, onChooseFrom, onEditAddresses })}
      />
    );
    const { rerender } = render(shown(false));
    const picker = screen.getByRole("button", { name: /Ross Studio\s+·\s+hi@rossstudio\.art/ });
    expect(picker).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(picker);
    expect(onToggleFrom).toHaveBeenCalledOnce();
    rerender(shown(true));
    await userEvent.click(screen.getByRole("radio", { name: /maya@northwind\.co/ }));
    expect(onChooseFrom).toHaveBeenCalledWith("work", "maya@northwind.co");
    await userEvent.click(screen.getByRole("button", { name: /Add an address/ }));
    expect(onEditAddresses).toHaveBeenCalledOnce();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onToggleFrom).toHaveBeenCalledTimes(2);
  });

  it("a menu opened without handlers does nothing when used", async () => {
    render(<ComposerFields {...props({ fromOpen: true })} />);
    await userEvent.click(screen.getByRole("radio", { name: /maya@northwind\.co/ }));
    await userEvent.click(screen.getByRole("button", { name: /Add an address/ }));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(screen.getByRole("radiogroup", { name: "Send from" })).toBeInTheDocument();
  });

  it("shows why the account was picked only when there is a reason", () => {
    const { rerender } = render(<ComposerFields {...props()} />);
    expect(screen.queryByText("The account you use with Priya")).not.toBeInTheDocument();
    rerender(<ComposerFields {...props({ fromReason: "The account you use with Priya" })} />);
    expect(screen.getByText("The account you use with Priya")).toBeInTheDocument();
  });

  it("shows only To at first, with buttons to add CC and BCC", async () => {
    const user = userEvent.setup();
    const p = props();
    render(<ComposerFields {...p} />);
    expect(screen.getByRole("textbox", { name: "To recipients" })).toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: "CC recipients" })).not.toBeInTheDocument();
    expect(screen.queryByRole("textbox", { name: "BCC recipients" })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /^CC/ }));
    await user.click(screen.getByRole("button", { name: /^BCC/ }));
    expect(p.onShowCc).toHaveBeenCalledOnce();
    expect(p.onShowBcc).toHaveBeenCalledOnce();
  });

  it("shows the CC row and only the BCC button once CC is on", () => {
    render(<ComposerFields {...props({ showCc: true })} />);
    expect(screen.getByRole("textbox", { name: "CC recipients" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^CC/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^BCC/ })).toBeInTheDocument();
  });

  it("shows only the CC button once BCC is on, and says BCC is hidden", () => {
    render(<ComposerFields {...props({ showBcc: true })} />);
    expect(screen.getByRole("textbox", { name: "BCC recipients" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^CC/ })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^BCC/ })).not.toBeInTheDocument();
    expect(screen.getByText("Hidden from everyone else")).toBeInTheDocument();
  });

  it("has no add buttons with both rows on", () => {
    render(<ComposerFields {...props({ showCc: true, showBcc: true })} />);
    expect(screen.getAllByRole("textbox")).toHaveLength(3);
    expect(screen.queryByRole("button", { name: /^\+?B?CC/ })).not.toBeInTheDocument();
  });

  it("shows a chip per recipient and marks outsiders", () => {
    render(<ComposerFields {...props({ to: [lena, ines] })} />);
    expect(chips()).toEqual(["Lena Vogt", "Ines Duarte"]);
    expect(screen.getByText("· outside")).toBeInTheDocument();
    expect(screen.getAllByText("· outside")).toHaveLength(1);
  });

  it("has no outsiders for an account without an organisation", () => {
    render(<ComposerFields {...props({ from: personal, to: [lena, ines] })} />);
    expect(screen.queryByText("· outside")).not.toBeInTheDocument();
  });

  it("removing a chip reports the list without that person, for that field", async () => {
    const p = props({ showCc: true, to: [lena], cc: [jonas, ines] });
    render(<ComposerFields {...p} />);
    await userEvent.setup().click(screen.getByRole("button", { name: "Remove Jonas Weber" }));
    expect(p.onChange).toHaveBeenCalledWith("cc", [ines]);
  });

  it("clicking anywhere on a row focuses its input", async () => {
    render(<ComposerFields {...props({ showCc: true })} />);
    await userEvent.setup().click(screen.getByText("CC", { selector: "span" }));
    expect(screen.getByRole("textbox", { name: "CC recipients" })).toHaveFocus();
  });
});

describe("autocomplete", () => {
  it("suggests nobody until you type", async () => {
    const user = userEvent.setup();
    render(<Stateful />);
    await user.type(screen.getByRole("textbox", { name: "To recipients" }), "   ");
    expect(options()).toHaveLength(0);
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
  });

  it("matches names and addresses, ignoring case, at most five", async () => {
    const user = userEvent.setup();
    render(<Stateful />);
    const input = screen.getByRole("textbox", { name: "To recipients" });
    await user.type(input, "JO");
    expect(suggested()).toEqual([
      "Jonas Weber",
      "Joanna Lee",
      "Jo March",
    ]);
    await user.clear(input);
    await user.type(input, "studio-");
    expect(options().map((o) => o.textContent)).toEqual(["IDInes Duarteines@studio-ines.pt↵"]);
    await user.clear(input);
    await user.type(input, "a");
    expect(options()).toHaveLength(5);
  });

  it("puts first whoever you chose before for the same or similar text", async () => {
    const user = userEvent.setup();
    render(<Stateful picks={[{ typed: "jo m", email: jo.email }]} />);
    await user.type(screen.getByRole("textbox", { name: "To recipients" }), "jo");
    expect(suggested()).toEqual(["Jo March", "Jonas Weber", "Joanna Lee"]);
  });

  it("brings up someone you chose before even when more than five match", async () => {
    const user = userEvent.setup();
    render(<Stateful picks={[{ typed: "a", email: maya.email }]} />);
    await user.type(screen.getByRole("textbox", { name: "To recipients" }), "a");
    expect(suggested()).toHaveLength(5);
    expect(suggested()[0]).toBe(maya.name);
  });

  it("says who you chose and what you had typed, by key or by click", async () => {
    const user = userEvent.setup();
    const onPick = vi.fn();
    render(<Stateful onPick={onPick} showCc />);
    await user.type(screen.getByRole("textbox", { name: "To recipients" }), " Jo{Enter}");
    expect(onPick).toHaveBeenLastCalledWith("jo", jonas);
    await user.type(screen.getByRole("textbox", { name: "CC recipients" }), "pri");
    await user.click(screen.getByRole("option", { name: /Priya Nair/ }));
    expect(onPick).toHaveBeenLastCalledWith("pri", priya);
    // A whole address typed out is not a choice from the list.
    await user.type(screen.getByRole("textbox", { name: "To recipients" }), "new@x.io{Enter}");
    expect(onPick).toHaveBeenCalledTimes(2);
  });

  it("leaves out people already in the field", async () => {
    const user = userEvent.setup();
    render(<Stateful to={[jonas]} />);
    await user.type(screen.getByRole("textbox", { name: "To recipients" }), "jo");
    expect(suggested()).toEqual(["Joanna Lee", "Jo March"]);
  });

  it("grows the input with the text", async () => {
    const user = userEvent.setup();
    render(<Stateful />);
    const input = screen.getByRole("textbox", { name: "To recipients" });
    expect(input).toHaveAttribute("size", "1");
    await user.type(input, "len");
    expect(input).toHaveAttribute("size", "4");
  });

  it("arrow keys move the highlight and wrap", async () => {
    const user = userEvent.setup();
    render(<Stateful />);
    await user.type(screen.getByRole("textbox", { name: "To recipients" }), "jo");
    expect(highlighted()).toHaveTextContent("Jonas Weber");
    await user.keyboard("{ArrowDown}");
    expect(highlighted()).toHaveTextContent("Joanna Lee");
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(highlighted()).toHaveTextContent("Jonas Weber");
    await user.keyboard("{ArrowUp}");
    expect(highlighted()).toHaveTextContent("Jo March");
  });

  it("arrow keys are not swallowed when there is nothing to pick", () => {
    render(<Stateful />);
    const input = screen.getByRole("textbox", { name: "To recipients" });
    expect(fireEvent.keyDown(input, { key: "ArrowDown" })).toBe(true);
    expect(fireEvent.keyDown(input, { key: "ArrowUp" })).toBe(true);
  });

  it("Enter adds the highlighted person and clears the text", async () => {
    const user = userEvent.setup();
    render(<Stateful />);
    const input = screen.getByRole("textbox", { name: "To recipients" });
    await user.type(input, "jo{ArrowDown}{Enter}");
    expect(chips()).toEqual(["Joanna Lee"]);
    expect(input).toHaveValue("");
    expect(options()).toHaveLength(0);
    // The highlight starts at the top again for the next person.
    await user.type(input, "jo");
    expect(highlighted()).toHaveTextContent("Jonas Weber");
  });

  it("Tab adds the highlighted person and keeps focus in the field", async () => {
    const user = userEvent.setup();
    render(<Stateful />);
    const input = screen.getByRole("textbox", { name: "To recipients" });
    await user.type(input, "lena{Tab}");
    expect(chips()).toEqual(["Lena Vogt"]);
    expect(input).toHaveFocus();
  });

  it("Enter and Tab do nothing without a match", async () => {
    const p = props();
    render(<ComposerFields {...p} />);
    const input = screen.getByRole("textbox", { name: "To recipients" });
    await userEvent.setup().type(input, "nobody{Enter}");
    expect(fireEvent.keyDown(input, { key: "Tab" })).toBe(true);
    expect(p.onChange).not.toHaveBeenCalled();
    expect(input).toHaveValue("nobody");
  });

  it("clicking a suggestion adds it", async () => {
    const user = userEvent.setup();
    render(<Stateful showCc />);
    await user.type(screen.getByRole("textbox", { name: "CC recipients" }), "pri");
    await user.click(screen.getByRole("option", { name: /Priya Nair/ }));
    expect(chips()).toEqual(["Priya Nair"]);
  });

  it("hovering a suggestion leaves the highlight where it was", async () => {
    const user = userEvent.setup();
    render(<Stateful />);
    await user.type(screen.getByRole("textbox", { name: "To recipients" }), "jo");
    const first = screen.getAllByRole("option")[0];
    const jo = screen.getByRole("option", { name: /Jo March/ });
    expect(first).not.toBe(jo);
    fireEvent.mouseMove(jo);
    await user.hover(jo);
    expect(highlighted()).toBe(first);
    await user.keyboard("{Enter}");
    expect(chips()).not.toContain("Jo March");
  });

  it("lines the suggestions up under the input", async () => {
    const original = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "offsetLeft")!;
    Object.defineProperty(HTMLElement.prototype, "offsetLeft", {
      configurable: true,
      get(this: HTMLElement) {
        return this.tagName === "INPUT" ? 120 : 0;
      },
    });
    try {
      render(<Stateful />);
      await userEvent.setup().type(screen.getByRole("textbox", { name: "To recipients" }), "jo");
      expect(screen.getByRole("listbox")).toHaveStyle({ left: "106px" });
    } finally {
      Object.defineProperty(HTMLElement.prototype, "offsetLeft", original);
    }
  });

  it("each row keeps its own text and suggestions", async () => {
    const user = userEvent.setup();
    render(<Stateful showCc showBcc />);
    await user.type(screen.getByRole("textbox", { name: "BCC recipients" }), "ines{Enter}");
    expect(screen.getByRole("textbox", { name: "To recipients" })).toHaveValue("");
    expect(chips()).toEqual(["Ines Duarte"]);
    // Ines is only taken in BCC, so To still suggests her.
    await user.type(screen.getByRole("textbox", { name: "To recipients" }), "ines");
    expect(options()).toHaveLength(1);
  });
});

describe("typed addresses", () => {
  const typed = (text: string) => userEvent.setup().type(screen.getByRole("textbox", { name: "To recipients" }), text);

  it("Enter adds a full address nobody knows yet, and clears the text", async () => {
    render(<Stateful />);
    await typed("sam@harbour.studio{Enter}");
    expect(chips()).toEqual(["sam@harbour.studio"]);
    expect(screen.getByRole("textbox", { name: "To recipients" })).toHaveValue("");
  });

  it("Tab, comma and semicolon add it too", async () => {
    render(<Stateful />);
    await typed("a@one.io{Tab}b@two.io,c@three.io;");
    expect(chips()).toEqual(["a@one.io", "b@two.io", "c@three.io"]);
    expect(screen.getByRole("textbox", { name: "To recipients" })).toHaveValue("");
  });

  it("leaving the field adds it, so clicking Send keeps it", async () => {
    const user = userEvent.setup();
    render(<Stateful />);
    await user.type(screen.getByRole("textbox", { name: "To recipients" }), "sam@harbour.studio");
    await user.click(document.body);
    expect(chips()).toEqual(["sam@harbour.studio"]);
  });

  it("⌘Enter adds it and still reaches Send", async () => {
    const p = props();
    render(<ComposerFields {...p} />);
    const input = screen.getByRole("textbox", { name: "To recipients" });
    await userEvent.setup().type(input, "sam@harbour.studio");
    expect(fireEvent.keyDown(input, { key: "Enter", metaKey: true })).toBe(true);
    expect(p.onChange).toHaveBeenLastCalledWith("to", [expect.objectContaining({ email: "sam@harbour.studio" })]);
  });

  it("a known address adds the contact with their name", async () => {
    render(<Stateful />);
    await typed("JO@elsewhere.org,");
    expect(chips()).toEqual(["Jo March"]);
  });

  it("a highlighted suggestion still wins on Enter", async () => {
    render(<Stateful />);
    await typed("jo@elsewhere.or{Enter}");
    expect(chips()).toEqual(["Jo March"]);
  });

  it("clicking a suggestion over a typed address adds just the suggestion", async () => {
    const user = userEvent.setup();
    render(<Stateful />);
    await user.type(screen.getByRole("textbox", { name: "To recipients" }), "jo@elsewhere.or");
    await user.click(screen.getByRole("option", { name: /Jo March/ }));
    expect(chips()).toEqual(["Jo March"]);
  });

  it("pasting a list adds everyone in it", async () => {
    const user = userEvent.setup();
    render(<Stateful />);
    await user.click(screen.getByRole("textbox", { name: "To recipients" }));
    await user.paste("Lena Vogt <lena@northwind.co>, x@y.io; not-yet");
    expect(chips()).toEqual(["Lena Vogt", "x@y.io"]);
    expect(screen.getByRole("textbox", { name: "To recipients" })).toHaveValue("not-yet");
  });

  it("keeps unfinished addresses from a paste as text", async () => {
    const user = userEvent.setup();
    render(<Stateful />);
    await user.click(screen.getByRole("textbox", { name: "To recipients" }));
    await user.paste("x@y.io, sam@harbour, more");
    expect(chips()).toEqual(["x@y.io"]);
    expect(screen.getByRole("textbox", { name: "To recipients" })).toHaveValue("sam@harbour, more");
  });

  it("an address already in the field is not added twice", async () => {
    render(<Stateful to={[lena]} />);
    await typed(`${lena.email.toUpperCase()},`);
    expect(chips()).toEqual(["Lena Vogt"]);
    expect(screen.getByRole("textbox", { name: "To recipients" })).toHaveValue("");
  });

  it("leaves text that is not a full address alone", async () => {
    const p = props();
    const user = userEvent.setup();
    render(<ComposerFields {...p} />);
    const input = screen.getByRole("textbox", { name: "To recipients" });
    await user.type(input, "sam@harbour,");
    await user.click(document.body);
    expect(fireEvent.keyDown(input, { key: "Enter", metaKey: true })).toBe(true);
    expect(p.onChange).not.toHaveBeenCalled();
    expect(input).toHaveValue("sam@harbour,");
  });
});

describe("Backspace", () => {
  it("removes the last recipient when the field is empty", async () => {
    const user = userEvent.setup();
    render(<Stateful to={[lena, jonas]} />);
    await user.click(screen.getByRole("textbox", { name: "To recipients" }));
    await user.keyboard("{Backspace}");
    expect(chips()).toEqual(["Lena Vogt"]);
    await user.keyboard("{Backspace}");
    expect(chips()).toEqual([]);
  });

  it("deletes text instead of a recipient while there is text", async () => {
    const p = props({ to: [lena] });
    render(<ComposerFields {...p} />);
    const input = screen.getByRole("textbox", { name: "To recipients" });
    await userEvent.setup().type(input, "ab{Backspace}");
    expect(input).toHaveValue("a");
    expect(p.onChange).not.toHaveBeenCalled();
  });

  it("does nothing on an empty field with no recipients", async () => {
    const p = props();
    render(<ComposerFields {...p} />);
    await userEvent.setup().type(screen.getByRole("textbox", { name: "To recipients" }), "{Backspace}");
    expect(p.onChange).not.toHaveBeenCalled();
  });
});
