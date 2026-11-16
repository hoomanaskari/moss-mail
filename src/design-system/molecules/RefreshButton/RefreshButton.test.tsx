import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { RefreshButton } from "./RefreshButton";

describe("RefreshButton", () => {
  it("is an icon with a name, and checks for new mail each time it's clicked", async () => {
    const onRefresh = vi.fn();
    render(<RefreshButton onRefresh={onRefresh} />);
    const button = screen.getByRole("button", { name: "Check for new mail" });
    expect(button).toHaveTextContent("");
    await userEvent.click(button);
    await userEvent.click(button);
    expect(onRefresh).toHaveBeenCalledTimes(2);
  });
});
