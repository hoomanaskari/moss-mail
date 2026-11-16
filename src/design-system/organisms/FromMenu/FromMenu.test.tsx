import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { Account } from "../../../data/types";
import { work } from "../../../test/fixtures";
import { digitFroms, pressDigit } from "../../../test/keys";
import { FromMenu } from "./FromMenu";

const icloud: Account = {
  id: "icloud",
  name: "iCloud",
  letter: "I",
  address: "maya@icloud.com",
  senderName: "Maya Ross",
  color: "sky",
  addresses: [
    { email: "hi@rossstudio.art", name: "Ross Studio", state: "added", source: "you", seen: 0, blocked: false },
    { email: "shop@mayaross.me", state: "suggested", source: "mail", seen: 3, blocked: false },
  ],
};

function setup(chosen = { accountId: "icloud", email: "hi@rossstudio.art" }) {
  const onChoose = vi.fn();
  const onClose = vi.fn();
  const onEdit = vi.fn();
  render(<FromMenu accounts={[work, icloud]} chosen={chosen} onChoose={onChoose} onClose={onClose} onEdit={onEdit} />);
  return { onChoose, onClose, onEdit };
}

describe("FromMenu", () => {
  it("lists every address you can send from, by account, numbered, with the chosen one picked", () => {
    setup();
    const group = screen.getByRole("radiogroup", { name: "Send from" });
    expect(group).toHaveTextContent("Work");
    expect(group).toHaveTextContent("iCloud");
    const options = screen.getAllByRole("radio");
    expect(options.map((o) => o.closest("label")!.textContent)).toEqual([
      "maya@northwind.coMaya Ross1",
      "maya@icloud.comMaya Ross2",
      "hi@rossstudio.artRoss Studio3",
    ]);
    expect(screen.getByRole("radio", { name: /hi@rossstudio\.art/ })).toBeChecked();
    expect(screen.queryByText("shop@mayaross.me")).not.toBeInTheDocument();
  });

  it("a click or a number picks an address", async () => {
    const { onChoose } = setup();
    await userEvent.click(screen.getByRole("radio", { name: /maya@icloud\.com/ }));
    expect(onChoose).toHaveBeenLastCalledWith("icloud", "maya@icloud.com");
    fireEvent.keyDown(window, { key: "1" });
    expect(onChoose).toHaveBeenLastCalledWith("work", "maya@northwind.co");
  });

  it.each(digitFroms)("a number from the %s keys picks, with or without ⌥ still held from ⌥F", (from) => {
    const { onChoose } = setup();
    expect(pressDigit(2, from)).toBe(false);
    expect(onChoose).toHaveBeenLastCalledWith("icloud", "maya@icloud.com");
    expect(pressDigit(1, from, { alt: true })).toBe(false);
    expect(onChoose).toHaveBeenLastCalledWith("work", "maya@northwind.co");
    // 4 is past the last address.
    expect(pressDigit(4, from)).toBe(true);
    expect(onChoose).toHaveBeenCalledTimes(2);
  });

  it("↑ ↓ ↵ pick from the address it goes from now, and no key reaches the app", async () => {
    const outer = vi.fn();
    window.addEventListener("keydown", outer);
    const { onChoose } = setup();
    const user = userEvent.setup();
    await user.keyboard("{ArrowDown}{Enter}");
    expect(onChoose).toHaveBeenLastCalledWith("work", "maya@northwind.co");
    await user.keyboard("{ArrowUp}{Enter}");
    expect(onChoose).toHaveBeenLastCalledWith("icloud", "hi@rossstudio.art");
    // Hovering never moves the mark.
    await user.hover(screen.getByRole("radio", { name: /maya@icloud\.com/ }).closest("label")!);
    await user.keyboard("{Enter}");
    expect(onChoose).toHaveBeenLastCalledWith("icloud", "hi@rossstudio.art");
    expect(outer).not.toHaveBeenCalled();
    window.removeEventListener("keydown", outer);
  });

  it("Esc closes it, and other keys pass through", () => {
    const { onClose, onChoose } = setup();
    fireEvent.keyDown(window, { key: "9" });
    fireEvent.keyDown(window, { key: "x" });
    expect(onChoose).not.toHaveBeenCalled();
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("goes to Settings to add more addresses", async () => {
    const { onEdit } = setup();
    await userEvent.click(screen.getByRole("button", { name: /Add an address/ }));
    expect(onEdit).toHaveBeenCalledOnce();
  });

  it("picks an account's own address when that is the one it goes from", () => {
    setup({ accountId: "work", email: "maya@northwind.co" });
    expect(screen.getByRole("radio", { name: /maya@northwind\.co/ })).toBeChecked();
  });

  it("numbers only the first nine addresses", () => {
    const many = { ...icloud, addresses: Array.from({ length: 9 }, (_, i) => ({ email: `a${i}@x.io`, state: "added" as const, source: "you" as const, seen: 0, blocked: false })) };
    render(<FromMenu accounts={[many]} chosen={{ accountId: "icloud", email: "maya@icloud.com" }} onChoose={vi.fn()} onClose={vi.fn()} onEdit={vi.fn()} />);
    expect(screen.getByRole("radio", { name: /a8@x\.io/ }).closest("label")).toHaveTextContent(/^a8@x\.ioMaya Ross$/);
  });
});
