import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { SenderRule } from "../data/types";
import { useSenderRules } from "../state/senderRules";
import { useStore } from "../state/store";
import { fakeMail } from "../test/fakeMail";
import { resetStore } from "../test/fixtures";
import { SendersScreen } from "./SendersScreen";

const rule = (sender: string, blocked: boolean, accountId = "work"): SenderRule => ({
  accountId,
  sender,
  wholeDomain: !sender.includes("@"),
  blocked,
  createdAt: new Date(2026, 9, 22, 9).getTime(),
});
const rules = [rule("jordan@growthleads.example", true), rule("promomail.example", true, "personal"), rule("accounts.google.com", false)];

beforeEach(() => {
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(new Date(2026, 10, 4, 12));
  resetStore([]);
  useSenderRules.setState({ rules: undefined, problem: undefined });
  useStore.getState().openSenders();
});

afterEach(() => vi.useRealTimers());

async function renderWith(list: SenderRule[] = rules) {
  const mail = fakeMail({ senderRules: vi.fn(async () => list) });
  render(<SendersScreen />);
  await screen.findByRole("list", { name: "Blocked" });
  return mail;
}

const row = (list: string, name: string) =>
  within(screen.getByRole("list", { name: list }))
    .getAllByRole("listitem")
    .find((r) => r.textContent?.includes(name))!;

it("lists who you blocked and who you accepted, each with its account", async () => {
  await renderWith();
  const blocked = within(screen.getByRole("list", { name: "Blocked" })).getAllByRole("listitem");
  expect(blocked.map((r) => r.textContent)).toEqual([
    expect.stringMatching(/^jordan@growthleads\.exampleWork · Oct 22Unblock/),
    expect.stringMatching(/^Everyone at promomail\.examplePersonal · Oct 22Unblock/),
  ]);
  const accepted = within(screen.getByRole("list", { name: "Accepted" })).getAllByRole("listitem");
  expect(accepted.map((r) => r.textContent)).toEqual([expect.stringMatching(/^Everyone at accounts\.google\.comWork · Oct 22Remove/)]);
});

it("Unblock and Remove forget the rule, so Moss asks about them again", async () => {
  const { backend } = await renderWith();
  await userEvent.click(within(row("Blocked", "jordan@")).getByRole("button", { name: "Unblock" }));
  expect(backend.forgetSenderRule).toHaveBeenCalledWith("work", "jordan@growthleads.example");
  await userEvent.click(within(row("Accepted", "google")).getByRole("button", { name: "Remove" }));
  expect(backend.forgetSenderRule).toHaveBeenCalledWith("work", "accounts.google.com");
});

it("↑ ↓ move between rules and ⌫ forgets the one under the cursor", async () => {
  const user = userEvent.setup();
  const { backend } = await renderWith();
  expect(row("Blocked", "jordan@")).toHaveAttribute("aria-current", "true");
  await user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}{ArrowUp}{ArrowDown}");
  expect(row("Accepted", "google")).toHaveAttribute("aria-current", "true");
  await user.keyboard("{Backspace}");
  expect(backend.forgetSenderRule).toHaveBeenCalledWith("work", "accounts.google.com");
  // The cursor stays on the last rule left.
  expect(row("Blocked", "promomail")).toHaveAttribute("aria-current", "true");
  await user.keyboard("{Meta>}{Backspace}{/Meta}t");
  expect(backend.forgetSenderRule).toHaveBeenCalledOnce();
});

it("says how to block or accept someone when there is no one yet, and ⌫ does nothing", async () => {
  const user = userEvent.setup();
  const { backend } = await renderWith([]);
  expect(screen.getByRole("list", { name: "Blocked" })).toHaveTextContent("No one is blocked. On a new sender’s email, B blocks them.");
  expect(screen.getByRole("list", { name: "Accepted" })).toHaveTextContent("No one accepted yet. A accepts a new sender, ⇧A everyone at their domain.");
  await user.keyboard("{Backspace}");
  expect(backend.forgetSenderRule).not.toHaveBeenCalled();
});

it("opens the Blocked list from here", async () => {
  await renderWith();
  await userEvent.click(screen.getByRole("button", { name: "Blocked mailCtrl+Shift+B" }));
  expect(useStore.getState().view).toBe("blocked");
});

it("says why a change didn't save", async () => {
  fakeMail({
    senderRules: vi.fn(async () => rules),
    forgetSenderRule: vi.fn(() => Promise.reject({ kind: "store", message: "The disk is full." })),
  });
  render(<SendersScreen />);
  await userEvent.click(within(await screen.findByRole("list", { name: "Blocked" })).getAllByRole("button", { name: "Unblock" })[0]);
  expect(await screen.findByText("The disk is full.")).toBeInTheDocument();
});

it("shows no lists until the rules are in", () => {
  fakeMail({ senderRules: vi.fn(() => new Promise<SenderRule[]>(() => {})) });
  render(<SendersScreen />);
  expect(screen.getByRole("heading", { level: 1, name: "Senders" })).toBeInTheDocument();
  expect(screen.queryByRole("list", { name: "Blocked" })).not.toBeInTheDocument();
});

it("is its own screen: says what it is for, shows its keys, and Esc or Back goes back", async () => {
  const user = userEvent.setup();
  await renderWith();
  expect(screen.getByRole("heading", { level: 1, name: "Senders" })).toBeInTheDocument();
  expect(screen.getByText(/Who Moss keeps out, and who it lets through without asking/)).toBeInTheDocument();
  expect(screen.getByRole("contentinfo")).toHaveTextContent(/Move.*Remove.*Back/);
  await user.keyboard("{Escape}");
  expect(useStore.getState().view).toBe("inbox");
  useStore.getState().openSenders();
  await user.click(screen.getByRole("button", { name: /Back/ }));
  expect(useStore.getState().view).toBe("inbox");
});
