import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { platformKeys } from "../../atoms/Kbd/Kbd";
import { CommandHint } from "./CommandHint";

describe("CommandHint", () => {
  it("shows the palette shortcut and opens the palette", async () => {
    const onOpen = vi.fn();
    render(<CommandHint onOpen={onOpen} />);
    const button = screen.getByRole("button", { name: /Ask AI or run a command/ });
    expect(button.querySelector("kbd")).toHaveTextContent(platformKeys("⌘K"));
    await userEvent.click(button);
    expect(onOpen).toHaveBeenCalledOnce();
  });
});
