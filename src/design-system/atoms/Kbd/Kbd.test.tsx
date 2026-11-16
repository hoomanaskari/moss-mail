import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Kbd, platformKeys } from "./Kbd";

// isMac is read once at import time; a getter lets each test pick the platform.
const platform = vi.hoisted(() => ({ isMac: true }));
vi.mock("../../../lib/platform", () => ({
  get isMac() {
    return platform.isMac;
  },
}));

beforeEach(() => {
  platform.isMac = true;
});

describe("platformKeys", () => {
  it("keeps Mac symbols on a Mac", () => {
    expect(platformKeys("⌘⇧K")).toBe("⌘⇧K");
  });

  it("spells modifiers out elsewhere", () => {
    platform.isMac = false;
    expect(platformKeys("⌘K")).toBe("Ctrl+K");
    expect(platformKeys("⌥⇧B")).toBe("Alt+Shift+B");
    expect(platformKeys("E")).toBe("E");
    expect(platformKeys("⌫")).toBe("Backspace");
  });
});

describe("Kbd", () => {
  it("renders a key cap", () => {
    render(<Kbd>E</Kbd>);
    const kbd = screen.getByText("E");
    expect(kbd.tagName).toBe("KBD");
  });

  it("translates string shortcuts for the platform", () => {
    platform.isMac = false;
    render(<Kbd tone="quiet">⌘↵</Kbd>);
    expect(screen.getByText("Ctrl+↵")).toBeInTheDocument();
  });

  it("leaves non-string children alone", () => {
    platform.isMac = false;
    render(
      <Kbd className="extra">
        <span>⌘</span>
      </Kbd>,
    );
    const inner = screen.getByText("⌘");
    expect(inner.tagName).toBe("SPAN");
    expect(inner.parentElement).toHaveClass("extra");
  });
});
