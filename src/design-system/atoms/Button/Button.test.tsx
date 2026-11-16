import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Paperclip } from "lucide-react";
import { describe, expect, it, vi } from "vitest";
import { platformKeys } from "../Kbd/Kbd";
import kbdStyles from "../Kbd/Kbd.module.css";
import { Button, type ButtonVariant } from "./Button";

describe("Button", () => {
  it("is a plain button that fires onClick", async () => {
    const onClick = vi.fn();
    render(
      <Button variant="outline" onClick={onClick}>
        Reply
      </Button>,
    );
    const button = screen.getByRole("button", { name: "Reply" });
    expect(button).toHaveAttribute("type", "button");
    expect(button).not.toHaveAttribute("aria-pressed");
    await userEvent.click(button);
    expect(onClick).toHaveBeenCalledOnce();
  });

  it("passes through type, disabled and other button attributes", async () => {
    const onClick = vi.fn();
    render(
      <Button variant="primary" type="submit" disabled onClick={onClick} aria-label="Send now" size="lg">
        Send
      </Button>,
    );
    const button = screen.getByRole("button", { name: "Send now" });
    expect(button).toHaveAttribute("type", "submit");
    expect(button).toBeDisabled();
    await userEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
  });

  it("an AI action has a teal icon", () => {
    render(
      <Button variant="ai" icon={Paperclip} shortcut="⌘J">
        Change it
      </Button>,
    );
    const button = screen.getByRole("button", { name: `Change it${platformKeys("⌘J")}` });
    expect(button.querySelector("svg")).toHaveAttribute("stroke", "var(--ai)");
  });

  it("choice and toolbar buttons report whether they are pressed", () => {
    render(
      <>
        <Button variant="choice" active>
          Chosen
        </Button>
        <Button variant="choice">Other</Button>
        <Button variant="toolbar">Attach</Button>
      </>,
    );
    expect(screen.getByRole("button", { name: "Chosen" })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getByRole("button", { name: "Other" })).toHaveAttribute("aria-pressed", "false");
    expect(screen.getByRole("button", { name: "Attach" })).toHaveAttribute("aria-pressed", "false");
  });

  it("busy, it shows a spinner instead of its icon and can't be pressed again", async () => {
    const onClick = vi.fn();
    render(
      <Button variant="primary" icon={Paperclip} busy shortcut="⌘↵" onClick={onClick}>
        Sending…
      </Button>,
    );
    const button = screen.getByRole("button", { name: /^Sending…/ });
    expect(button).toHaveAttribute("aria-busy", "true");
    expect(button).toBeDisabled();
    expect(button.querySelector("svg")).not.toHaveClass("lucide-paperclip");
    await userEvent.click(button);
    expect(onClick).not.toHaveBeenCalled();
  });

  it("has no icon or key cap unless given one", () => {
    render(<Button variant="outline">Plain</Button>);
    const button = screen.getByRole("button", { name: "Plain" });
    expect(button.querySelector("svg")).toBeNull();
    expect(button.querySelector("kbd")).toBeNull();
  });

  it.each<[ButtonVariant, boolean, string, string]>([
    ["toolbar", true, "var(--ai)", "16"],
    ["toolbar", false, "var(--ink-2)", "16"],
    ["subtle", false, "var(--ink-3)", "16"],
    ["warn", false, "var(--due)", "16"],
    ["mini", false, "var(--ink-2)", "13"],
    ["outlineSm", false, "var(--ink-2)", "13"],
    ["secondary", false, "var(--ink-2)", "14"],
  ])("%s (active %s) draws its icon in %s at %spx", (variant, active, color, size) => {
    render(
      <Button variant={variant} active={active} icon={Paperclip}>
        Go
      </Button>,
    );
    const svg = screen.getByRole("button").querySelector("svg")!;
    expect(svg.getAttribute("stroke")).toBe(color);
    expect(svg).toHaveAttribute("width", size);
  });

  it("shows its shortcut in a key cap", () => {
    render(
      <Button variant="primary" shortcut="⌘↵">
        Send
      </Button>,
    );
    const kbd = screen.getByRole("button").querySelector("kbd");
    expect(kbd).toHaveTextContent(platformKeys("⌘↵"));
  });

  it.each<[ButtonVariant, "default" | "onAccent" | "onInvert"]>([
    ["primary", "onAccent"],
    ["invert", "onInvert"],
    ["outline", "default"],
  ])("%s picks the %s key cap", (variant, tone) => {
    render(
      <Button variant={variant} shortcut="E">
        Done
      </Button>,
    );
    const kbd = screen.getByRole("button", { name: /Done/ }).querySelector("kbd");
    expect(kbd).toHaveTextContent("E");
    expect(kbd).toHaveClass(kbdStyles[tone]);
  });

  it("a link has no icon even when given one", () => {
    render(
      <Button variant="link" icon={Paperclip} shortcut="⌥A">
        Use your own Google OAuth client
      </Button>,
    );
    expect(screen.getByRole("button").querySelector("svg")).toBeNull();
  });

  it("shortcutTone overrides the variant's key cap", () => {
    render(
      <Button variant="primary" shortcut="E" shortcutTone="quiet">
        Done
      </Button>,
    );
    const kbd = screen.getByRole("button", { name: /Done/ }).querySelector("kbd");
    expect(kbd).toHaveClass(kbdStyles.quiet);
    expect(kbd).not.toHaveClass(kbdStyles.onAccent);
  });
});
