import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { Signature } from "../../../state/signatures";
import { digitFroms, pressDigit } from "../../../test/keys";
import { SignatureBlock, type SignatureBlockProps } from "./SignatureBlock";

const studio: Signature = { id: "studio", name: "Studio", lines: ["Maya Ross", "Ross Studio · Photography", "rossstudio.art"] };
const short: Signature = { id: "short", name: "Work (short)", lines: ["Maya"] };
const full: Signature = { id: "full", name: "Work (full)", lines: ["Maya Ross", "Design lead"] };

function setup(props: Partial<SignatureBlockProps> = {}) {
  const p: SignatureBlockProps = {
    signature: studio,
    options: [studio, short, full],
    reason: "Your signature for hello@rossstudio.art",
    why: "Picked because you’re writing from hello@rossstudio.art.",
    open: false,
    onToggle: vi.fn(),
    onChoose: vi.fn(),
    onEdit: vi.fn(),
    ...props,
  };
  render(<SignatureBlock {...p} />);
  return p;
}

describe("SignatureBlock", () => {
  it("shows the signature under the body, with web addresses as links", () => {
    setup();
    expect(screen.getByText("Maya Ross")).toBeInTheDocument();
    expect(screen.getByText("Ross Studio · Photography")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "rossstudio.art" })).toHaveAttribute("href", "https://rossstudio.art");
  });

  it("keeps a link that already says https", () => {
    setup({ signature: { id: "x", name: "X", lines: ["Maya", "https://maya.dev/work"] } });
    expect(screen.getByRole("link", { name: "https://maya.dev/work" })).toHaveAttribute("href", "https://maya.dev/work");
    expect(screen.getByText("Your signature for hello@rossstudio.art")).toBeInTheDocument();
  });

  it("the chip changes it: it opens the picker, says so, and shows ⌥S", async () => {
    const p = setup();
    const chip = screen.getByRole("button", { name: "Change signature, Studio" });
    expect(chip).toHaveTextContent("StudioAlt+S");
    expect(chip).toHaveAttribute("aria-expanded", "false");
    expect(chip).toHaveAttribute("aria-keyshortcuts", "Alt+S");
    await userEvent.click(chip);
    expect(p.onToggle).toHaveBeenCalledOnce();
    expect(screen.queryByRole("radiogroup")).not.toBeInTheDocument();
  });

  it("Remove clears the signature from this email", async () => {
    const p = setup();
    await userEvent.click(screen.getByRole("button", { name: "Remove signature" }));
    expect(p.onChoose).toHaveBeenCalledWith(null);
    expect(p.onToggle).not.toHaveBeenCalled();
  });

  it("the chip says the picker is open", () => {
    setup({ open: true });
    expect(screen.getByRole("button", { name: "Change signature, Studio" })).toHaveAttribute("aria-expanded", "true");
  });

  it("with no signature, says so plainly and offers to add one with ⌥S", async () => {
    const p = setup({ signature: undefined, reason: "" });
    const row = screen.getByRole("group", { name: "Signature on this email" });
    expect(row).toHaveTextContent("No signature");
    expect(screen.queryByRole("button", { name: "Remove signature" })).not.toBeInTheDocument();
    const add = screen.getByRole("button", { name: "Add signatureAlt+S" });
    expect(add).toHaveAttribute("aria-expanded", "false");
    expect(add).toHaveAttribute("aria-keyshortcuts", "Alt+S");
    await userEvent.click(add);
    expect(p.onToggle).toHaveBeenCalledOnce();
    expect(p.onEdit).not.toHaveBeenCalled();
  });

  it("with no signature and the picker open, Add says it is open", () => {
    setup({ signature: undefined, open: true });
    expect(screen.getByRole("button", { name: "Add signatureAlt+S" })).toHaveAttribute("aria-expanded", "true");
  });

  it("with no saved signatures at all, the control leads to Settings", async () => {
    const p = setup({ signature: undefined, options: [], reason: "" });
    expect(screen.getByRole("group", { name: "Signature on this email" })).toHaveTextContent("No signature");
    expect(screen.queryByRole("button", { name: /^Add signature/ })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Set up a signature" }));
    expect(p.onEdit).toHaveBeenCalledOnce();
    expect(p.onToggle).not.toHaveBeenCalled();
  });

  it("minimal, for a Quick reply: the chip, the lines and the reason, and the chip even with no signature", () => {
    setup({ minimal: true });
    expect(screen.getByRole("group", { name: "Signature" })).toHaveTextContent("Maya Ross");
    expect(screen.getByRole("button", { name: /Studio/ })).toBeInTheDocument();
    expect(screen.getByText("Your signature for hello@rossstudio.art")).toBeInTheDocument();
    document.body.innerHTML = "";
    setup({ minimal: true, signature: undefined, reason: "" });
    expect(screen.getByRole("group", { name: "Signature on this email" })).toHaveTextContent("No signature");
    expect(screen.getByRole("button", { name: "Add signatureAlt+S" })).toBeInTheDocument();
  });

  it("minimal, the picker keeps the choices, the why and Edit signatures", async () => {
    const p = setup({ minimal: true, open: true });
    expect(screen.getAllByRole("radio")).toHaveLength(4);
    expect(screen.getByText(/Picked because/)).toBeInTheDocument();
    await userEvent.setup().click(screen.getByRole("button", { name: /Edit signatures/ }));
    expect(p.onEdit).toHaveBeenCalledOnce();
  });

  it("leaves the why out when there's none to give", () => {
    setup({ open: true, why: "" });
    expect(screen.queryByText("AI")).not.toBeInTheDocument();
  });

  it("with no signature but a reason, says why Moss left it out", () => {
    setup({ signature: undefined, reason: "Left out: quick reply in a back-and-forth" });
    expect(screen.getByRole("group", { name: "Signature on this email" })).toHaveTextContent(
      "No signatureAdd signatureAlt+SLeft out: quick reply in a back-and-forth",
    );
    expect(screen.getByText("Left out: quick reply in a back-and-forth")).toBeInTheDocument();
    expect(screen.queryByRole("group", { name: "Signature" })).not.toBeInTheDocument();
  });

  it("still shows the row with no signature, nothing to say and the picker closed", () => {
    render(<SignatureBlock options={[]} reason="" why="" open={false} onToggle={() => {}} onChoose={() => {}} onEdit={() => {}} />);
    expect(screen.getByRole("group", { name: "Signature on this email" })).toHaveTextContent("No signature");
  });

  it("lists signatures numbered 1–3 and None as 0, with the current one chosen", async () => {
    const p = setup({ open: true });
    const group = screen.getByRole("radiogroup", { name: "Signature for this email" });
    expect(group).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /^Studio/ })).toBeChecked();
    expect(screen.getByText("Maya Ross · Ross Studio")).toBeInTheDocument();
    expect(screen.getByText("No signature on this email")).toBeInTheDocument();
    expect(screen.getByText(p.why)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("radio", { name: /^Work \(full\)/ }));
    expect(p.onChoose).toHaveBeenLastCalledWith("full");
    await userEvent.click(screen.getByRole("button", { name: /^Edit signatures/ }));
    expect(p.onEdit).toHaveBeenCalledOnce();
  });

  it("picks with number keys, 0 for none, and closes with Esc before anything else hears it", () => {
    const outer = vi.fn();
    window.addEventListener("keydown", outer);
    const p = setup({ open: true });
    fireEvent.keyDown(window, { key: "2" });
    expect(p.onChoose).toHaveBeenLastCalledWith("short");
    fireEvent.keyDown(window, { key: "0" });
    expect(p.onChoose).toHaveBeenLastCalledWith(null);
    fireEvent.keyDown(window, { key: "9" });
    expect(p.onChoose).toHaveBeenCalledTimes(2);
    fireEvent.keyDown(window, { key: "Escape" });
    expect(p.onToggle).toHaveBeenCalledOnce();
    expect(outer).toHaveBeenCalledTimes(1); // only the "9" got through
    window.removeEventListener("keydown", outer);
  });

  it.each(digitFroms)("picks from the %s keys, with or without ⌥ still held from ⌥S", (from) => {
    const p = setup({ open: true });
    expect(pressDigit(3, from)).toBe(false);
    expect(p.onChoose).toHaveBeenLastCalledWith("full");
    expect(pressDigit(2, from, { alt: true })).toBe(false);
    expect(p.onChoose).toHaveBeenLastCalledWith("short");
    expect(pressDigit(0, from, { alt: true })).toBe(false);
    expect(p.onChoose).toHaveBeenLastCalledWith(null);
    expect(pressDigit(4, from)).toBe(true);
    expect(p.onChoose).toHaveBeenCalledTimes(3);
  });

  it("↑ ↓ ↵ pick from the signature on the email, None last, and no key reaches the app", async () => {
    const outer = vi.fn();
    window.addEventListener("keydown", outer);
    const p = setup({ open: true, signature: short });
    const user = userEvent.setup();
    await user.keyboard("{ArrowDown}{Enter}");
    expect(p.onChoose).toHaveBeenLastCalledWith("full");
    await user.keyboard("{ArrowDown}{Enter}");
    expect(p.onChoose).toHaveBeenLastCalledWith(null);
    // Hovering never moves the mark.
    await user.hover(screen.getByRole("radio", { name: /^Studio/ }).closest("label")!);
    await user.keyboard("{Enter}");
    expect(p.onChoose).toHaveBeenLastCalledWith(null);
    expect(outer).not.toHaveBeenCalled();
    window.removeEventListener("keydown", outer);
  });

  it("starts on None when there is no signature", async () => {
    const p = setup({ open: true, signature: undefined });
    await userEvent.setup().keyboard("{Enter}");
    expect(p.onChoose).toHaveBeenCalledWith(null);
  });

  it("leaves ↑ ↓ alone while the picker is shut", () => {
    const outer = vi.fn();
    window.addEventListener("keydown", outer);
    const p = setup();
    fireEvent.keyDown(document.body, { key: "ArrowDown" });
    fireEvent.keyDown(document.body, { key: "Enter" });
    expect(outer).toHaveBeenCalledTimes(2);
    expect(p.onChoose).not.toHaveBeenCalled();
    window.removeEventListener("keydown", outer);
  });

  it("marks None when there is no signature", () => {
    setup({ open: true, signature: undefined });
    expect(screen.getByRole("radio", { name: /^None/ })).toBeChecked();
    expect(screen.getByRole("button", { name: "Add signatureAlt+S" })).toBeInTheDocument();
  });
});
