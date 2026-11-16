import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { ProviderTile } from "./ProviderTile";

it("shows the provider’s letter, or an @ for other email", () => {
  const { container, rerender } = render(<ProviderTile letter="G" />);
  expect(screen.getByText("G")).toBeInTheDocument();
  expect(container.firstElementChild).toHaveAttribute("aria-hidden", "true");
  rerender(<ProviderTile size={24} raised />);
  expect(container.querySelector("svg")).toHaveAttribute("width", "14");
  rerender(<ProviderTile />);
  expect(container.querySelector("svg")).toHaveAttribute("width", "18");
});
