import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { gmailDemoAccount } from "../../data/demo";
import { useSync } from "../../services/live";
import { useStore } from "../../state/store";
import { resetDemoStore, work } from "../../test/fixtures";
import { fakeMail } from "../../test/fakeMail";
import { AccountsSection } from "./AccountsSection";

const s = () => useStore.getState();
const ids = () => s().accounts.map((a) => a.id);
const editing = () => screen.getByRole("region", { name: /^Editing/ });

beforeEach(() => {
  resetDemoStore();
  s().addAccount(gmailDemoAccount);
  s().openSettings("accounts");
});

describe("Accounts settings", () => {
  it("lists the accounts in order with their key, how they connect and whether they are signed in", () => {
    render(<AccountsSection />);
    const rows = screen.getAllByRole("option");
    expect(rows.map((r) => within(r).getByText(/^(Work|Personal|Studio|Gmail)$/).textContent)).toEqual([
      "Work",
      "Personal",
      "Studio",
      "Gmail",
    ]);
    expect(rows[0]).toHaveAttribute("aria-selected", "true");
    expect(rows[0]).toHaveTextContent("Microsoft 365");
    expect(rows[0]).toHaveTextContent("Up to date");
    expect(rows[0]).toHaveTextContent("Ctrl+1");
    expect(rows[3]).toHaveTextContent("Signed out");
  });

  it("an account that can't sync says so instead of “Up to date”", () => {
    useSync.setState({ problems: { work: { problem: "offline", message: "no network" }, personal: { problem: "server", message: "refused" } } });
    render(<AccountsSection />);
    const rows = screen.getAllByRole("option");
    expect(rows[0]).toHaveTextContent("Offline");
    expect(rows[1]).toHaveTextContent("Not syncing");
    expect(rows[2]).toHaveTextContent("Up to date");
  });

  it("“Sign in again”, ⌘N and the Add account button open Add account", async () => {
    const user = userEvent.setup();
    render(<AccountsSection />);
    await user.click(screen.getByRole("button", { name: "Sign in again" }));
    expect(s().view).toBe("add-account");
    s().leave();
    await user.keyboard("{Meta>}n{/Meta}");
    expect(s().view).toBe("add-account");
    s().leave();
    await user.click(screen.getByRole("button", { name: /Add account/ }));
    expect(s().view).toBe("add-account");
  });

  it("↓ and ↑ move between accounts (J and K do nothing), and clicking one edits it", async () => {
    const user = userEvent.setup();
    render(<AccountsSection />);
    await user.keyboard("jJkK");
    expect(editing()).toHaveAccessibleName("Editing Work");
    await user.keyboard("{ArrowDown}");
    expect(editing()).toHaveAccessibleName("Editing Personal");
    await user.keyboard("{ArrowUp}{ArrowUp}");
    expect(editing()).toHaveAccessibleName("Editing Work");
    await user.click(screen.getByText("Studio"));
    expect(editing()).toHaveAccessibleName("Editing Studio");
    await user.keyboard("{ArrowDown}{ArrowDown}{ArrowDown}");
    expect(editing()).toHaveAccessibleName("Editing Gmail");
  });

  it("renames an account, and its letter follows the new name", async () => {
    const user = userEvent.setup();
    render(<AccountsSection />);
    const name = screen.getByLabelText("Name");
    await user.clear(name);
    expect(s().accounts[0]).toMatchObject({ name: "", letter: "W" });
    await user.type(name, "job");
    expect(s().accounts[0]).toMatchObject({ name: "job", letter: "J" });
    // Typing in the field doesn't move between accounts.
    expect(editing()).toHaveAccessibleName("Editing job");
  });

  it("changes the colour with ← →, the Change key or a swatch", async () => {
    const user = userEvent.setup();
    render(<AccountsSection />);
    await user.keyboard("{ArrowRight}");
    expect(s().accounts[0].color).toBe("personal");
    await user.keyboard("{ArrowLeft}{ArrowLeft}");
    expect(s().accounts[0].color).toBe("olive");
    await user.click(screen.getByRole("button", { name: /Change/ }));
    expect(s().accounts[0].color).toBe("work");
    await user.click(screen.getByRole("radio", { name: "Violet" }));
    expect(s().accounts[0].color).toBe("violet");
  });

  it("reorders with ⌥↑ ⌥↓ or the Move buttons", async () => {
    const user = userEvent.setup();
    render(<AccountsSection />);
    await user.keyboard("{Alt>}{ArrowDown}{/Alt}");
    expect(ids()).toEqual(["personal", "work", "studio", "gmail"]);
    await user.keyboard("{Alt>}{ArrowUp}{/Alt}");
    expect(ids()).toEqual(["work", "personal", "studio", "gmail"]);
    await user.click(screen.getByRole("button", { name: /Move down/ }));
    await user.click(screen.getByRole("button", { name: /Move down/ }));
    expect(ids()).toEqual(["personal", "studio", "work", "gmail"]);
    await user.click(screen.getByRole("button", { name: /Move up/ }));
    expect(ids()).toEqual(["personal", "work", "studio", "gmail"]);
    // Other ⌥ keys do nothing.
    await user.keyboard("{Alt>}j{/Alt}");
    expect(editing()).toHaveAccessibleName("Editing Work");
  });

  it("asks once before removing, and Esc changes its mind without closing Settings", async () => {
    const user = userEvent.setup();
    render(<AccountsSection />);
    await user.keyboard("{Meta>}{Backspace}{/Meta}");
    expect(screen.getByRole("button", { name: /Yes, remove Work/ })).toBeInTheDocument();
    await user.keyboard("{Escape}");
    expect(s().view).toBe("settings");
    expect(screen.getByRole("button", { name: /Remove account…/ })).toBeInTheDocument();
    expect(screen.getByText("Local draft recovery copies remain on this Mac. Review or discard them in Drafts.")).toBeVisible();
    await user.click(screen.getByRole("button", { name: /Remove account…/ }));
    await user.click(screen.getByRole("button", { name: /Yes, remove Work/ }));
    expect(ids()).toEqual(["personal", "studio", "gmail"]);
    expect(editing()).toHaveAccessibleName("Editing Personal");
  });

  it("warns that removing a Resend account deletes the only copy of its mail", async () => {
    const resend = { ...work, id: "re", name: "Studio mail", connection: { provider: "resend" as const, summary: "Resend · 2 domains" } };
    useStore.setState({ accounts: [work, resend] });
    const user = userEvent.setup();
    render(<AccountsSection />);
    expect(screen.getByText(/Nothing changes on the server/)).toBeInTheDocument();
    await user.click(screen.getByText("Studio mail"));
    expect(
      screen.getByText(
        "Deletes this account’s mail from this Mac, then removes its API key from the Keychain. Moss will show a reminder if credential cleanup has to wait. Resend deletes received mail after 30 days, so older mail is only here: it’s gone for good.",
      ),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: /Remove account…/ }));
    await user.click(screen.getByRole("button", { name: /Yes, delete Studio mail for good/ }));
    expect(ids()).toEqual(["work"]);
  });

  it("after removing the last one, edits the one above; with none left there is nothing to edit", async () => {
    useStore.setState({ accounts: [work, gmailDemoAccount] });
    const user = userEvent.setup();
    render(<AccountsSection />);
    await user.keyboard("{ArrowDown}{Meta>}{Backspace}{Backspace}{/Meta}");
    expect(editing()).toHaveAccessibleName("Editing Work");
    await user.keyboard("{Meta>}{Backspace}{Backspace}{/Meta}");
    expect(ids()).toEqual([]);
    expect(screen.queryByRole("region")).not.toBeInTheDocument();
    expect(screen.queryByRole("listbox")).not.toBeInTheDocument();
    // With no account left, Moss goes back to Add account. Keys with nothing to act on are ignored.
    expect(s().view).toBe("add-account");
    await user.keyboard("{ArrowDown}{ArrowRight}{Meta>}{Backspace}{/Meta}");
    expect(s().view).toBe("add-account");
  });

  it("names an account without connection details plainly, and numbers only the first nine", () => {
    const many = Array.from({ length: 10 }, (_, i) => ({ ...work, id: `a${i}`, name: `A${i}`, connection: undefined }));
    useStore.setState({ accounts: many });
    render(<AccountsSection />);
    const rows = screen.getAllByRole("option");
    expect(rows[0]).toHaveTextContent("Email account");
    expect(rows[9].querySelector("kbd")).toBeNull();
  });
});

