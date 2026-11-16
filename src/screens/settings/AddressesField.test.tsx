import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { demoMail, setMail } from "../../services/mail";
import { fakeMail } from "../../test/fakeMail";
import type { Account, AccountAddress, ProviderId } from "../../data/types";
import { resetStore, thread, work } from "../../test/fixtures";
import { useStore } from "../../state/store";
import { AddressesField } from "./AddressesField";

const s = () => useStore.getState();
const address = (email: string, extra: Partial<AccountAddress> = {}): AccountAddress => ({
  email,
  state: "added",
  source: "you",
  seen: 0,
  blocked: false,
  ...extra,
});

const icloud: Account = {
  id: "icloud",
  name: "iCloud",
  letter: "I",
  address: "maya@icloud.com",
  senderName: "Maya Ross",
  color: "sky",
  connection: { provider: "icloud", summary: "iCloud · App password" },
  addresses: [
    address("hi@rossstudio.art", { name: "Ross Studio" }),
    address("me@mayaross.me", { source: "gmail" }),
    address("shop@mayaross.me", { state: "suggested", source: "mail", seen: 4 }),
    address("one@mayaross.me", { state: "suggested", source: "mail", seen: 1 }),
    address("old@mayaross.me", { state: "dismissed", source: "mail", blocked: true }),
  ],
};

const account = () => s().accounts.find((a) => a.id === "icloud")!;
const shown = () => render(<AddressesField account={account()} />);

beforeEach(() => resetStore([thread("x", { accountId: "icloud" })], [work, icloud]));
afterEach(() => setMail(demoMail));

