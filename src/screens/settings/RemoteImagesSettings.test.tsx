import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { useRemoteImages } from "../../state/remoteImages";
import { AppearanceSection } from "./AppearanceSection";

afterEach(() => { vi.restoreAllMocks(); localStorage.clear(); useRemoteImages.setState({ policy: "familiar", saved: true }); });
it("lets the user choose strict images and explains direct requests", async () => {
  const user = userEvent.setup(); render(<AppearanceSection />);
  expect(screen.getByRole("radio", { name: "Familiar senders" })).toHaveAttribute("aria-checked", "true");
  await user.click(screen.getByRole("radio", { name: "Ask for every message" }));
  expect(useRemoteImages.getState().policy).toBe("ask");
  expect(screen.getByText(/Loading remote images can reveal your IP address/)).toBeInTheDocument();
  await user.click(screen.getByRole("radio", { name: "Familiar senders" }));
  expect(useRemoteImages.getState().policy).toBe("familiar");
});
it("reports that a choice was not saved", async () => {
  vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => { throw Error("unavailable"); });
  render(<AppearanceSection />);
  act(() => useRemoteImages.getState().setPolicy("ask"));
  expect(screen.getByRole("status")).toHaveTextContent("This choice could not be saved. It applies until Moss closes.");
});
