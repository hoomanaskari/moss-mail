import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { PageTitle } from "./PageTitle";

describe("PageTitle", () => {
  it("is the page's top-level heading", () => {
    render(<PageTitle>Inbox</PageTitle>);
    expect(screen.getByRole("heading", { level: 1, name: "Inbox" })).toBeInTheDocument();
  });
});
