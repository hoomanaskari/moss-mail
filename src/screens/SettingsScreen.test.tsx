import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it } from "vitest";
import { demoAi, setAi } from "../services/ai";
import { useStore } from "../state/store";
import { resetDemoStore } from "../test/fixtures";
import { digitFroms, pressDigit } from "../test/keys";
import { SettingsScreen } from "./SettingsScreen";

const s = () => useStore.getState();

beforeEach(() => {
  resetDemoStore();
  s().openSettings();
});

it("is titled Settings and closes with Esc or its button", async () => {
  const user = userEvent.setup();
  const { unmount } = render(<SettingsScreen />);
  expect(screen.getByRole("heading", { name: "Settings" })).toBeInTheDocument();
  await user.keyboard("{Escape}");
  expect(s().view).toBe("inbox");
  s().openSettings();
  await user.click(screen.getByRole("button", { name: /Close/ }));
  expect(s().view).toBe("inbox");
  unmount();
});

it("⌘1–⌘6 and the list on the left switch sections, each with its own keys along the bottom", async () => {
  const user = userEvent.setup();
  render(<SettingsScreen />);
  const footer = () => screen.getByRole("contentinfo");
  expect(screen.getByRole("heading", { level: 2, name: "Accounts" })).toBeInTheDocument();
  expect(footer()).toHaveTextContent("Reorder");
  expect(footer()).not.toHaveTextContent(/Move|J K/);
  await user.keyboard("{Meta>}2{/Meta}");
  expect(screen.getByRole("heading", { level: 2, name: "Signatures" })).toBeInTheDocument();
  expect(footer()).toHaveTextContent("New signature");
  expect(footer()).not.toHaveTextContent(/Move|J K/);
  await user.keyboard("{Meta>}3{/Meta}");
  expect(screen.getByRole("heading", { level: 2, name: "Appearance" })).toBeInTheDocument();
  expect(footer()).toHaveTextContent("Next setting");
  await user.keyboard("{Meta>}4{/Meta}");
  expect(screen.getByRole("heading", { level: 2, name: "AI" })).toBeInTheDocument();
  expect(footer()).toHaveTextContent("Download model");
  expect(footer()).toHaveTextContent("Delete model");
  expect(footer()).toHaveTextContent("Your agents");
  expect(footer()).not.toHaveTextContent(/Move|J K/);
  await user.keyboard("{Meta>}5{/Meta}");
  expect(await screen.findByRole("heading", { level: 2, name: "Storage" })).toBeInTheDocument();
  expect(footer()).toHaveTextContent("Clear cache");
  expect(footer()).not.toHaveTextContent(/Move|J K/);
  await user.keyboard("{Meta>}6{/Meta}");
  expect(screen.getByRole("heading", { level: 2, name: "Keyboard shortcuts" })).toBeInTheDocument();
  expect(footer()).toHaveTextContent("Show shortcuts anywhere");
  // ⌘7 is not a section, and Senders lives in ⌘K, not here.
  await user.keyboard("{Meta>}7{/Meta}");
  expect(s().settingsSection).toBe("keyboard");
  await user.click(screen.getByRole("button", { name: /Accounts/ }));
  expect(s().settingsSection).toBe("accounts");
  expect(screen.getByRole("navigation", { name: "Settings" })).toHaveTextContent("Moss 0.1.0 · Free and open source");
  expect(screen.getByRole("navigation", { name: "Settings" })).not.toHaveTextContent("Senders");
});

it.each(digitFroms)("⌘1–⌘6 switch sections from the %s keys too", (from) => {
  render(<SettingsScreen />);
  expect(pressDigit(3, from, { meta: true })).toBe(false);
  expect(screen.getByRole("heading", { level: 2, name: "Appearance" })).toBeInTheDocument();
  pressDigit(6, from, { ctrl: true });
  expect(s().settingsSection).toBe("keyboard");
  expect(pressDigit(7, from, { meta: true })).toBe(true);
  // Without ⌘ a digit is not a section key.
  pressDigit(2, from);
  expect(s().settingsSection).toBe("keyboard");
});

it("Esc while a model's Delete is asking stops asking, and leaves Settings open", async () => {
  const user = userEvent.setup();
  setAi(demoAi({ state: "ready" }));
  s().openSettings("ai");
  render(<SettingsScreen />);
  await screen.findByText(/This Mac:/);
  await user.keyboard("{Meta>}{Backspace}{/Meta}");
  expect(screen.getByRole("button", { name: "Yes, delete Gemma 4 E4B" })).toBeInTheDocument();
  await user.keyboard("{Escape}");
  expect(screen.getByRole("button", { name: "Delete Gemma 4 E4B" })).toBeInTheDocument();
  expect(s().view).toBe("settings");
  await user.keyboard("{Escape}");
  expect(s().view).toBe("inbox");
});