it("keeps the selected account while removal is pending or refused, then retries", async () => {
  let reject!: (error: Error) => void;
  const { backend } = fakeMail({ removeAccount: vi.fn().mockImplementationOnce(() => new Promise<void>((_, fail) => { reject = fail; })).mockResolvedValue({ pending: false }) });
  const user = userEvent.setup();
  render(<AccountsSection />);
  await user.click(screen.getByRole("button", { name: /Remove account/ }));
  await user.click(screen.getByRole("button", { name: /Yes, remove Work/ }));
  expect(editing()).toHaveAccessibleName("Editing Work");
  expect(screen.getByRole("button", { name: /Removing/ })).toBeDisabled();
  await user.keyboard("{Meta>}{Backspace}{/Meta}");
  expect(backend.removeAccount).toHaveBeenCalledOnce();
  await act(async () => reject(new Error("Resolve pending server changes first.")));
  expect(screen.getByRole("alert")).toHaveTextContent("Resolve pending server changes first.");
  expect(editing()).toHaveAccessibleName("Editing Work");
  await user.click(screen.getByRole("button", { name: /Yes, remove Work/ }));
  expect(ids()).not.toContain("work");
  expect(editing()).toHaveAccessibleName("Editing Personal");
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("a completed removal does not change another account's selection or confirmation", async () => {
  let finish!: () => void;
  fakeMail({ removeAccount: vi.fn(() => new Promise<{ pending: boolean }>(resolve => { finish = () => resolve({ pending: false }); })) });
  const user = userEvent.setup();
  render(<AccountsSection />);
  await user.click(screen.getByRole("button", { name: /Remove account/ }));
  await user.click(screen.getByRole("button", { name: /Yes, remove Work/ }));
  await user.click(screen.getByText("Studio"));
  await user.click(screen.getByRole("button", { name: /Remove account/ }));
  await act(async () => finish());
  expect(editing()).toHaveAccessibleName("Editing Studio");
  expect(screen.getByRole("button", { name: /Yes,.*Studio/ })).toBeEnabled();
});

it("asks again before losing server changes, then removes the account with them", async () => {
  const lost = { kind: "unsentServerChanges", count: 2, message: "2 changes haven't reached the server and will be lost." };
  const { backend } = fakeMail({ removeAccount: vi.fn().mockRejectedValueOnce(lost).mockResolvedValue({ pending: false }) });
  const user = userEvent.setup();
  render(<AccountsSection />);
  await user.click(screen.getByRole("button", { name: /Remove account/ }));
  await user.click(screen.getByRole("button", { name: /Yes, remove Work/ }));
  expect(screen.getByRole("alert")).toHaveTextContent("2 changes haven't reached the server and will be lost.");
  expect(screen.getByRole("alert")).toHaveTextContent("Your mail server keeps your mail as it was.");
  await user.click(screen.getByRole("button", { name: /Remove Work and lose them/ }));
  expect(backend.removeAccount).toHaveBeenLastCalledWith("work", 2);
  expect(ids()).not.toContain("work");
});
