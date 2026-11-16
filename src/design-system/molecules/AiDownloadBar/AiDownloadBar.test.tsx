import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { AiDownloadBar } from "./AiDownloadBar";

it("says how far the download is and opens on click", async () => {
  const onOpen = vi.fn();
  render(<AiDownloadBar label="Getting AI · 2.1 of 6.4 GB" value={0.33} onOpen={onOpen} />);
  expect(screen.getByRole("progressbar", { name: "AI download" })).toHaveAttribute("aria-valuenow", "33");
  await userEvent.setup().click(screen.getByRole("button", { name: /Getting AI/ }));
  expect(onOpen).toHaveBeenCalledOnce();
});

it("a waiting download goes grey", () => {
  render(<AiDownloadBar label="AI paused · phone hotspot" value={0.5} paused onOpen={() => {}} />);
  expect(screen.getByRole("progressbar", { name: "AI download" })).toHaveAttribute("aria-valuenow", "50");
  expect(screen.getByText("AI paused · phone hotspot")).toBeInTheDocument();
});
