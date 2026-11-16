import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ReplyBar } from "./ReplyBar";

function setup(hint?: string, replyTo?: string) {
  const props = { hint, replyTo, onReply: vi.fn(), onReplyAll: vi.fn(), onForward: vi.fn() };
  render(<ReplyBar {...props} />);
  return props;
}

describe("ReplyBar", () => {
  it("shows the hint", () => {
    setup("3 suggested replies ready");
    expect(screen.getByText("3 suggested replies ready")).toBeInTheDocument();
  });

  it("shows each action with its key", () => {
    setup();
    // The accessible name is the label followed by the key cap. jsdom is not a Mac, so ⇧ is spelled out.
    expect(screen.getByRole("button", { name: "ReplyR" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Reply allShift+R" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "ForwardF" })).toBeInTheDocument();
  });

  it("calls the matching handler for each button", async () => {
    const user = userEvent.setup();
    const p = setup();
    await user.click(screen.getByRole("button", { name: "ReplyR" }));
    expect(p.onReply).toHaveBeenCalledOnce();
    expect(p.onReplyAll).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Reply allShift+R" }));
    expect(p.onReplyAll).toHaveBeenCalledOnce();
    await user.click(screen.getByRole("button", { name: "ForwardF" }));
    expect(p.onForward).toHaveBeenCalledOnce();
    expect(p.onReply).toHaveBeenCalledOnce();
  });

  it("with someone to reply to, is a full-width reply field with reply all and forward beside it", async () => {
    const user = userEvent.setup();
    const p = setup(undefined, "Priya");
    await user.click(screen.getByRole("button", { name: "Reply to Priya…R" }));
    expect(p.onReply).toHaveBeenCalledOnce();
    await user.click(screen.getByRole("button", { name: "Shift+RReply all" }));
    expect(p.onReplyAll).toHaveBeenCalledOnce();
    await user.click(screen.getByRole("button", { name: "FForward" }));
    expect(p.onForward).toHaveBeenCalledOnce();
  });

  it("keeps a long address to one line, with the whole of it on hover", () => {
    const address = "jordan.whitfield@mortgage-desk.lending.example.com";
    setup(undefined, address);
    // The field cuts the address short with an ellipsis (CSS), so the full one is in its tooltip.
    expect(screen.getByRole("button", { name: `Reply to ${address}…R` })).toHaveAttribute("title", `Reply to ${address}`);
  });

  it("offers Quick reply (Q) between Reply and Reply all when Moss can write", async () => {
    const user = userEvent.setup();
    const onQuickReply = vi.fn();
    render(<ReplyBar replyTo="Dana" onReply={vi.fn()} onReplyAll={vi.fn()} onForward={vi.fn()} onQuickReply={onQuickReply} />);
    const names = screen.getAllByRole("button").map((b) => b.textContent);
    expect(names).toEqual(["Reply to Dana…R", "QQuick reply", "Shift+RReply all", "FForward"]);
    await user.click(screen.getByRole("button", { name: "QQuick reply" }));
    expect(onQuickReply).toHaveBeenCalledOnce();
  });

  it("without Quick reply, there's no button for it", () => {
    setup(undefined, "Dana");
    expect(screen.queryByRole("button", { name: /Quick reply/ })).not.toBeInTheDocument();
  });
});

it("makes the full reply address available when the field is truncated",()=>{
 const address="cabinet-project-coordinator@design-studio.example.test";
 setup(undefined,address);
 expect(screen.getByRole("button",{name:`Reply to ${address}…R`})).toHaveAttribute("title",`Reply to ${address}`);
});
