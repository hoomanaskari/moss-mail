import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { ServerSettings } from "./ServerSettings";

const servers = {
  incoming: { host: "posteo.de", port: "993", security: "ssl" as const },
  outgoing: { host: "", port: "465", security: "ssl" as const },
};

it("shows what was found and lets you change any of it", async () => {
  const onChange = vi.fn();
  render(<ServerSettings found servers={servers} domain="posteo.de" onChange={onChange} />);
  expect(screen.getByText("Server settings found")).toBeInTheDocument();
  expect(screen.getByRole("textbox", { name: "Incoming · IMAP server" })).toHaveValue("posteo.de");
  const user = userEvent.setup();
  await user.type(screen.getByRole("textbox", { name: "Outgoing · SMTP server" }), "x");
  expect(onChange).toHaveBeenLastCalledWith("outgoing", { host: "x" });
  await user.type(screen.getByRole("textbox", { name: "Incoming · IMAP port" }), "4");
  expect(onChange).toHaveBeenLastCalledWith("incoming", { port: "9934" });
  await user.selectOptions(screen.getByRole("combobox", { name: "Outgoing · SMTP security" }), "starttls");
  expect(onChange).toHaveBeenLastCalledWith("outgoing", { security: "starttls" });
});

it("asks you to type them when none were found, suggesting the usual names", () => {
  render(<ServerSettings found={false} servers={servers} domain="ross-studio.co" onChange={() => {}} />);
  expect(screen.getByText("Type your server settings")).toBeInTheDocument();
  expect(screen.getByText(/ross-studio.co IMAP settings/)).toBeInTheDocument();
  expect(screen.getByRole("textbox", { name: "Outgoing · SMTP server" })).toHaveAttribute("placeholder", "smtp.ross-studio.co");
});
