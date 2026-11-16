import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AutocompleteOption } from "./AutocompleteOption";

function setup(highlighted: boolean, external = false) {
  const onChoose = vi.fn();
  render(
    <AutocompleteOption
      name="Lena Vogt"
      email="lena@northwind.co"
      initials="LV"
      external={external}
      highlighted={highlighted}
      onChoose={onChoose}
    />,
  );
  return { option: screen.getByRole("option", { name: /Lena Vogt/ }), onChoose };
}

describe("AutocompleteOption", () => {
  it("shows the person's name, email and initials", () => {
    const { option } = setup(false, true);
    expect(option).toHaveTextContent("lena@northwind.co");
    expect(screen.getByText("LV")).toBeInTheDocument();
  });

  it("a highlighted option is selected and shows the Enter key", () => {
    const { option } = setup(true);
    expect(option).toHaveAttribute("aria-selected", "true");
    expect(option.querySelector("kbd")).toHaveTextContent("↵");
  });

  it("a plain option has no key cap", () => {
    const { option } = setup(false);
    expect(option).toHaveAttribute("aria-selected", "false");
    expect(option.querySelector("kbd")).toBeNull();
  });

  it("chooses on mouse down without stealing focus from the input", () => {
    const { option, onChoose } = setup(false);
    const event = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    fireEvent(option, event);
    expect(onChoose).toHaveBeenCalledOnce();
    expect(event.defaultPrevented).toBe(true);
  });

  it("moving the mouse over it neither highlights nor chooses it", async () => {
    const { option, onChoose } = setup(false);
    await userEvent.hover(option);
    await userEvent.pointer({ target: option, coords: { x: 1, y: 1 } });
    expect(option).toHaveAttribute("aria-selected", "false");
    expect(onChoose).not.toHaveBeenCalled();
  });
});
