import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Inbox } from "lucide-react";
import { describe, expect, it, vi } from "vitest";
import { EmptyState } from "./EmptyState";

describe("EmptyState", () => {
  it("announces what belongs here, with no steps by default", () => {
    render(<EmptyState icon={Inbox} title="Nothing to reply to" description="Emails that need you show up here." />);
    const status = screen.getByRole("status");
    expect(status).toHaveTextContent("Nothing to reply to");
    expect(status).toHaveTextContent("Emails that need you show up here.");
    expect(screen.queryByRole("button")).toBeNull();
  });

  it("offers each step as a button with its key", async () => {
    const compose = vi.fn();
    const later = vi.fn();
    render(
      <EmptyState
        icon={Inbox}
        title="Inbox zero"
        description="All done."
        steps={[
          { keys: "C", label: "New email", onClick: compose },
          { keys: "L", label: "See later", onClick: later },
        ]}
      />,
    );
    const buttons = screen.getAllByRole("button");
    expect(buttons.map((b) => b.textContent)).toEqual(["CNew email", "LSee later"]);
    await userEvent.click(screen.getByRole("button", { name: /See later/ }));
    expect(later).toHaveBeenCalledOnce();
    expect(compose).not.toHaveBeenCalled();
  });
});
