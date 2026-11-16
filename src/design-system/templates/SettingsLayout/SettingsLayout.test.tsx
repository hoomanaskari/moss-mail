import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { SettingsLayout } from "./SettingsLayout";

it("puts the section list beside the section", () => {
  render(<SettingsLayout nav={<nav aria-label="Sections" />}>Accounts page</SettingsLayout>);
  expect(screen.getByRole("navigation", { name: "Sections" })).toBeInTheDocument();
  expect(screen.getByText("Accounts page")).toBeInTheDocument();
});
