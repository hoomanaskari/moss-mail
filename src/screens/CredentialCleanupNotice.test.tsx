import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { resetCredentialCleanup, showCredentialCleanup } from "../services/credentialCleanup";
import { CredentialCleanupNotice } from "./CredentialCleanupNotice";
beforeEach(resetCredentialCleanup);
it("stays absent when nothing needs cleanup", () => {
  render(<CredentialCleanupNotice />);
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});
it("explains pending cleanup and keeps the warning until a retry succeeds", async () => {
  let finish!: () => void;
  fakeMail({ retryCredentialCleanup: vi.fn(() => new Promise<{ pending: boolean }>(resolve => { finish = () => resolve({ pending: false }); })) });
  showCredentialCleanup({ pending: true });
  const user = userEvent.setup(); render(<CredentialCleanupNotice />);
  expect(screen.getByRole("status")).toHaveTextContent("Removed accounts stay removed.");
  await user.click(screen.getByRole("button", { name: "Retry credential cleanup" }));
  expect(screen.getByRole("button", { name: "Checking…" })).toBeDisabled();
  await act(async () => finish());
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});
it("offers another attempt after a failed retry without displaying sensitive error text", async () => {
  fakeMail({ retryCredentialCleanup: vi.fn().mockRejectedValue(new Error("private fixture")) });
  showCredentialCleanup({ pending: true });
  const user = userEvent.setup(); render(<CredentialCleanupNotice />);
  await user.click(screen.getByRole("button", { name: "Retry credential cleanup" }));
  expect(screen.getByRole("status")).toHaveTextContent("Cannot check credential cleanup.");
  expect(screen.getByRole("status")).not.toHaveTextContent("private fixture");
  expect(screen.getByRole("button", { name: "Retry credential cleanup" })).toBeEnabled();
});
