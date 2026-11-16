import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { ProgressSummary } from "./ProgressSummary";

const fill = (ring: HTMLElement) => {
  const [filled, full] = ring.querySelectorAll("circle")[1].getAttribute("stroke-dasharray")!.split(" ").map(Number);
  return filled / full;
};

describe("ProgressSummary", () => {
  it("says how many are done and fills the ring to match", () => {
    render(<ProgressSummary done={2} total={8} eta="Inbox zero in ~12 min" />);
    expect(screen.getByText("2 of 8 done")).toBeInTheDocument();
    expect(screen.getByText("Inbox zero in ~12 min")).toBeInTheDocument();
    const ring = screen.getByRole("img", { name: "2 of 8 done" });
    expect(fill(ring)).toBeCloseTo(0.25);
  });

  it("a day with nothing to do shows a full ring", () => {
    render(<ProgressSummary done={0} total={0} eta="Nothing today" />);
    expect(fill(screen.getByRole("img", { name: "0 of 0 done" }))).toBe(1);
  });

  it("is plain text until it has somewhere to go", () => {
    render(<ProgressSummary done={2} total={8} eta="Inbox zero in ~12 min" />);
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });

  it("opens what you've done when it can", async () => {
    const onOpen = vi.fn();
    render(<ProgressSummary done={2} total={8} eta="Inbox zero in ~12 min" onOpen={onOpen} />);
    await userEvent.click(screen.getByRole("button", { name: "2 of 8 done. See what you’ve done" }));
    expect(onOpen).toHaveBeenCalledOnce();
  });
});
