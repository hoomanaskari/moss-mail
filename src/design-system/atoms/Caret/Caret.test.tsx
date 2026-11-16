import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Caret } from "./Caret";

describe("Caret", () => {
  it("is 18px tall by default and hidden from screen readers", () => {
    const { container } = render(<Caret />);
    const caret = container.firstElementChild as HTMLElement;
    expect(caret).toHaveAttribute("aria-hidden", "true");
    expect(caret.style.height).toBe("18px");
  });

  it("takes a custom height", () => {
    const { container } = render(<Caret height={30} />);
    expect((container.firstElementChild as HTMLElement).style.height).toBe("30px");
  });
});
