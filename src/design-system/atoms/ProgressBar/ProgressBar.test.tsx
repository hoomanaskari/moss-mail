import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { ProgressBar } from "./ProgressBar";

it("reports how far along it is, kept between 0 and 100%", () => {
  render(
    <>
      <ProgressBar value={0.64} label="Downloading" />
      <ProgressBar value={3} label="Over" />
    </>,
  );
  const [bar, over] = screen.getAllByRole("progressbar");
  expect(bar).toHaveAttribute("aria-valuenow", "64");
  expect(bar).toHaveAccessibleName("Downloading");
  expect(over).toHaveAttribute("aria-valuenow", "100");
});
