import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { closeWindow } from "../lib/window";
import { resetStore, thread } from "../test/fixtures";
import { useStore } from "../state/store";
import { InboxZeroScreen } from "./InboxZeroScreen";
import { changeWhatNoListShows, renderCounted } from "../test/renders";

vi.mock("../lib/window", () => ({ closeWindow: vi.fn() }));

it("lets you close the window or see what’s coming back", async () => {
  resetStore([thread("a", { section: "later" })]);
  useStore.setState({ view: "zero", handled: 3 });
  render(<InboxZeroScreen />);
  const user = userEvent.setup();
  await user.click(screen.getByRole("button", { name: /Close and take a break/ }));
  expect(closeWindow).toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: /See what’s coming back/ }));
  expect(useStore.getState().view).toBe("inbox");
});

it("credits the photo behind it", () => {
  resetStore([]);
  useStore.setState({ view: "zero" });
  render(<InboxZeroScreen />);
  expect(screen.getByText(/Sprout in the moss.*Runic Earth on Unsplash/)).toBeInTheDocument();
});

it("a store change it doesn't show doesn't redraw it", () => {
  const { renders } = renderCounted(<InboxZeroScreen />);
  changeWhatNoListShows();
  act(() => useStore.setState({ cursorId: "elsewhere", doneCursorId: "elsewhere" }));
  expect(renders).not.toHaveBeenCalled();
});
