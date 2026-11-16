import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { StatusCard } from "./StatusCard";

it("shows a waiting spinner or a done tick with a title, a line and an action", () => {
  const { rerender } = render(<StatusCard kind="waiting" title="Waiting for you in the browser" sub="Finish signing in." action={<button type="button">Open again</button>} />);
  expect(screen.getByRole("status")).toHaveTextContent("Waiting for you in the browser");
  expect(screen.getByRole("img", { name: "Waiting" })).toBeInTheDocument();
  rerender(<StatusCard kind="ready" title="You can start using Moss now." sub="Downloading carries on." />);
  expect(screen.queryByRole("img", { name: "Waiting" })).toBeNull();
  expect(screen.getByRole("status")).toHaveTextContent("You can start using Moss now.");
});
