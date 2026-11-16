import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Palette, Users } from "lucide-react";
import { expect, it, vi } from "vitest";
import { SettingsNav } from "./SettingsNav";

it("lists the sections with their keys and marks the current one", async () => {
  const onSelect = vi.fn();
  render(
    <SettingsNav
      items={[
        { id: "accounts", label: "Accounts", icon: Users, keys: "⌘1" },
        { id: "appearance", label: "Appearance", icon: Palette, keys: "⌘3" },
      ]}
      selected="accounts"
      onSelect={onSelect}
      footer="Moss 0.1"
    />,
  );
  expect(screen.getByRole("navigation", { name: "Settings" })).toHaveTextContent("Moss 0.1");
  expect(screen.getByRole("button", { name: /Accounts/ })).toHaveAttribute("aria-current", "page");
  expect(screen.getByRole("button", { name: /Appearance/ })).not.toHaveAttribute("aria-current");
  expect(screen.getByRole("button", { name: /Appearance/ })).toHaveTextContent("Ctrl+3");
  await userEvent.click(screen.getByRole("button", { name: /Appearance/ }));
  expect(onSelect).toHaveBeenCalledWith("appearance");
});
