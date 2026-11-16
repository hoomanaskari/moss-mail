import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ScreenErrorBoundary } from "./ScreenErrorBoundary";

let shouldThrow = false;
function Screen() {
  if (shouldThrow) throw new Error("boom");
  return <p>Inbox screen</p>;
}

describe("ScreenErrorBoundary", () => {
  it("renders its children when nothing goes wrong", () => {
    shouldThrow = false;
    render(
      <ScreenErrorBoundary onRecover={() => {}}>
        <Screen />
      </ScreenErrorBoundary>,
    );
    expect(screen.getByText("Inbox screen")).toBeInTheDocument();
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("shows a calm message and logs the error when a screen crashes", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      shouldThrow = true;
      render(
        <ScreenErrorBoundary onRecover={() => {}}>
          <Screen />
        </ScreenErrorBoundary>,
      );
      expect(screen.getByRole("status")).toHaveTextContent("Something went wrong on this screen");
      expect(screen.getByText("Your emails and drafts are safe. Go back to the inbox to carry on.")).toBeInTheDocument();
      expect(screen.queryByText("Inbox screen")).not.toBeInTheDocument();
      expect(error).toHaveBeenCalledWith("Moss screen error:", expect.objectContaining({ message: "boom" }));
    } finally {
      error.mockRestore();
    }
  });

  it("Back to inbox calls onRecover and shows the children again", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    try {
      const user = userEvent.setup();
      shouldThrow = true;
      const onRecover = vi.fn(() => {
        shouldThrow = false;
      });
      render(
        <ScreenErrorBoundary onRecover={onRecover}>
          <Screen />
        </ScreenErrorBoundary>,
      );
      await user.click(screen.getByRole("button", { name: /Back to inbox/ }));
      expect(onRecover).toHaveBeenCalledOnce();
      expect(screen.getByText("Inbox screen")).toBeInTheDocument();
      expect(screen.queryByRole("status")).not.toBeInTheDocument();
    } finally {
      error.mockRestore();
    }
  });
});
