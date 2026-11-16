import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { RotateCcw } from "lucide-react";
import { describe, expect, it, vi } from "vitest";
import { RemindMenu } from "./RemindMenu";

const at = (...a: [number, number, number, number, number?]) => new Date(...a).getTime();
const tuesday = at(2026, 10, 3, 13, 20); // Tuesday 3 November 2026, 1:20 PM

function setup(now = tuesday) {
  const onPick = vi.fn();
  const noReply = vi.fn();
  const extras = [{ icon: RotateCcw, label: "Remind me if no reply", keys: "⇧H", onClick: noReply }];
  render(<RemindMenu now={now} onPick={onPick} extras={extras} />);
  return { onPick, noReply, user: userEvent.setup() };
}

describe("RemindMenu", () => {
  it("offers the ready-made times, each with when it comes back, then picking your own", () => {
    setup();
    expect(screen.getAllByRole("button").map((b) => b.textContent)).toEqual([
      "Later today5 PM",
      "TomorrowWed 8 AM",
      "This weekendSat 8 AM",
      "Next weekMon 8 AM",
      "Pick a date and time…",
      "Remind me if no replyShift+H",
    ]);
  });

  it("picks a time with a click", async () => {
    const { onPick, user } = setup();
    await user.click(screen.getByRole("button", { name: /This weekend/ }));
    expect(onPick).toHaveBeenCalledWith(at(2026, 10, 7, 8, 0));
  });

  it("marks Tomorrow first, so H then ↵ is what H used to do", async () => {
    const { onPick, user } = setup();
    await user.keyboard("{Enter}");
    expect(onPick).toHaveBeenCalledWith(at(2026, 10, 4, 8, 0));
  });

  it("↑ ↓ move the mark, and ↵ picks it", async () => {
    const { onPick, user } = setup();
    await user.keyboard("{ArrowUp}{Enter}");
    expect(onPick).toHaveBeenCalledWith(at(2026, 10, 3, 17, 0));
  });

  it("runs the rows under the times", async () => {
    const { noReply, onPick, user } = setup();
    await user.click(screen.getByRole("button", { name: /if no reply/ }));
    expect(noReply).toHaveBeenCalledOnce();
    expect(onPick).not.toHaveBeenCalled();
  });

  describe("picking your own", () => {
    async function openPicker(now = tuesday) {
      const r = setup(now);
      await r.user.click(screen.getByRole("button", { name: "Pick a date and time…" }));
      return r;
    }

    it("starts at tomorrow 8 AM and reminds you then", async () => {
      const { onPick, user } = await openPicker();
      expect(screen.getByLabelText("Date")).toHaveValue("2026-11-04");
      expect(screen.getByLabelText("Time")).toHaveValue("08:00");
      expect(screen.getByLabelText("Date")).toHaveFocus();
      await user.click(screen.getByRole("button", { name: "Remind me Wed 8 AM" }));
      expect(onPick).toHaveBeenCalledWith(at(2026, 10, 4, 8, 0));
    });

    it("reminds you at the date and time you set, with ↵ too", async () => {
      const { onPick, user } = await openPicker();
      fireEvent.change(screen.getByLabelText("Date"), { target: { value: "2026-12-24" } });
      fireEvent.change(screen.getByLabelText("Time"), { target: { value: "09:30" } });
      expect(screen.getByRole("button", { name: "Remind me Thu 24 Dec, 9:30 AM" })).toBeEnabled();
      await user.click(screen.getByLabelText("Time"));
      await user.keyboard("{Enter}");
      expect(onPick).toHaveBeenCalledWith(at(2026, 11, 24, 9, 30));
    });

    it("won't remind you at a time that has gone", async () => {
      const { onPick, user } = await openPicker();
      fireEvent.change(screen.getByLabelText("Date"), { target: { value: "2026-11-03" } });
      fireEvent.change(screen.getByLabelText("Time"), { target: { value: "13:00" } });
      expect(screen.getByText("That time has passed.")).toBeInTheDocument();
      const remind = screen.getByRole("button", { name: "Remind me" });
      expect(remind).toBeDisabled();
      await user.keyboard("{Enter}");
      fireEvent.submit(screen.getByRole("form", { name: "Pick a date and time" }));
      expect(onPick).not.toHaveBeenCalled();
    });

    it("needs both a date and a time", async () => {
      await openPicker();
      fireEvent.change(screen.getByLabelText("Time"), { target: { value: "" } });
      expect(screen.getByRole("button", { name: "Remind me" })).toBeDisabled();
      expect(screen.queryByText("That time has passed.")).not.toBeInTheDocument();
    });

    it("Back returns to the ready-made times", async () => {
      const { user } = await openPicker();
      await user.click(screen.getByRole("button", { name: "Back" }));
      expect(screen.getByRole("button", { name: /Tomorrow/ })).toBeInTheDocument();
      expect(screen.queryByLabelText("Date")).not.toBeInTheDocument();
    });

    it("says what picking does, e.g. Send", async () => {
      const onPick = vi.fn();
      render(<RemindMenu now={tuesday} onPick={onPick} action="Send" />);
      await userEvent.click(screen.getByRole("button", { name: "Pick a date and time…" }));
      await userEvent.click(screen.getByRole("button", { name: "Send Wed 8 AM" }));
      expect(onPick).toHaveBeenCalledWith(at(2026, 10, 4, 8, 0));
      fireEvent.change(screen.getByLabelText("Time"), { target: { value: "" } });
      expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    });

    it("leaves ↑ ↓ to the date and time fields", async () => {
      const { onPick, user } = await openPicker();
      await user.keyboard("{ArrowDown}");
      expect(onPick).not.toHaveBeenCalled();
      expect(screen.getByLabelText("Date")).toBeInTheDocument();
    });
  });
});