describe("AddressesField", () => {
  it("lists the address the account signs in with, then the ones you send from", () => {
    shown();
    const list = screen.getByRole("list", { name: "Addresses you send from" });
    const rows = within(list).getAllByRole("listitem").map((r) => r.textContent);
    expect(rows[0]).toContain("maya@icloud.com");
    expect(rows[0]).toContain("Signs in");
    expect(rows[1]).toContain("hi@rossstudio.art");
    expect(rows[2]).toContain("From Gmail");
  });

  it("renaming one changes the name its mail shows", async () => {
    const user = userEvent.setup();
    const { rerender } = shown();
    const name = screen.getByRole("textbox", { name: "Name on mail from hi@rossstudio.art" });
    expect(name).toHaveValue("Ross Studio");
    await user.clear(name);
    rerender(<AddressesField account={account()} />);
    expect(screen.getByRole("textbox", { name: "Name on mail from hi@rossstudio.art" })).toHaveAttribute("placeholder", "Maya Ross");
    expect(account().addresses?.[0].name).toBeUndefined();
  });

  it("keeps a space you type at the end of a name, so you can go on to the next word", async () => {
    const user = userEvent.setup();
    function Live() {
      const live = useStore((st) => st.accounts.find((a) => a.id === "icloud")!);
      return <AddressesField account={live} />;
    }
    render(<Live />);
    const name = screen.getByRole("textbox", { name: "Name on mail from hi@rossstudio.art" });
    await user.type(name, " ");
    expect(name).toHaveValue("Ross Studio ");
    await user.type(name, "Two");
    expect(name).toHaveValue("Ross Studio Two");
    expect(account().addresses?.[0].name).toBe("Ross Studio Two");
  });

  it("shows a name that changed somewhere else, not what you last typed", () => {
    const { rerender } = shown();
    s().renameAddress("icloud", "hi@rossstudio.art", "Ross Studio Ltd");
    rerender(<AddressesField account={account()} />);
    expect(screen.getByRole("textbox", { name: "Name on mail from hi@rossstudio.art" })).toHaveValue("Ross Studio Ltd");
  });

  it("removes one", async () => {
    const { rerender } = shown();
    await userEvent.click(screen.getByRole("button", { name: "Remove hi@rossstudio.art" }));
    rerender(<AddressesField account={account()} />);
    expect(screen.queryByText("hi@rossstudio.art")).not.toBeInTheDocument();
  });

  it("asks about addresses found in your mail: send from it, not mine, or block it", async () => {
    const user = userEvent.setup();
    const { rerender } = shown();
    const found = screen.getByRole("list", { name: "Found in your mail" });
    expect(found).toHaveTextContent("shop@mayaross.me4 emails came to it");
    expect(found).toHaveTextContent("one@mayaross.me1 email came to it");
    await user.click(within(found).getByRole("button", { name: "Send from shop@mayaross.me" }));
    await user.click(within(found).getByRole("button", { name: "one@mayaross.me isn’t mine" }));
    rerender(<AddressesField account={account()} />);
    expect(screen.queryByRole("list", { name: "Found in your mail" })).not.toBeInTheDocument();
    expect(account().addresses?.find((a) => a.email === "shop@mayaross.me")?.state).toBe("added");
  });

  it("blocks a found address, and unblocks a blocked one", async () => {
    const user = userEvent.setup();
    const { rerender } = shown();
    await user.click(screen.getByRole("button", { name: "Block mail to shop@mayaross.me" }));
    rerender(<AddressesField account={account()} />);
    const blocked = screen.getByRole("list", { name: "Blocked addresses" });
    expect(within(blocked).getAllByRole("listitem").map((r) => r.textContent)).toEqual([
      expect.stringContaining("shop@mayaross.me"),
      expect.stringContaining("old@mayaross.me"),
    ]);
    await user.click(screen.getByRole("button", { name: "Unblock old@mayaross.me" }));
    rerender(<AddressesField account={account()} />);
    expect(screen.queryByText("old@mayaross.me")).not.toBeInTheDocument();
  });

  it("adds an address you type, with Enter or the button, and says when it isn't one", async () => {
    const user = userEvent.setup();
    const { rerender } = shown();
    const box = screen.getByRole("textbox", { name: "Another address" });
    await user.type(box, "studio{Enter}");
    expect(screen.getByText("That isn’t a full address, like hi@yourdomain.com")).toBeInTheDocument();
    expect(box).toHaveAttribute("aria-invalid", "true");
    await user.type(box, "@rossstudio.art{Enter}");
    rerender(<AddressesField account={account()} />);
    expect(account().addresses?.at(-1)).toMatchObject({ email: "studio@rossstudio.art", state: "added", source: "you" });
    expect(screen.getByRole("textbox", { name: "Another address" })).toHaveValue("");
    expect(screen.queryByText(/isn’t a full address/)).not.toBeInTheDocument();
    await user.type(screen.getByRole("textbox", { name: "Another address" }), "x@rossstudio.art");
    await user.click(screen.getByRole("button", { name: "Add" }));
    rerender(<AddressesField account={account()} />);
    expect(account().addresses?.at(-1)?.email).toBe("x@rossstudio.art");
  });

  it("says the account's own address is already there", async () => {
    shown();
    await userEvent.type(screen.getByRole("textbox", { name: "Another address" }), "Maya@iCloud.com{Enter}");
    expect(screen.getByText("That’s the address iCloud signs in with")).toBeInTheDocument();
  });

  it("looks for addresses in your mail when asked, and says so while it looks", async () => {
    let answer!: (a: Account) => void;
    fakeMail({ discoverAddresses: vi.fn(() => new Promise<Account>((done) => (answer = done))) });
    shown();
    await userEvent.click(screen.getByRole("button", { name: "Look in my mail" }));
    expect(screen.getByRole("button", { name: "Looking…" })).toBeDisabled();
    answer(icloud);
    expect(await screen.findByRole("button", { name: "Look in my mail" })).toBeEnabled();
  });

  it.each<[ProviderId | undefined, RegExp]>([
    ["icloud", /icloud\.com first/],
    ["gmail", /Send mail as/],
    ["fastmail", /Fastmail first/],
    ["resend", /domain you verified in Resend/],
    ["outlook", /provider has to let/],
    [undefined, /provider has to let/],
  ])("tells a %s account where its addresses come from", (provider, text) => {
    const a = { ...icloud, addresses: [], connection: provider && { provider, summary: "" } };
    render(<AddressesField account={a} />);
    expect(screen.getByText(text)).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "Found in your mail" })).not.toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "Blocked addresses" })).not.toBeInTheDocument();
  });
});
