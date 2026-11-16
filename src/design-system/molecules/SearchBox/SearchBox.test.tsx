import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { SearchBox } from "./SearchBox";

it("is a focused search field with the result count beside it", async () => {
  const onChange = vi.fn();
  render(<SearchBox value="" onChange={onChange} count="7 results" />);
  const box = screen.getByRole("searchbox", { name: "Search all mail" });
  expect(box).toHaveFocus();
  await userEvent.type(box, "b");
  expect(onChange).toHaveBeenCalledWith("b");
  expect(screen.getByText("7 results")).toBeInTheDocument();
});
