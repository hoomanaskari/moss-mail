import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Check } from "lucide-react";
import { describe, expect, it, vi } from "vitest";
import { Toast } from "./Toast";

describe("Toast", () => {
  it("says what happened, politely, without taking the focus", () => {
    render(<Toast icon={Check}>Moved to Done</Toast>);
    expect(screen.getByRole("status")).toHaveTextContent("Moved to Done");
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("offers an action with its key", async () => {
    const onClick = vi.fn();
    render(<Toast action={{ label: "Undo", keys: "⌘Z", onClick }}>Moved to Done</Toast>);
    await userEvent.click(screen.getByRole("button", { name: "UndoCtrl+Z" }));
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("can lead with something other than an icon, such as a countdown", () => {
    render(<Toast lead={<span data-testid="ring" />}>Sending in 6s</Toast>);
    expect(screen.getByRole("status")).toContainElement(screen.getByTestId("ring"));
  });
});
