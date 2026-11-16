import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { RecipientChip } from "./RecipientChip";

describe("RecipientChip", () => {
  it("shows the person and a remove button named after them", async () => {
    const onRemove = vi.fn();
    render(<RecipientChip name="Lena Vogt" initials="LV" onRemove={onRemove} />);
    expect(screen.getByText("Lena Vogt")).toBeInTheDocument();
    expect(screen.getByText("LV")).toBeInTheDocument();
    expect(screen.queryByText("· outside")).toBeNull();
    const remove = screen.getByRole("button", { name: "Remove Lena Vogt" });
    expect(remove.querySelector("svg")).toHaveAttribute("stroke", "var(--ink-3)");
    await userEvent.click(remove);
    expect(onRemove).toHaveBeenCalledOnce();
  });

  it("marks someone outside your organisation", () => {
    render(<RecipientChip name="Ines" initials="I" external />);
    expect(screen.getByText("· outside")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove Ines" }).querySelector("svg")).toHaveAttribute(
      "stroke",
      "var(--due)",
    );
  });
});
