import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { personal, work } from "../../../test/fixtures";
import { AccountSwitcher, type AccountSwitcherProps } from "./AccountSwitcher";

// jsdom has no Element.scrollTo / scrollBy; the switcher calls both.
const scrollTo = vi.fn();
const scrollBy = vi.fn();
beforeEach(() => {
  scrollTo.mockReset();
  scrollBy.mockReset();
  Element.prototype.scrollTo = scrollTo as unknown as Element["scrollTo"];
  Element.prototype.scrollBy = scrollBy as unknown as Element["scrollBy"];
});
afterEach(() => {
  delete (Element.prototype as Partial<Element>).scrollTo;
  delete (Element.prototype as Partial<Element>).scrollBy;
  vi.unstubAllGlobals();
});

function setup(overrides: Partial<AccountSwitcherProps> = {}) {
  const props: AccountSwitcherProps = {
    accounts: [work, personal],
    counts: { all: 7, work: 5, personal: 2 },
    selected: "all",
    onSelect: vi.fn(),
    onAdd: vi.fn(),
    ...overrides,
  };
  const view = render(<AccountSwitcher {...props} />);
  return { ...view, props, tablist: screen.getByRole("tablist", { name: "Accounts" }) };
}

/** Gives an element fake layout, since jsdom reports zero for everything. */
function layout(el: Element, sizes: Record<string, number>) {
  for (const [key, value] of Object.entries(sizes)) Object.defineProperty(el, key, { value, configurable: true, writable: true });
}

function rect(el: Element, left: number) {
  el.getBoundingClientRect = () => ({ left }) as DOMRect;
}

describe("tabs", () => {
  it("shows All accounts first, then every account with its count", () => {
    setup();
    const tabs = screen.getAllByRole("tab");
    expect(tabs).toHaveLength(3);
    expect(tabs[0]).toHaveTextContent(/^All accountsEverything in one place7$/);
    expect(tabs[1]).toHaveTextContent(/Workmaya@northwind\.co5$/);
    expect(tabs[2]).toHaveTextContent(/Personalmaya\.ross@fastmail\.com2$/);
  });

  it("marks the selected account", () => {
    setup({ selected: "personal" });
    expect(screen.getByRole("tab", { name: /Personal/ })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: /All accounts/ })).toHaveAttribute("aria-selected", "false");
  });

  it("shows zero when an account has no count", () => {
    setup({ counts: {} });
    for (const tab of screen.getAllByRole("tab")) expect(tab).toHaveTextContent(/0$/);
  });

  it("calls onSelect with all or the account id", async () => {
    const user = userEvent.setup();
    const { props } = setup();
    await user.click(screen.getByRole("tab", { name: /Work/ }));
    await user.click(screen.getByRole("tab", { name: /All accounts/ }));
    expect(props.onSelect).toHaveBeenNthCalledWith(1, "work");
    expect(props.onSelect).toHaveBeenNthCalledWith(2, "all");
  });

  it("calls onAdd from the add button", async () => {
    const { props } = setup();
    await userEvent.setup().click(screen.getByRole("button", { name: "Add account" }));
    expect(props.onAdd).toHaveBeenCalledOnce();
  });

  it("shows trailing content only when given", () => {
    const { rerender, props } = setup();
    expect(screen.queryByText("3 of 7")).not.toBeInTheDocument();
    rerender(<AccountSwitcher {...props} trailing={<span>3 of 7</span>} />);
    expect(screen.getByText("3 of 7")).toBeInTheDocument();
  });
});

describe("shortcut hints", () => {
  it("while shown, each tab shows its key: ⌘0 for All accounts, then ⌘1, ⌘2…", () => {
    setup({ showShortcuts: true });
    const tabs = screen.getAllByRole("tab");
    expect(tabs[0]).toHaveTextContent("Ctrl+0");
    expect(tabs[1]).toHaveTextContent("Ctrl+1");
    expect(tabs[2]).toHaveTextContent("Ctrl+2");
  });

  it("only the first nine accounts have a key", () => {
    const many = Array.from({ length: 10 }, (_, i) => ({ ...work, id: `a${i}`, name: `Account ${i + 1}` }));
    setup({ accounts: many, showShortcuts: true });
    expect(screen.getByRole("tab", { name: /Account 9(?!\d)/ })).toHaveTextContent("Ctrl+9");
    expect(screen.getByRole("tab", { name: /Account 10/ }).querySelector("kbd")).toBeNull();
  });

  it("are hidden otherwise", () => {
    setup();
    for (const tab of screen.getAllByRole("tab")) expect(tab.querySelector("kbd")).toBeNull();
  });
});

