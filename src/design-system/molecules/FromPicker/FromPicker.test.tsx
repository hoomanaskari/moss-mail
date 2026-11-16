import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { FromPicker } from "./FromPicker";

const account = { letter: "W", color: "work" } as const;

describe("FromPicker", () => {
  it("inline variant has a From label", () => {
    render(<FromPicker account={account} text="maya@northwind.co" variant="inline" />);
    const button = screen.getByRole("button");
    expect(button).toHaveTextContent(/^FromWmaya@northwind\.co$/);
  });

  it("field variant shows just the badge and address", () => {
    render(<FromPicker account={account} text="Maya Ross · maya@northwind.co" variant="field" />);
    const button = screen.getByRole("button");
    expect(button).not.toHaveTextContent("From");
    expect(button).toHaveTextContent("Maya Ross · maya@northwind.co");
    expect(screen.getByText("W")).toBeInTheDocument();
  });

  it("shows its shortcut and whether its menu is open", () => {
    render(<FromPicker account={account} text="maya@northwind.co" variant="field" keys="⌥F" expanded />);
    expect(screen.getByRole("button")).toHaveTextContent(/Alt\+F$/);
    expect(screen.getByRole("button")).toHaveAttribute("aria-expanded", "true");
  });

  it("calls onClick", async () => {
    const onClick = vi.fn();
    render(<FromPicker account={account} text="maya@northwind.co" variant="field" onClick={onClick} />);
    await userEvent.click(screen.getByRole("button"));
    expect(onClick).toHaveBeenCalledOnce();
  });
});
