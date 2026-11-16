import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { FilePen } from "lucide-react";
import { SplitTab } from "./SplitTab";

describe("SplitTab", () => {
  it("shows label and count and reports its selection", () => {
    render(
      <>
        <SplitTab label="Important" count={5} selected onSelect={() => {}} />
        <SplitTab label="Other" count={12} selected={false} onSelect={() => {}} />
      </>,
    );
    const important = screen.getByRole("tab", { name: /Important/ });
    expect(important).toHaveTextContent("5");
    expect(important).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: /Other/ })).toHaveAttribute("aria-selected", "false");
  });

  it("says how many are unread when some are", () => {
    render(
      <>
        <SplitTab label="Newsletters" count={9} unread={2} selected={false} onSelect={() => {}} />
        <SplitTab label="Receipts" count={3} unread={0} selected={false} onSelect={() => {}} />
      </>,
    );
    expect(screen.getByRole("tab", { name: /Newsletters/ })).toHaveAccessibleName("Newsletters9, 2 unread");
    expect(screen.getByRole("tab", { name: /Receipts/ })).not.toHaveAccessibleName(/unread/);
  });

  it("calls onSelect when clicked", async () => {
    const onSelect = vi.fn();
    render(<SplitTab label="Other" count={0} selected={false} onSelect={onSelect} />);
    await userEvent.click(screen.getByRole("tab"));
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it("without a selection is a plain button that opens another view, like Drafts", async () => {
    const onSelect = vi.fn();
    render(<SplitTab label="Drafts" count={2} icon={FilePen} onSelect={onSelect} />);
    const drafts = screen.getByRole("button", { name: "Drafts2" });
    // An icon sets it apart from the tabs.
    expect(drafts.querySelector("svg")).toBeInTheDocument();
    expect(drafts).not.toHaveAttribute("aria-selected");
    expect(screen.queryByRole("tab")).not.toBeInTheDocument();
    await userEvent.click(drafts);
    expect(onSelect).toHaveBeenCalledOnce();
  });
});
