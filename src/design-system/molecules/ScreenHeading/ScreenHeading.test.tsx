import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { ScreenHeading } from "./ScreenHeading";

it("is a heading with a sentence under it", () => {
  render(<ScreenHeading title="Accounts" sub="They show in the inbox in this order." />);
  expect(screen.getByRole("heading", { level: 2, name: "Accounts" })).toBeInTheDocument();
  expect(screen.getByText("They show in the inbox in this order.")).toBeInTheDocument();
});
