import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { PersonLine } from "./PersonLine";

describe("PersonLine", () => {
  it("shows name and email, with no badge or flag by default", () => {
    const { container } = render(<PersonLine name="Lena Vogt" email="lena@northwind.co" />);
    expect(screen.getByText("Lena Vogt")).toBeInTheDocument();
    expect(screen.getByText("lena@northwind.co")).toBeInTheDocument();
    expect(container.querySelector("[aria-hidden='true']:not(svg)")).toBeNull();
  });

  it("shows your account badge and an outside flag when given", () => {
    render(
      <PersonLine name="Ines" email="ines@acme.example" account={{ letter: "W", color: "work" }} flag="Outside Northwind" />,
    );
    expect(screen.getByText("W")).toHaveAttribute("aria-hidden", "true");
    expect(screen.getByText("Outside Northwind")).toBeInTheDocument();
  });

  it("Copy and Contact call their handlers", async () => {
    const onCopy = vi.fn();
    const onContact = vi.fn();
    render(<PersonLine name="Lena Vogt" email="lena@northwind.co" onCopy={onCopy} onContact={onContact} />);
    await userEvent.click(screen.getByRole("button", { name: "Copy" }));
    expect(onCopy).toHaveBeenCalledOnce();
    expect(onContact).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "Contact" }));
    expect(onContact).toHaveBeenCalledOnce();
  });
});
