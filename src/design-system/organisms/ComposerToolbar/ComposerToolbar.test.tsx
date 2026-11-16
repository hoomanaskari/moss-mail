import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ComposerToolbar, type ComposerToolbarProps } from "./ComposerToolbar";

function setup(remindIfNoReply = false) {
  const props: ComposerToolbarProps = {
    remindIfNoReply,
    onAttach: vi.fn(),
    onSendLater: vi.fn(),
    onToggleRemind: vi.fn(),
    onDiscard: vi.fn(),
    onSend: vi.fn(),
  };
  render(<ComposerToolbar {...props} />);
  return props;
}

describe("ComposerToolbar", () => {
  it("starts with Write with Moss (⌘J) when Moss can write", async () => {
    const onWriteWithMoss = vi.fn();
    render(
      <ComposerToolbar
        remindIfNoReply={false}
        onAttach={vi.fn()}
        onSendLater={vi.fn()}
        onToggleRemind={vi.fn()}
        onDiscard={vi.fn()}
        onSend={vi.fn()}
        onWriteWithMoss={onWriteWithMoss}
      />,
    );
    const first = screen.getAllByRole("button")[0];
    expect(first).toHaveAccessibleName("Write with MossCtrl+J");
    await userEvent.click(first);
    expect(onWriteWithMoss).toHaveBeenCalledOnce();
  });

  it("without it, has no Moss button", () => {
    setup();
    expect(screen.queryByRole("button", { name: /Moss/ })).not.toBeInTheDocument();
  });

  it("each button calls its own handler", async () => {
    const user = userEvent.setup();
    const p = setup();
    const pairs = [
      ["AttachCtrl+Shift+A", p.onAttach],
      [/^Send later/, p.onSendLater],
      ["Remind me if no reply", p.onToggleRemind],
      ["Discard", p.onDiscard],
      [/^Send(?! later)/, p.onSend],
    ] as const;
    for (const [name, handler] of pairs) {
      await user.click(screen.getByRole("button", { name }));
      expect(handler).toHaveBeenCalledOnce();
    }
  });

  it("Send shows at once that it is sending, and takes no second press", () => {
    render(<ComposerToolbar remindIfNoReply={false} sending onAttach={vi.fn()} onSendLater={vi.fn()} onToggleRemind={vi.fn()} onDiscard={vi.fn()} onSend={vi.fn()} />);
    const send = screen.getByRole("button", { name: /^Sending…/ });
    expect(send).toHaveAttribute("aria-busy", "true");
    expect(send).toBeDisabled();
  });

  it("shows whether the reminder is on", () => {
    setup(false);
    expect(screen.getByRole("button", { name: "Remind me if no reply" })).toHaveAttribute("aria-pressed", "false");
  });

  it("presses the reminder button when it is on", () => {
    setup(true);
    expect(screen.getByRole("button", { name: "Remind me if no reply" })).toHaveAttribute("aria-pressed", "true");
  });

  it("shows the send shortcut, and Send later's", () => {
    setup();
    expect(screen.getByRole("button", { name: /^Send later/ })).toHaveTextContent(/^Send later(⌘⇧|Ctrl\+Shift\+)↵$/);
    // "⌘↵" on a Mac, "Ctrl+↵" elsewhere.
    expect(screen.getByRole("button", { name: /^Send(?! later)/ })).toHaveTextContent(/^Send(⌘|Ctrl\+)↵$/);
  });
});
