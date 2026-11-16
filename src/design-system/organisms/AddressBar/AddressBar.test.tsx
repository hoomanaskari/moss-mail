import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AddressBar } from "./AddressBar";

const addresses = [
  { email: "maya@icloud.com", count: 3 },
  { email: "hi@rossstudio.art", count: 0 },
];

describe("AddressBar", () => {
  it("offers all of the account's addresses, then each one with its count and key", () => {
    render(<AddressBar addresses={addresses} total={3} onSelect={vi.fn()} />);
    const tabs = screen.getAllByRole("tab");
    expect(tabs.map((t) => t.textContent)).toEqual(["All addresses3Alt+0", "maya@icloud.com3Alt+1", "hi@rossstudio.artAlt+2"]);
    expect(screen.getByRole("tab", { name: /All addresses/ })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tablist", { name: "Addresses" })).toBeInTheDocument();
  });

  it("marks the one chosen, and picking one or all reports it", async () => {
    const onSelect = vi.fn();
    render(<AddressBar addresses={addresses} total={3} selected="hi@rossstudio.art" onSelect={onSelect} />);
    expect(screen.getByRole("tab", { name: /hi@rossstudio/ })).toHaveAttribute("aria-selected", "true");
    await userEvent.click(screen.getByRole("tab", { name: /maya@icloud/ }));
    expect(onSelect).toHaveBeenLastCalledWith("maya@icloud.com");
    await userEvent.click(screen.getByRole("tab", { name: /All addresses/ }));
    expect(onSelect).toHaveBeenLastCalledWith(undefined);
  });

  it("numbers only the first nine", () => {
    const many = Array.from({ length: 10 }, (_, i) => ({ email: `a${i}@x.io`, count: 1 }));
    render(<AddressBar addresses={many} total={10} onSelect={vi.fn()} />);
    expect(screen.getByRole("tab", { name: /a9@x\.io/ })).toHaveTextContent(/^a9@x\.io1$/);
  });
});
