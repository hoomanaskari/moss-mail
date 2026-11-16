import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ReadingLayout } from "./ReadingLayout";

describe("ReadingLayout", () => {
  it("puts the email in the reading column with the footer pinned below it", () => {
    const { container } = render(
      <ReadingLayout footer={<div>Reply bar</div>}>
        <article>Email</article>
      </ReadingLayout>,
    );
    const layout = container.firstElementChild!;
    const column = layout.firstElementChild!.firstElementChild!;
    expect([...column.children].map((c) => c.textContent)).toEqual(["Email", "", "Reply bar"]);
    // The column is all there is: no side panel beside it.
    expect(layout.children).toHaveLength(1);
  });

  it("renders without a footer, compact or not", () => {
    const { rerender } = render(
      <ReadingLayout compact>
        <article>Email</article>
      </ReadingLayout>,
    );
    expect(screen.getByText("Email")).toBeInTheDocument();
    rerender(
      <ReadingLayout>
        <article>Email</article>
      </ReadingLayout>,
    );
    expect(screen.getByText("Email")).toBeInTheDocument();
  });
});
