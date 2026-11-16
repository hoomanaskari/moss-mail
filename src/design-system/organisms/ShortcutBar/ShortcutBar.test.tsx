import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ShortcutBar } from "./ShortcutBar";

describe("ShortcutBar", () => {
  it("shows the main keys with what they do, in order", () => {
    render(<ShortcutBar />);
    const bar = screen.getByRole("contentinfo");
    const keys = [...bar.querySelectorAll("kbd")].map((k) => k.textContent);
    expect(keys).toEqual(["↵", "E", "H", "C", "/", "Ctrl+J"]);
    for (const label of ["Open", "Done", "Remind me", "New email", "Search", "Ask Moss"]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });

  it("leaves out Move (the arrows need no reminder) and Ask AI (the top bar has its button)", () => {
    render(<ShortcutBar />);
    expect(screen.queryByText("Move")).not.toBeInTheDocument();
    expect(screen.queryByText("Ask AI")).not.toBeInTheDocument();
    expect(screen.getByRole("contentinfo")).not.toHaveTextContent(/J K|Ctrl\+K/);
  });

  it("shows a screen's own keys instead when given them", () => {
    render(<ShortcutBar hints={[{ keys: "Esc", label: "Close" }]} />);
    expect(screen.getByRole("contentinfo")).toHaveTextContent("EscClose");
    expect(screen.queryByText("Move")).not.toBeInTheDocument();
  });

  it("is a reminder only, with nothing to click", () => {
    render(<ShortcutBar />);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});