describe("overflow", () => {
  it("has no arrows when everything fits", () => {
    setup();
    expect(screen.queryByRole("button", { name: /accounts$/ })).not.toBeInTheDocument();
  });

  it("shows an arrow on each side that has more, and pages by most of the width", async () => {
    const user = userEvent.setup();
    const { tablist } = setup();
    layout(tablist, { scrollWidth: 1000, clientWidth: 300, scrollLeft: 0 });
    fireEvent.scroll(tablist);
    expect(screen.getByRole("button", { name: "Show more accounts" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Show earlier accounts" })).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Show more accounts" }));
    expect(scrollBy).toHaveBeenLastCalledWith({ left: 240, behavior: "smooth" });

    tablist.scrollLeft = 300;
    fireEvent.scroll(tablist);
    await user.click(screen.getByRole("button", { name: "Show earlier accounts" }));
    expect(scrollBy).toHaveBeenLastCalledWith({ left: -240, behavior: "smooth" });

    tablist.scrollLeft = 700;
    fireEvent.scroll(tablist);
    expect(screen.queryByRole("button", { name: "Show more accounts" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Show earlier accounts" })).toBeInTheDocument();
  });

  it("pages without animation when reduced motion is on", async () => {
    vi.stubGlobal("matchMedia", (q: string) => ({ matches: q.includes("reduce") }));
    const { tablist } = setup();
    layout(tablist, { scrollWidth: 1000, clientWidth: 300, scrollLeft: 0 });
    fireEvent.scroll(tablist);
    await userEvent.setup().click(screen.getByRole("button", { name: "Show more accounts" }));
    expect(scrollBy).toHaveBeenLastCalledWith({ left: 240, behavior: "auto" });
  });

  it("turns a vertical mouse wheel into sideways scrolling", () => {
    const { tablist } = setup();
    layout(tablist, { scrollWidth: 1000, clientWidth: 300, scrollLeft: 0 });
    const notPrevented = fireEvent.wheel(tablist, { deltaY: 120, deltaX: 0 });
    expect(notPrevented).toBe(false);
    expect(tablist.scrollLeft).toBe(120);
  });

  it("leaves sideways wheels alone", () => {
    const { tablist } = setup();
    layout(tablist, { scrollWidth: 1000, clientWidth: 300, scrollLeft: 0 });
    expect(fireEvent.wheel(tablist, { deltaY: 10, deltaX: 40 })).toBe(true);
    expect(tablist.scrollLeft).toBe(0);
  });

  it("leaves the wheel alone when nothing overflows", () => {
    const { tablist } = setup();
    layout(tablist, { scrollWidth: 300, clientWidth: 300, scrollLeft: 0 });
    expect(fireEvent.wheel(tablist, { deltaY: 120 })).toBe(true);
    expect(tablist.scrollLeft).toBe(0);
  });
});

describe("bringing the selected account into view", () => {
  function prepare(tabLeft: number, tabWidth: number) {
    const view = setup({ selected: "all" });
    const { tablist } = view;
    layout(tablist, { clientWidth: 300, scrollLeft: 100 });
    rect(tablist, 0);
    const tab = screen.getByRole("tab", { name: /Personal/ });
    rect(tab, tabLeft);
    layout(tab, { offsetWidth: tabWidth });
    scrollTo.mockClear();
    return view;
  }

  it("scrolls back when the tab is hidden on the left", () => {
    const { rerender, props } = prepare(-80, 120);
    rerender(<AccountSwitcher {...props} selected="personal" />);
    // start = -80 + 100 = 20, minus the 48px fade.
    expect(scrollTo).toHaveBeenCalledWith({ left: -28, behavior: "smooth" });
  });

  it("scrolls forward when the tab is hidden on the right", () => {
    const { rerender, props } = prepare(260, 120);
    rerender(<AccountSwitcher {...props} selected="personal" />);
    // end = 260 + 100 + 120 = 480; 480 - 300 + 48.
    expect(scrollTo).toHaveBeenCalledWith({ left: 228, behavior: "smooth" });
  });

  it("does not scroll when the tab is already in view", () => {
    const { rerender, props } = prepare(80, 100);
    rerender(<AccountSwitcher {...props} selected="personal" />);
    expect(scrollTo).not.toHaveBeenCalled();
  });

  it("jumps instead of animating when reduced motion is on", () => {
    vi.stubGlobal("matchMedia", () => ({ matches: true }));
    const { rerender, props } = prepare(-80, 120);
    rerender(<AccountSwitcher {...props} selected="personal" />);
    expect(scrollTo).toHaveBeenCalledWith({ left: -28, behavior: "auto" });
    scrollTo.mockClear();
    rect(screen.getByRole("tab", { name: /Work/ }), 260);
    layout(screen.getByRole("tab", { name: /Work/ }), { offsetWidth: 120 });
    rerender(<AccountSwitcher {...props} selected="work" />);
    expect(scrollTo).toHaveBeenCalledWith({ left: 228, behavior: "auto" });
  });

  it("does nothing when the selected id matches no tab", () => {
    const { rerender, props } = prepare(-80, 120);
    rerender(<AccountSwitcher {...props} selected="gone" />);
    expect(scrollTo).not.toHaveBeenCalled();
  });
});
