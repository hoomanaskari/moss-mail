import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { RemindDialog } from "./RemindDialog";

const tuesday = new Date(2026, 10, 3, 13, 20).getTime();

function setup() {
  const p = { subject: "Q4 roadmap", onPick: vi.fn(), onClose: vi.fn() };
  render(<RemindDialog {...p} now={tuesday} />);
  return { p, user: userEvent.setup() };
}

describe("RemindDialog", () => {
  it("asks when to bring back the email it names", () => {
    setup();
    const dialog = screen.getByRole("dialog", { name: "Remind me" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog).toHaveTextContent("Q4 roadmap");
    expect(screen.getByRole("button", { name: "TomorrowWed 8 AM" })).toBeInTheDocument();
  });

  it("takes the keys from where you were typing, gives them back once closed, and can ask about sending later", async () => {
    render(<input aria-label="Body" autoFocus />);
    const onPick = vi.fn();
    const { unmount } = render(<RemindDialog title="Send later" action="Send" subject="Lunch?" onPick={onPick} onClose={vi.fn()} now={tuesday} />);
    const dialog = screen.getByRole("dialog", { name: "Send later" });
    expect(dialog).toHaveFocus();
    expect(dialog).toHaveTextContent("Lunch?");
    await userEvent.click(screen.getByRole("button", { name: "Pick a date and time…" }));
    expect(screen.getByRole("button", { name: "Send Wed 8 AM" })).toBeInTheDocument();
    unmount();
    expect(screen.getByLabelText("Body")).toHaveFocus();
  });

  it("passes on the time you pick", async () => {
    const { p, user } = setup();
    await user.click(screen.getByRole("button", { name: "Next weekMon 8 AM" }));
    expect(p.onPick).toHaveBeenCalledWith(new Date(2026, 10, 9, 8, 0).getTime());
  });

  it("closes with Esc, without the key reaching the app", async () => {
    const outer = vi.fn();
    window.addEventListener("keydown", outer);
    const { p, user } = setup();
    await user.keyboard("{Escape}");
    expect(p.onClose).toHaveBeenCalledOnce();
    expect(outer).not.toHaveBeenCalled();
    window.removeEventListener("keydown", outer);
  });

  it("lets other keys through, and closes with a click beside it", async () => {
    const outer = vi.fn();
    window.addEventListener("keydown", outer);
    const { p, user } = setup();
    await user.keyboard("e");
    expect(outer).toHaveBeenCalled();
    expect(p.onClose).not.toHaveBeenCalled();
    await user.click(screen.getByTestId("remind-scrim"));
    expect(p.onClose).toHaveBeenCalledOnce();
    window.removeEventListener("keydown", outer);
  });
});
