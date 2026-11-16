import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { providers } from "../../../data/providers";
import { ProviderList } from "./ProviderList";

it("lists providers with their number keys, the cursor row offering Continue", async () => {
  const onPick = vi.fn();
  render(<ProviderList providers={providers} cursor={0} onPick={onPick} />);
  const options = screen.getAllByRole("option");
  expect(options).toHaveLength(7);
  expect(options[0]).toHaveAttribute("aria-selected", "true");
  expect(options[0]).toHaveTextContent("Continue");
  expect(options[6]).toHaveTextContent("Other (IMAP)");
  expect(options[6]).toHaveTextContent("7");
  const user = userEvent.setup();
  // Pointing at a row never moves the cursor; only the keys and a click do.
  await user.hover(options[2]);
  expect(options[2]).toHaveAttribute("aria-selected", "false");
  expect(onPick).not.toHaveBeenCalled();
  await user.click(options[3]);
  expect(onPick).toHaveBeenCalledWith(3);
});
