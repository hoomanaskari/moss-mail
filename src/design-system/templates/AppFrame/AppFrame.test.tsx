import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { AppFrame, AppFrameSide } from "./AppFrame";

describe("AppFrame", () => {
  it("stacks the top bar, the main body and the footer in order", () => {
    const { container } = render(
      <AppFrame topBar={<header>Top</header>} footer={<footer>Keys</footer>}>
        <p>Body</p>
      </AppFrame>,
    );
    expect(screen.getByRole("main")).toContainElement(screen.getByText("Body"));
    const frame = container.firstElementChild!;
    expect([...frame.children].map((c) => c.textContent)).toEqual(["Top", "Body", "Keys"]);
  });

  it("works without a footer", () => {
    const { container } = render(
      <AppFrame topBar={<header>Top</header>}>
        <p>Body</p>
      </AppFrame>,
    );
    expect(container.firstElementChild!.children).toHaveLength(2);
    expect(screen.queryByRole("contentinfo")).not.toBeInTheDocument();
  });

  it("puts what the app gives as its side beside the body, between the top bar and the footer", () => {
    const { container } = render(
      <AppFrameSide.Provider value={<aside>Ask</aside>}>
        <AppFrame topBar={<header>Top</header>} footer={<footer>Keys</footer>}>
          <p>Body</p>
        </AppFrame>
      </AppFrameSide.Provider>,
    );
    const frame = container.firstElementChild!;
    expect([...frame.children].map((c) => c.textContent)).toEqual(["Top", "BodyAsk", "Keys"]);
    expect(screen.getByRole("complementary").previousElementSibling).toBe(screen.getByRole("main"));
  });

  it("marks the frame while a side is open, so the top bar is lined above it; not without one", () => {
    const { container, rerender } = render(
      <AppFrame topBar={<header>Top</header>}>
        <p>Body</p>
      </AppFrame>,
    );
    expect(container.firstElementChild).not.toHaveAttribute("data-side");
    rerender(
      <AppFrameSide.Provider value={<aside>Ask</aside>}>
        <AppFrame topBar={<header>Top</header>}>
          <p>Body</p>
        </AppFrame>
      </AppFrameSide.Provider>,
    );
    expect(container.firstElementChild).toHaveAttribute("data-side");
  });
});
