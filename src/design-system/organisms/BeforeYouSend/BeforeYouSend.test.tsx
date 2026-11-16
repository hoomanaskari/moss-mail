import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { BeforeYouSend } from "./BeforeYouSend";

describe("BeforeYouSend", () => {
  it("shows only who the email goes to when nothing is missing", () => {
    render(<BeforeYouSend audience="Goes to 4 people and 1 BCC." onAttach={vi.fn()} />);
    expect(screen.getByText("Goes to 4 people and 1 BCC.")).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Attach file" })).not.toBeInTheDocument();
  });

  it("warns about a missing attachment and offers to attach one", async () => {
    const onAttach = vi.fn();
    render(<BeforeYouSend audience="Goes to Lena." missingAttachment="You mention notes, but nothing is attached." onAttach={onAttach} />);
    expect(screen.getByRole("alert")).toHaveTextContent("You mention notes, but nothing is attached.");
    expect(screen.getByText("Goes to Lena.")).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: "Attach file" }));
    expect(onAttach).toHaveBeenCalledOnce();
  });
});
