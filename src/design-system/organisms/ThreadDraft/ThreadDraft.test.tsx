import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { ThreadDraft } from "./ThreadDraft";

const setup = (extra: Partial<Parameters<typeof ThreadDraft>[0]> = {}) => {
  const props = { details: { from: "maya@example.test", to: [], cc: [], bcc: [] }, line: "To Alex Morgan Studio Build · from maya@example.test", body: "Hi Alex,\n\nI'll text you soon.", saved: "Saved 2 min ago", onEdit: vi.fn(), onDelete: vi.fn(), onSend: vi.fn(), ...extra };
  render(<ThreadDraft {...props} />);
  return props;
};

it("shows the draft at the end of its conversation: who it's for, that it isn't sent, and its words", () => {
  setup();
  const card = screen.getByRole("region", { name: "Your draft" });
  expect(card).toHaveTextContent("To Alex Morgan Studio Build · from maya@example.test");
  expect(card).toHaveTextContent("Not sent yet · Saved 2 min ago");
  expect(screen.getByText(/I'll text you soon\./)).toBeInTheDocument();
});

it("Edit ⌘E opens it, Delete throws it away, Send ⌘↵ sends it", async () => {
  const props = setup();
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: "EditCtrl+E" }));
  expect(props.onEdit).toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Delete" }));
  expect(props.onDelete).toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "SendCtrl+↵" }));
  expect(props.onSend).toHaveBeenCalled();
});

it("a draft set to go later says when; one with no words says so", () => {
  setup({ at: "Mon 9:00 AM", body: "  " });
  expect(screen.getByRole("button", { name: "Send Mon 9:00 AMCtrl+↵" })).toBeInTheDocument();
  expect(screen.getByText("No words yet.")).toBeInTheDocument();
});

it.each(["\u061c", "\u200e", "\u200f", "\u202a", "\u202b", "\u202c", "\u202d", "\u202e", "\u2066", "\u2067", "\u2068", "\u2069"])("keeps direction controls out of the send header: %s", control => {
  setup({ line: `To Dana${control} · from maya@example.test` });
  expect(screen.getByText("To Dana · from maya@example.test").closest("bdi")).toHaveAttribute("dir", "ltr");
});
