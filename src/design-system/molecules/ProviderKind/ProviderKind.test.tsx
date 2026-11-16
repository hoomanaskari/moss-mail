import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { ProviderKind } from "./ProviderKind";

it("names the provider, with an extra word when given", () => {
  const { rerender, container } = render(<ProviderKind letter="G" name="Gmail" />);
  expect(container).toHaveTextContent(/^GGmail$/);
  rerender(<ProviderKind letter="G" name="Gmail" extra="Advanced" />);
  expect(container).toHaveTextContent("Gmail·Advanced");
  expect(screen.getByText("Advanced")).toBeInTheDocument();
});
