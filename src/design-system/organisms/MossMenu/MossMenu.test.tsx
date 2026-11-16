import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { MossMenu } from "./MossMenu";

function setup(hasText = true, name: string | undefined = "Dana") {
  const handlers = { onPick: vi.fn(), onClose: vi.fn() };
  render(<MossMenu name={name} hasText={hasText} {...handlers} />);
  return { ...handlers, user: userEvent.setup() };
}

const selected = () => screen.getByRole("option", { selected: true }).querySelector("span")?.textContent;

describe("MossMenu", () => {
  it("opens on the note box, Quick reply highlighted, and promises nothing sends", () => {
    setup();
    expect(screen.getByRole("combobox", { name: "Tell Moss what to say" })).toHaveFocus();
    expect(selected()).toBe("Quick reply");
    expect(screen.getByRole("listbox")).toHaveTextContent("Accept what Dana asks");
    expect(screen.getByRole("dialog", { name: "Write with Moss" })).toHaveTextContent("Lands as text you can edit. Nothing sends by itself.");
  });

  it("with a note, ↵ writes from it", async () => {
    const { onPick, user } = setup();
    await user.keyboard("yes to the 17th, keep it short{Enter}");
    expect(onPick).toHaveBeenCalledWith({
      job: { kind: "instructions", instructions: "yes to the 17th, keep it short" },
      lead: "Moss wrote this from your note",
      note: "yes to the 17th, keep it short",
    });
  });

  it("with an empty box, ↑ ↓ pick an action and ↵ runs it", async () => {
    const { onPick, user } = setup();
    await user.keyboard("{ArrowDown}{ArrowDown}");
    expect(selected()).toBe("Say no, kindly");
    await user.keyboard("{ArrowUp}{Enter}");
    expect(onPick).toHaveBeenCalledWith({ job: { kind: "accept" }, lead: "Moss wrote a yes" });
  });

  it("moving stops at the ends", async () => {
    const { user } = setup();
    await user.keyboard("{ArrowUp}");
    expect(selected()).toBe("Quick reply");
    await user.keyboard("{ArrowDown>12/}");
    expect(selected()).toBe("Translate");
  });

  it("every action says what Moss did", async () => {
    const { onPick, user } = setup();
    for (const label of ["Quick reply", "Say no, kindly", "Ask for more time", "Shorter", "Fix spelling and grammar"])
      await user.click(screen.getByRole("option", { name: new RegExp(`^${label}`) }));
    expect(onPick.mock.calls.map(([p]) => p.lead)).toEqual([
      "Moss wrote a quick reply",
      "Moss wrote a kind no",
      "Moss asked for more time",
      "Moss made it shorter",
      "Moss fixed spelling and grammar",
    ]);
    expect(onPick.mock.calls[0][0].job.kind).toBe("instructions");
    expect(onPick.mock.calls.slice(1).map(([p]) => p.job.kind)).toEqual(["decline", "moreTime", "shorter", "fix"]);
  });

  it("Change tone asks which tone", async () => {
    const { onPick, user } = setup();
    await user.click(screen.getByRole("option", { name: /^Change tone/ }));
    expect(screen.getAllByRole("option").map((o) => o.querySelector("span")?.textContent)).toEqual(["Friendlier", "More formal", "More direct"]);
    await user.keyboard("{ArrowDown}{Enter}");
    expect(onPick).toHaveBeenLastCalledWith({ job: { kind: "formal" }, lead: "Moss made it more formal" });
    await user.click(screen.getByRole("option", { name: /^Friendlier/ }));
    expect(onPick).toHaveBeenLastCalledWith({ job: { kind: "friendlier" }, lead: "Moss made it friendlier" });
    await user.click(screen.getByRole("option", { name: /^More direct/ }));
    expect(onPick.mock.lastCall![0].job.kind).toBe("instructions");
    expect(onPick.mock.lastCall![0].lead).toBe("Moss made it more direct");
  });

  it("Translate asks which language: pick one or type your own", async () => {
    const { onPick, user } = setup();
    await user.click(screen.getByRole("option", { name: /^Translate/ }));
    expect(screen.getByRole("combobox", { name: "Into which language?" })).toHaveFocus();
    await user.click(screen.getByRole("option", { name: /^Persian/ }));
    expect(onPick).toHaveBeenLastCalledWith({ job: { kind: "translate", language: "Persian" }, lead: "Moss translated it into Persian" });
    await user.keyboard(" Japanese {Enter}");
    expect(onPick).toHaveBeenLastCalledWith({ job: { kind: "translate", language: "Japanese" }, lead: "Moss translated it into Japanese" });
  });

  it("with nothing written yet, changes are greyed out and skipped", async () => {
    const { onPick, user } = setup(false, "");
    expect(screen.getByRole("option", { name: /^Shorter/ })).toHaveAttribute("aria-disabled", "true");
    expect(screen.getByRole("listbox")).toHaveTextContent("Accept what they ask");
    await user.keyboard("{ArrowDown>9/}");
    expect(selected()).toBe("Ask for more time");
    await user.click(screen.getByRole("option", { name: /^Shorter/ }));
    expect(onPick).not.toHaveBeenCalled();
  });

  it("hovering leaves the highlight where it was; Esc closes", async () => {
    const { onClose, user } = setup();
    await user.hover(screen.getByRole("option", { name: /^Shorter/ }));
    expect(selected()).toBe("Quick reply");
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("other keys type into the box", async () => {
    const { onPick, user } = setup();
    await user.keyboard("{ArrowLeft}a");
    expect(screen.getByRole("combobox")).toHaveValue("a");
    expect(onPick).not.toHaveBeenCalled();
  });
});
