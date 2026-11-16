import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { Field } from "./Field";

it("labels its input, with help and a note", () => {
  render(
    <Field label="API key" htmlFor="key" help="Create one at resend.com/api-keys" note="Found 3 domains on this key.">
      <input id="key" />
    </Field>,
  );
  expect(screen.getByLabelText("API key")).toBeInTheDocument();
  expect(screen.getByText("Create one at resend.com/api-keys")).toBeInTheDocument();
  expect(screen.getByText("Found 3 domains on this key.")).toBeInTheDocument();
  expect(screen.queryByRole("alert")).toBeNull();
});

it("announces an error note", () => {
  render(
    <Field label="Password" htmlFor="pw" note="Use an app-specific password." noteTone="error">
      <input id="pw" />
    </Field>,
  );
  expect(screen.getByRole("alert")).toHaveTextContent("Use an app-specific password.");
});

it("has no note unless given one", () => {
  const { container } = render(
    <Field label="Email" htmlFor="e">
      <input id="e" />
    </Field>,
  );
  expect(container.querySelector("svg")).toBeNull();
});
