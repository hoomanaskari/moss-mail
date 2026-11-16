import { render, screen, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { work } from "../../../test/fixtures";
import { ThreadHeader } from "./ThreadHeader";

describe("ThreadHeader", () => {
  it("shows the inbox it is in and the subject as the page heading", () => {
    render(<ThreadHeader account={work} subject="Q3 budget" />);
    expect(screen.getByText("Work · maya@northwind.co")).toBeInTheDocument();
    expect(screen.getByText("W")).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 1, name: "Q3 budget" })).toBeInTheDocument();
  });

  it("shows each label", () => {
    render(<ThreadHeader account={work} subject="Q3 budget" labels={["Budget", "Finance"]} />);
    expect(screen.getByText("Budget")).toBeInTheDocument();
    expect(screen.getByText("Finance")).toBeInTheDocument();
  });

  it("shows the AI ask with its due date", () => {
    render(<ThreadHeader account={work} subject="Q3 budget" ask="Approve the budget." askDue="Due Wed" />);
    expect(screen.getByText("Approve the budget.")).toBeInTheDocument();
    expect(screen.getByText("Due Wed")).toBeInTheDocument();
  });

  it("shows the ask without a due date when there is none", () => {
    const { container } = render(<ThreadHeader account={work} subject="Q3 budget" ask="Approve the budget." />);
    expect(screen.getByText("Approve the budget.")).toBeInTheDocument();
    expect(container.querySelectorAll("p + span")).toHaveLength(0);
  });

  it("leaves out the ask, and its due date, when there is no ask", () => {
    const { container } = render(<ThreadHeader account={work} subject="Q3 budget" askDue="Due Wed" />);
    expect(container.querySelector("p")).toBeNull();
    expect(screen.queryByText("Due Wed")).not.toBeInTheDocument();
  });

  it("says when you moved it to its tab, beside why Moss put it there", () => {
    render(<ThreadHeader account={work} subject="Q3 budget" moved="Important" reasons={["Due Fri · says “by Friday”"]} />);
    expect(screen.getByText("You moved this to Important")).toBeInTheDocument();
    expect(screen.getByRole("list", { name: "Why Moss put it here" })).toHaveTextContent("Due Fri");
  });

  it("says why Moss put it where it is, one short line for each decision", () => {
    const { rerender } = render(
      <ThreadHeader account={work} subject="Q3 budget" reasons={["Needs your reply · AI, 95% sure", "Due Fri · says “by Friday”"]} />,
    );
    const why = screen.getByRole("list", { name: "Why Moss put it here" });
    expect(within(why).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["Needs your reply · AI, 95% sure", "Due Fri · says “by Friday”"]);
    rerender(<ThreadHeader account={work} subject="Q3 budget" reasons={[]} />);
    expect(screen.queryByRole("list", { name: "Why Moss put it here" })).not.toBeInTheDocument();
    rerender(<ThreadHeader account={work} subject="Q3 budget" />);
    expect(screen.queryByText(/AI, \d+% sure/)).not.toBeInTheDocument();
  });
});
