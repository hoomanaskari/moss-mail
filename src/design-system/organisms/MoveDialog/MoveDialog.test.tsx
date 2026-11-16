import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { splits } from "../../../data/demo";
import type { SplitId } from "../../../data/types";
import { MoveDialog } from "./MoveDialog";

function setup(current: SplitId = "receipts", moved = false) {
  const p = { subject: "Your receipt", current, moved, onPick: vi.fn(), onClose: vi.fn() };
  render(<MoveDialog splits={splits} {...p} />);
  return { p, user: userEvent.setup() };
}

const marked = () => screen.getAllByRole("menuitemradio").find((r) => r.getAttribute("data-active") === "true");

describe("MoveDialog", () => {
  it("asks which tab the email it names goes to, with a key for each, and says where it is now", () => {
    setup();
    const dialog = screen.getByRole("dialog", { name: "Move to" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog).toHaveTextContent("Your receipt");
    const rows = screen.getAllByRole("menuitemradio");
    expect(rows.map((r) => r.textContent)).toEqual(["Important1", "Other2", "Calendar3", "Newsletters4", "ReceiptsHere5"]);
    expect(screen.getByRole("menuitemradio", { name: /Receipts/ })).toHaveAttribute("aria-checked", "true");
    expect(marked()).toHaveTextContent("Important");
  });

  it("↵ moves it to Important straight away; ↓ ↵ picks another", async () => {
    const { p, user } = setup();
    await user.keyboard("{Enter}");
    expect(p.onPick).toHaveBeenLastCalledWith("important");
    await user.keyboard("{ArrowDown}{Enter}");
    expect(p.onPick).toHaveBeenLastCalledWith("other");
  });

  it("from Important, ↵ takes it out to Other", async () => {
    const { p, user } = setup("important");
    expect(marked()).toHaveTextContent("Other");
    await user.keyboard("{Enter}");
    expect(p.onPick).toHaveBeenCalledWith("other");
  });

  it("a number picks its tab, and the key goes no further", async () => {
    const outer = vi.fn();
    window.addEventListener("keydown", outer);
    const { p, user } = setup();
    await user.keyboard("4");
    expect(p.onPick).toHaveBeenCalledWith("newsletters");
    await user.keyboard("9");
    expect(p.onPick).toHaveBeenCalledTimes(1);
    expect(outer).toHaveBeenCalledTimes(1);
    window.removeEventListener("keydown", outer);
  });

  it("one you placed yourself can go back to Moss's sorting, with 0", async () => {
    const { p, user } = setup("other", true);
    expect(screen.getByRole("menuitem", { name: "Let Moss sort it0" })).toBeInTheDocument();
    await user.keyboard("0");
    expect(p.onPick).toHaveBeenCalledWith(null);
    await user.click(screen.getByRole("menuitem", { name: "Let Moss sort it0" }));
    expect(p.onPick).toHaveBeenCalledTimes(2);
  });

  it("↑ from the top marks Let Moss sort it, and ↵ picks it", async () => {
    const { p, user } = setup("other", true);
    await user.keyboard("{ArrowUp}");
    expect(marked()).toBeUndefined();
    expect(screen.getByRole("menuitem")).toHaveAttribute("data-active", "true");
    await user.keyboard("{Enter}");
    expect(p.onPick).toHaveBeenCalledWith(null);
  });

  it("leaves ⌘ keys to the app", async () => {
    const outer = vi.fn();
    window.addEventListener("keydown", outer);
    const { p, user } = setup();
    await user.keyboard("{Meta>}1{/Meta}");
    expect(p.onPick).not.toHaveBeenCalled();
    expect(outer).toHaveBeenCalled();
    window.removeEventListener("keydown", outer);
  });

  it("0 does nothing while Moss sorts it", async () => {
    const { p, user } = setup();
    expect(screen.queryByRole("menuitem")).not.toBeInTheDocument();
    await user.keyboard("0");
    expect(p.onPick).not.toHaveBeenCalled();
  });

  it("a click picks a tab; Esc or a click beside it closes", async () => {
    const outer = vi.fn();
    window.addEventListener("keydown", outer);
    const { p, user } = setup();
    await user.click(screen.getByRole("menuitemradio", { name: /Calendar/ }));
    expect(p.onPick).toHaveBeenCalledWith("calendar");
    await user.keyboard("{Escape}");
    expect(p.onClose).toHaveBeenCalledOnce();
    expect(outer).not.toHaveBeenCalled();
    await user.click(screen.getByTestId("move-scrim"));
    expect(p.onClose).toHaveBeenCalledTimes(2);
    window.removeEventListener("keydown", outer);
  });

  it("an email whose tab isn't known marks none as now", () => {
    render(<MoveDialog splits={splits} onPick={vi.fn()} onClose={vi.fn()} />);
    expect(screen.getAllByRole("menuitemradio").every((r) => r.getAttribute("aria-checked") === "false")).toBe(true);
    expect(screen.getByRole("dialog")).not.toHaveTextContent("Your receipt");
  });
});
