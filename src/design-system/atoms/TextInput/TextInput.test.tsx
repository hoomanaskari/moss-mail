import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { TextInput } from "./TextInput";

it("is a text box you can type in, with things after the text", async () => {
  const onChange = vi.fn();
  render(<TextInput aria-label="Email" defaultValue="" onChange={onChange} trailing={<span>Key works</span>} />);
  const box = screen.getByRole("textbox", { name: "Email" });
  await userEvent.type(box, "maya");
  expect(box).toHaveValue("maya");
  expect(onChange).toHaveBeenCalledTimes(4);
  expect(screen.getByText("Key works")).toBeInTheDocument();
});

it("marks a wrong value for screen readers", () => {
  render(<TextInput aria-label="Password" type="password" invalid compact readOnly value="x" />);
  expect(screen.getByLabelText("Password")).toHaveAttribute("aria-invalid", "true");
});

it("is not marked invalid by default", () => {
  render(<TextInput aria-label="Name" />);
  expect(screen.getByRole("textbox")).not.toHaveAttribute("aria-invalid");
});
