import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ShortcutHint } from "./ShortcutHint";

describe("ShortcutHint", () => {
  it("is plain text without onClick, key first, 8px gap", () => {
    const { container } = render(<ShortcutHint keys="E" label="Done" />);
    expect(screen.queryByRole("button")).toBeNull();
    const hint = container.firstElementChild as HTMLElement;
    expect(hint.tagName).toBe("SPAN");
    expect(hint).not.toHaveAttribute("type");
    expect(hint.style.gap).toBe("8px");
    expect(hint.firstElementChild?.tagName).toBe("KBD");
    expect(hint.textContent).toBe("EDone");
  });

  it("can put the label before the key", () => {
    const { container } = render(<ShortcutHint keys="I" label="Hide" labelFirst gap={6} size="sm" tone="ai" className="extra" />);
    const hint = container.firstElementChild as HTMLElement;
    expect(hint.textContent).toBe("HideI");
    expect(hint.lastElementChild?.tagName).toBe("KBD");
    expect(hint.style.gap).toBe("6px");
    expect(hint).toHaveClass("extra");
  });

  it("is a button when it has onClick", async () => {
    const onClick = vi.fn();
    render(<ShortcutHint keys="R" label="Reply" onClick={onClick} />);
    const button = screen.getByRole("button", { name: /Reply/ });
    expect(button).toHaveAttribute("type", "button");
    await userEvent.click(button);
    expect(onClick).toHaveBeenCalledOnce();
  });
});
