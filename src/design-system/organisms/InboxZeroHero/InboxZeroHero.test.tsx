import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { InboxZeroHero } from "./InboxZeroHero";

function setup() {
  const onClose = vi.fn();
  const onComingBack = vi.fn();
  render(
    <InboxZeroHero
      handled={23}
      minutes={14}
      comingBack={4}
      waiting={2}
      nextCheck="Next check at 10:30."
      photoCredit="Photo by Ansel"
      onClose={onClose}
      onComingBack={onComingBack}
    />,
  );
  return { onClose, onComingBack, user: userEvent.setup() };
}

describe("InboxZeroHero", () => {
  it("celebrates with what you got done", () => {
    setup();
    expect(screen.getByRole("heading", { level: 1, name: "Inbox zero." })).toBeInTheDocument();
    expect(screen.getByText("You handled 23 emails in 14 minutes. That’s it for now.")).toBeInTheDocument();
  });

  it("shows each stat next to its label", () => {
    setup();
    for (const [value, label] of [
      ["23", "handled"],
      ["4", "coming back later"],
      ["2", "waiting on you"],
    ]) {
      expect(screen.getByText(label).previousSibling).toHaveTextContent(value);
    }
  });

  it("shows the next check and the photo credit", () => {
    setup();
    expect(screen.getByText("Next check at 10:30.")).toBeInTheDocument();
    expect(screen.getByText("Photo by Ansel")).toBeInTheDocument();
  });

  it("closes from the break button", async () => {
    const { onClose, onComingBack, user } = setup();
    await user.click(screen.getByRole("button", { name: /Close and take a break/ }));
    expect(onClose).toHaveBeenCalledOnce();
    expect(onComingBack).not.toHaveBeenCalled();
  });

  it("shows what is coming back", async () => {
    const { onClose, onComingBack, user } = setup();
    await user.click(screen.getByRole("button", { name: "See what’s coming back" }));
    expect(onComingBack).toHaveBeenCalledOnce();
    expect(onClose).not.toHaveBeenCalled();
  });
});
