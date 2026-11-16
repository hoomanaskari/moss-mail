import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { Split } from "../../../data/types";
import { SplitTabs } from "./SplitTabs";

const splits: Split[] = [
  { id: "important", label: "Important" },
  { id: "other", label: "Other" },
  { id: "calendar", label: "Calendar" },
];

describe("SplitTabs", () => {
  it("renders one tab per split inside a labelled tablist", () => {
    render(<SplitTabs splits={splits} counts={{}} selected="important" onSelect={() => {}} />);
    expect(screen.getByRole("tablist", { name: "Inbox splits" })).toBeInTheDocument();
    expect(screen.getAllByRole("tab").map((t) => t.textContent)).toEqual(["Important0", "Other0", "Calendar0"]);
  });

  it("shows each split's count, and 0 when it has none", () => {
    render(<SplitTabs splits={splits} counts={{ important: 4, other: 12 }} selected="important" onSelect={() => {}} />);
    expect(screen.getByRole("tab", { name: /Important/ })).toHaveTextContent("4");
    expect(screen.getByRole("tab", { name: /Other/ })).toHaveTextContent("12");
    expect(screen.getByRole("tab", { name: /Calendar/ })).toHaveTextContent("0");
  });

  it("passes each split's unread count to its tab", () => {
    render(<SplitTabs splits={splits} counts={{ other: 3 }} unread={{ other: 1 }} selected="important" onSelect={() => {}} />);
    expect(screen.getByRole("tab", { name: /Other/ })).toHaveAccessibleName(/1 unread/);
    expect(screen.getByRole("tab", { name: /Important/ })).not.toHaveAccessibleName(/unread/);
  });

  it("marks only the selected split", () => {
    render(<SplitTabs splits={splits} counts={{}} selected="other" onSelect={() => {}} />);
    expect(screen.getByRole("tab", { selected: true })).toHaveTextContent("Other");
    expect(screen.getAllByRole("tab", { selected: false })).toHaveLength(2);
  });

  it("calls onSelect with the clicked split's id", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    render(<SplitTabs splits={splits} counts={{}} selected="important" onSelect={onSelect} />);
    await user.click(screen.getByRole("tab", { name: /Calendar/ }));
    expect(onSelect).toHaveBeenCalledWith("calendar");
  });

  it("puts trailing content at the end of the row, outside the tablist", () => {
    render(<SplitTabs splits={splits} counts={{}} selected="important" onSelect={() => {}} trailing={<button type="button">Drafts</button>} />);
    expect(screen.getByRole("button", { name: "Drafts" })).toBeInTheDocument();
    expect(within(screen.getByRole("tablist")).queryByRole("button", { name: "Drafts" })).not.toBeInTheDocument();
  });
});
