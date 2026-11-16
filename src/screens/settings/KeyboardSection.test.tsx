import { render, screen, within } from "@testing-library/react";
import { expect, it } from "vitest";
import { KeyboardSection } from "./KeyboardSection";

it("lists every key by what it does, in four groups", () => {
  render(<KeyboardSection />);
  for (const group of ["Move around", "Ask AI", "Act on email", "App"]) {
    expect(screen.getByRole("region", { name: group })).toBeInTheDocument();
  }
  const move = within(screen.getByRole("region", { name: "Move around" }));
  const next = move.getByText("Next or previous email").parentElement!;
  expect([...next.querySelectorAll("kbd")].map((k) => k.textContent)).toEqual(["↓", "↑"]);
  // J and K no longer move; nothing in the list offers them.
  const keys = [...document.querySelectorAll("kbd")].map((k) => k.textContent);
  expect(keys).not.toContain("J");
  expect(keys).not.toContain("K");
  expect(move.getByText("Done list").parentElement).toHaveTextContent("D");
  expect(move.getByText("Inbox zero").parentElement).toHaveTextContent("Z");
  expect(move.getByText("Blocked mail").parentElement).toHaveTextContent("Ctrl+Shift+B");
  expect(within(screen.getByRole("region", { name: "Ask AI" })).getByText("Summarise the open email").parentElement).toHaveTextContent("S");
  expect(within(screen.getByRole("region", { name: "Ask AI" })).getByText("Choose a kind of summary first").parentElement).toHaveTextContent("Shift+S");
  const act = within(screen.getByRole("region", { name: "Act on email" }));
  expect(act.getByText("Done, or not done again").parentElement).toHaveTextContent("E");
  expect(act.getByText("Delete (to Trash)").parentElement).toHaveTextContent("Backspace");
  expect(act.getByText("Delete a search result").parentElement).toHaveTextContent("Ctrl+Backspace");
  for (const [action, keys] of [["Block sender", "B"], ["Block their domain", "Shift+B"], ["Accept sender", "A"], ["Accept their domain", "Shift+A"], ["Unblock sender", "U"], ["Unread, or read again", "U"]])
    expect(act.getByText(action).parentElement).toHaveTextContent(keys);
  expect(act.getByText("Send later").parentElement).toHaveTextContent("Ctrl+Shift+↵");
  expect(act.getByText("Attach files").parentElement).toHaveTextContent("Ctrl+Shift+A");
  expect(act.getByText("Remove an attached file").parentElement).toHaveTextContent("Backspace");
  expect(within(screen.getByRole("region", { name: "App" })).getByText("Undo done, delete or snooze").parentElement).toHaveTextContent("Ctrl+Z");
  expect(within(screen.getByRole("region", { name: "App" })).getByText("Settings").parentElement).toHaveTextContent("Ctrl+,");
});
