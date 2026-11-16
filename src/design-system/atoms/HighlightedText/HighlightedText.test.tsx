import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { HighlightedText } from "./HighlightedText";

it("marks the matched words and keeps the spaces around them", () => {
  const { container } = render(
    <HighlightedText
      parts={[
        { text: "Q4 design ", match: false },
        { text: "budget", match: true },
        { text: " review", match: false },
      ]}
    />,
  );
  expect(container).toHaveTextContent("Q4 design budget review", { normalizeWhitespace: false });
  expect(screen.getByText("budget").tagName).toBe("MARK");
});
