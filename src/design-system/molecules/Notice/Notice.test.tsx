import { render, screen } from "@testing-library/react";
import { Info } from "lucide-react";
import { describe, expect, it } from "vitest";
import { Notice, type NoticeVariant } from "./Notice";

describe("Notice", () => {
  it("a warning is announced as an alert in the due colour", () => {
    render(
      <Notice variant="warn" icon={Info} action={<button type="button">Attach file</button>}>
        You mention attached notes
      </Notice>,
    );
    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent("You mention attached notes");
    expect(alert.querySelector("svg")).toHaveAttribute("stroke", "var(--due)");
    expect(screen.getByRole("button", { name: "Attach file" })).toBeInTheDocument();
  });

  it("a caution is in the due colour but not an alert, and a footnote has a smaller icon", () => {
    render(
      <>
        <Notice variant="caution" icon={Info}>
          Google signs you out every 7 days
        </Notice>
        <Notice variant="footnote" icon={Info}>
          Keychain
        </Notice>
      </>,
    );
    expect(screen.queryByRole("alert")).toBeNull();
    const caution = screen.getByText(/7 days/).parentElement!;
    expect(caution.querySelector("svg")).toHaveAttribute("stroke", "var(--due)");
    expect(screen.getByText("Keychain").parentElement!.querySelector("svg")).toHaveAttribute("width", "14");
  });

  it.each<[NoticeVariant, string]>([
    ["card", "var(--ink-2)"],
    ["inset", "var(--ink-2)"],
    ["plain", "var(--ink-3)"],
    ["note", "var(--ink-2)"],
    ["footnote", "var(--ink-3)"],
  ])("a %s notice is not an alert and uses %s", (variant, stroke) => {
    render(
      <Notice variant={variant} icon={Info} className="extra">
        Goes to 4 people
      </Notice>,
    );
    expect(screen.queryByRole("alert")).toBeNull();
    const notice = screen.getByText("Goes to 4 people").parentElement!;
    expect(notice).toHaveClass("extra");
    expect(notice.querySelector("svg")).toHaveAttribute("stroke", stroke);
  });
});
