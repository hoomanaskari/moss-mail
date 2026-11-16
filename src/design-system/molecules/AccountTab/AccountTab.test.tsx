import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AccountTab } from "./AccountTab";

describe("AccountTab", () => {
  it("shows the account's name, detail, count and letter", () => {
    render(
      <AccountTab
        name="Work"
        detail="maya@northwind.co"
        count={4}
        selected={false}
        account={{ letter: "W", color: "work" }}
        onSelect={() => {}}
      />,
    );
    const tab = screen.getByRole("tab", { name: /Work/ });
    expect(tab).toHaveAttribute("aria-selected", "false");
    expect(tab).toHaveTextContent("maya@northwind.co");
    expect(tab).toHaveTextContent("4");
    expect(screen.getByText("W")).toBeInTheDocument();
    expect(tab.querySelector("svg")).toBeNull();
    expect(tab.style.outlineColor).toBe("");
  });

  it("says when the account isn't syncing, in place of its address", () => {
    render(
      <AccountTab
        name="Personal"
        detail="maya.ross@fastmail.com"
        problem="Not syncing"
        count={3}
        selected={false}
        account={{ letter: "P", color: "work" }}
        onSelect={() => {}}
      />,
    );
    const tab = screen.getByRole("tab", { name: /Personal/ });
    expect(tab).toHaveTextContent("Not syncing");
    expect(tab).not.toHaveTextContent("maya.ross@fastmail.com");
  });

  it("outlines a selected account in its colour", () => {
    render(
      <AccountTab
        name="Personal"
        detail="maya.ross@fastmail.com"
        count={2}
        selected
        account={{ letter: "P", color: "personal" }}
        onSelect={() => {}}
      />,
    );
    const tab = screen.getByRole("tab", { name: /Personal/ });
    expect(tab).toHaveAttribute("aria-selected", "true");
    expect(tab.style.outlineColor).toBe("var(--personal)");
  });

  it("the All accounts tab shows an inbox icon and no colour outline", () => {
    render(<AccountTab name="All accounts" detail="2 inboxes" count={6} selected onSelect={() => {}} />);
    const tab = screen.getByRole("tab", { name: /All accounts/ });
    expect(tab.querySelector("svg")).not.toBeNull();
    expect(tab.style.outlineColor).toBe("");
  });

  it("calls onSelect when clicked", async () => {
    const onSelect = vi.fn();
    render(<AccountTab name="All accounts" detail="" count={0} selected={false} onSelect={onSelect} />);
    await userEvent.click(screen.getByRole("tab"));
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it("while shortcuts are shown, its key takes the address's place", () => {
    render(
      <AccountTab name="Work" detail="maya@northwind.co" count={4} shortcut="⌘1" showShortcut selected={false} onSelect={() => {}} />,
    );
    expect(screen.getByRole("tab", { name: /Work/ })).toHaveTextContent("Ctrl+1");
    expect(screen.getByText("maya@northwind.co")).not.toBeVisible();
    expect(screen.getByText("4")).toBeVisible();
  });

  it("the key covers a sync problem too, which comes back after", () => {
    const tab = (show: boolean) => (
      <AccountTab
        name="Work"
        detail="maya@northwind.co"
        problem="Not syncing"
        count={4}
        shortcut="⌘1"
        showShortcut={show}
        selected={false}
        onSelect={() => {}}
      />
    );
    const { rerender } = render(tab(true));
    expect(screen.getByText("Not syncing")).not.toBeVisible();
    rerender(tab(false));
    expect(screen.getByText("Not syncing")).toBeVisible();
  });

  it("otherwise shows the address and keeps its key out of sight", () => {
    render(<AccountTab name="Work" detail="maya@northwind.co" count={4} shortcut="⌘1" selected={false} onSelect={() => {}} />);
    expect(screen.getByRole("tab", { name: /Work/ })).not.toHaveTextContent("Ctrl+1");
    expect(screen.getByText("maya@northwind.co")).toBeVisible();
  });

  it("a tab with no key keeps its address even while shortcuts are shown", () => {
    render(<AccountTab name="Work" detail="maya@northwind.co" count={4} showShortcut selected={false} onSelect={() => {}} />);
    expect(screen.getByText("maya@northwind.co")).toBeVisible();
    expect(screen.getByRole("tab").querySelector("kbd")).toBeNull();
  });
});
