import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { ListCard, ListRow } from "./ListCard";

it("boxes rows and marks the chosen one with the teal cursor", () => {
  render(
    <ListCard as="ul" aria-label="Domains" className="extra">
      <ListRow as="li" selected>
        One
      </ListRow>
      <ListRow as="li">Two</ListRow>
    </ListCard>,
  );
  const list = screen.getByRole("list", { name: "Domains" });
  expect(list).toHaveClass("extra");
  const [one, two] = screen.getAllByRole("listitem");
  expect(one.querySelector("[aria-hidden]")).not.toBeNull();
  expect(two.querySelector("[aria-hidden]")).toBeNull();
});

it("is a div by default", () => {
  render(
    <ListCard role="listbox" aria-label="Accounts">
      <ListRow role="option" aria-selected>
        Work
      </ListRow>
    </ListCard>,
  );
  expect(screen.getByRole("listbox").tagName).toBe("DIV");
  expect(screen.getByRole("option").tagName).toBe("DIV");
});
