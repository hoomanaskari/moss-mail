import { recoveryScenes } from "../dev/recoveryScenes";
import { useStore } from "../state/store";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import { fakeMail } from "../test/fakeMail";
import { draft } from "../test/fixtures";
import { DeliveryAttention } from "./DeliveryAttention";
import type { DeliveryAttentionEntry, DeliveryAttentionPage } from "../services/deliveryAttention";

const accepted: DeliveryAttentionEntry = { id: "accepted" as const, accountId: "work", state: "accepted" as const, interruptedFrom: null, createdAt: 1, fileCount: 0, fileBytes: 0 };
it("requires explicit acknowledgement before stopping accepted repair and never offers another send", async () => {
  const { backend } = fakeMail({ deliveryAttention: vi.fn(async () => ({ entries: [accepted], next: null })), resolveDelivery: vi.fn(async () => ({ status: "resolved" as const, recoveryId: null })) });
  const onRecoveryChanged = vi.fn(); const user = userEvent.setup();
  render(<DeliveryAttention onRecoveryChanged={onRecoveryChanged} />);
  await user.click(await screen.findByRole("button", { name: /^Review delivery for / }));
  expect(await screen.findByText(/server accepted this email/)).toBeVisible();
  expect(screen.queryByRole("button", { name: "Keep a recovery copy" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Discard recovery files" }));
  await user.click(screen.getByRole("checkbox", { name: "I understand I am discarding recovery files without creating a copy" }));
  expect(backend.resolveDelivery).not.toHaveBeenCalled();
  expect(screen.getByText(/only local copy/)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Confirm review" }));
  await waitFor(() => expect(screen.queryByRole("button", { name: "Confirm review" })).not.toBeInTheDocument());
  expect(onRecoveryChanged).not.toHaveBeenCalled();
  expect(backend.resolveDelivery).toHaveBeenCalledWith("accepted", "reviewed");
});

it("keeps damaged uncertain content reviewable without claiming it was sent", async () => {
  fakeMail({ deliveryAttention: vi.fn(async () => ({ entries: [{ ...accepted, state: "uncertain" as const }], next: null })), deliveryAttentionDraft: vi.fn(async () => { throw new Error("fixture"); }) });
  const user = userEvent.setup(); render(<DeliveryAttention onRecoveryChanged={vi.fn()} />);
  await user.click(await screen.findByRole("button", { name: /^Review delivery for / }));
  expect(await screen.findByRole("alert")).toHaveTextContent("saved content cannot be previewed");
  expect(screen.getByText(/Check Sent before sending another copy/)).toBeVisible();
  expect(screen.getByRole("button", { name: "Keep a recovery copy" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Discard recovery files" })).toBeEnabled();
});

it("copies an unsent snapshot only after confirmation and refreshes the list", async () => {
  useStore.setState({ accounts: [] });
  const snapshot = { id: "draft", mode: "new" as const, fromAccountId: "work", to: [], cc: [], bcc: [], subject: "Saved subject", body: "<script>plain text only</script>", bodyHtml: "", remindIfNoReply: false, updatedAt: 1, signatureId: null, fieldsOpen: true, showCc: false, showBcc: false, attachments: [] };
  const { backend } = fakeMail({ deliveryAttention: vi.fn(async () => ({ entries: [{ ...accepted, state: "held" as const }], next: null })), deliveryAttentionDraft: vi.fn(async () => snapshot), resolveDelivery: vi.fn(async () => ({ status: "resolved" as const, recoveryId: "copy" })) });
  const user = userEvent.setup(); const onRecoveryChanged = vi.fn(); render(<DeliveryAttention onRecoveryChanged={onRecoveryChanged} />);
  await user.click(await screen.findByRole("button", { name: /^Review delivery for / }));
  expect(await screen.findByText(snapshot.body)).toBeVisible();
  expect(screen.getByText("Saved subject")).toBeVisible();
  expect(screen.getByText(/email was not submitted/)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Keep a recovery copy" }));
  expect(backend.resolveDelivery).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Keep reviewing" }));
  expect(screen.queryByRole("button", { name: "Confirm review" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Keep a recovery copy" }));
  await user.click(screen.getByRole("button", { name: "Confirm review" }));
  await waitFor(() => expect(onRecoveryChanged).toHaveBeenCalledOnce());
  expect(backend.resolveDelivery).toHaveBeenCalledWith("accepted", "copied");
});

it("retains the selected action after ambiguous failure and refuses an in-progress delivery", async () => {
  const resolve = vi.fn().mockRejectedValueOnce(new Error("lost reply")).mockResolvedValueOnce({ status: "sending" }).mockResolvedValueOnce({ status: "alreadyFinished" });
  fakeMail({ deliveryAttention: vi.fn(async () => ({ entries: [accepted], next: null })), resolveDelivery: resolve });
  const user = userEvent.setup(); const onRecoveryChanged = vi.fn(); render(<DeliveryAttention onRecoveryChanged={onRecoveryChanged} />);
  await user.click(await screen.findByRole("button", { name: /^Review delivery for / }));
  await user.click(screen.getByRole("button", { name: "Discard recovery files" }));
  await user.click(screen.getByRole("checkbox", { name: "I understand I am discarding recovery files without creating a copy" }));
  await user.click(screen.getByRole("button", { name: "Confirm review" }));
  expect(await screen.findByText(/Could not confirm the review/)).toBeVisible();
  expect(screen.queryByText(/separate recovery copy may/)).not.toBeInTheDocument();
  expect(onRecoveryChanged).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Confirm review" }));
  expect(await screen.findByText(/delivery is still in progress/)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Confirm review" }));
  await waitFor(() => expect(screen.queryByRole("button", { name: "Confirm review" })).not.toBeInTheDocument());
  expect(onRecoveryChanged).not.toHaveBeenCalled();
  expect(resolve.mock.calls).toEqual([["accepted", "reviewed"], ["accepted", "reviewed"], ["accepted", "reviewed"]]);
});

it("retries list failures and pages using the returned cursor", async () => {
  const next = { createdAt: 4, id: "next" };
  const list = vi.fn().mockRejectedValueOnce(new Error("offline")).mockResolvedValueOnce({ entries: [accepted], next }).mockResolvedValueOnce({ entries: [], next: null }).mockResolvedValue({ entries: [], next: null });
  fakeMail({ deliveryAttention: list }); const user = userEvent.setup(); render(<DeliveryAttention onRecoveryChanged={vi.fn()} />);
  expect(await screen.findByRole("alert")).toHaveTextContent("Cannot check deliveries");
  expect(screen.queryByText("Checking deliveries…")).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Refresh deliveries" }));
  await user.click(await screen.findByRole("button", { name: "Next deliveries" }));
  await waitFor(() => expect(list).toHaveBeenLastCalledWith(next));
  expect(screen.getByRole("region", { name: "Delivery recovery" })).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Refresh deliveries" }));
  await waitFor(() => expect(screen.queryByRole("region", { name: "Delivery recovery" })).not.toBeInTheDocument());
});


it("closes a review and ignores its late preview, including after unmount", async () => {
  let finish!: (value: undefined) => void;
  const preview = vi.fn(() => new Promise<undefined>((resolve) => { finish = resolve; }));
  fakeMail({ deliveryAttention: vi.fn(async () => ({ entries: [{ ...accepted, state: "uncertain" as const, interruptedFrom: "held" as const }], next: null })), deliveryAttentionDraft: preview });
  const user = userEvent.setup(); const view = render(<DeliveryAttention onRecoveryChanged={vi.fn()} />);
  await user.click(await screen.findByRole("button", { name: /^Review delivery for / }));
  expect(screen.getByRole("button", { name: "Discard recovery files" })).toBeDisabled();
  expect(screen.getByText(/email was not submitted/)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Close delivery review" }));
  await act(async () => finish(undefined));
  expect(screen.queryByText(/delivery changed/)).not.toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: /^Review delivery for / }));
  view.unmount(); await act(async () => finish(undefined));
  expect(preview).toHaveBeenCalledTimes(2);
});

it("does not revive a list or error after its screen unmounts", async () => {
  for (const rejects of [false, true]) {
    let settle!: () => void;
    fakeMail({ deliveryAttention: vi.fn(() => new Promise<DeliveryAttentionPage>((resolve, reject) => { settle = () => rejects ? reject(new Error("late")) : resolve({ entries: [accepted], next: null }); })) });
    const view = render(<DeliveryAttention onRecoveryChanged={vi.fn()} />);
    expect(screen.queryByRole("region", { name: "Delivery recovery" })).not.toBeInTheDocument();
    view.unmount(); await act(async () => settle());
    expect(screen.queryByRole("region", { name: "Delivery recovery" })).not.toBeInTheDocument();
  }
});

it("keeps one review request in flight and renders an untitled saved snapshot", async () => {
  let finish!: (value: { status: "resolved"; recoveryId: null }) => void;
  const resolve = vi.fn(() => new Promise<{ status: "resolved"; recoveryId: null }>((done) => { finish = done; }));
  const snapshot = { id: "untitled", mode: "new" as const, fromAccountId: "work", fromAddress: "sender@example.test", to: [], cc: [], bcc: [], subject: "", body: "Kept", bodyHtml: "", remindIfNoReply: false, updatedAt: 1, signatureId: null, fieldsOpen: true, showCc: false, showBcc: false };
  fakeMail({ deliveryAttention: vi.fn(async () => ({ entries: [accepted], next: null })), deliveryAttentionDraft: vi.fn(async () => snapshot), resolveDelivery: resolve });
  const user = userEvent.setup(); const onRecoveryChanged = vi.fn(); render(<DeliveryAttention onRecoveryChanged={onRecoveryChanged} />);
  await user.click(await screen.findByRole("button", { name: /^Review delivery for / }));
  expect(await screen.findByRole("heading", { name: "No subject" })).toBeVisible();
  expect(screen.getByText("sender@example.test")).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Discard recovery files" }));
  await user.click(screen.getByRole("checkbox", { name: "I understand I am discarding recovery files without creating a copy" }));
  const confirm = screen.getByRole("button", { name: "Confirm review" });
  act(() => { confirm.click(); confirm.click(); });
  expect(resolve).toHaveBeenCalledOnce();
  expect(confirm).toBeDisabled();
  await act(async () => finish({ status: "resolved", recoveryId: null }));
  expect(onRecoveryChanged).not.toHaveBeenCalled();
});

it("ignores an obsolete preview failure after closing its review", async () => {
  let fail!: (error: Error) => void;
  fakeMail({ deliveryAttention: vi.fn(async () => ({ entries: [accepted], next: null })), deliveryAttentionDraft: vi.fn(() => new Promise<undefined>((_, reject) => { fail = reject; })) });
  const user = userEvent.setup(); render(<DeliveryAttention onRecoveryChanged={vi.fn()} />);
  await user.click(await screen.findByRole("button", { name: /^Review delivery for / }));
  await user.click(screen.getByRole("button", { name: "Close delivery review" }));
  await act(async () => fail(new Error("obsolete")));
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("identifies deliveries across accounts and explains accepted repair is still retrying", async () => {
  fakeMail({ deliveryAttention: vi.fn(async () => ({ entries: [accepted, { ...accepted, id: "second", accountId: "other", createdAt: 1000 }], next: null })) });
  const user = userEvent.setup(); render(<DeliveryAttention onRecoveryChanged={vi.fn()} />);
  const buttons = await screen.findAllByRole("button", { name: /^Review delivery for / });
  expect(buttons).toHaveLength(2);
  expect(buttons[0]).not.toHaveAccessibleName(buttons[1].getAttribute("aria-label")!);
  await user.click(buttons[0]);
  expect(await screen.findByText(/Moss keeps retrying local repair/)).toBeVisible();
});

it("shows a copy refusal and refreshes independent recoveries even when marking review failed", async () => {
  const changed = vi.fn();
  fakeMail({ deliveryAttention: vi.fn(async () => ({ entries: [{ ...accepted, state: "uncertain" as const }], next: null })), deliveryAttentionDraft: vi.fn(async () => draft("copy")), resolveDelivery: vi.fn(async () => { throw new Error("Recovery storage is full"); }) });
  const user = userEvent.setup(); render(<DeliveryAttention onRecoveryChanged={changed} />);
  await user.click(await screen.findByRole("button", { name: /^Review delivery for / }));
  await user.click(screen.getByRole("button", { name: "Keep a recovery copy" }));
  await user.click(screen.getByRole("button", { name: "Confirm review" }));
  expect(await screen.findByText(/Recovery storage is full/)).toBeVisible();
  expect(changed).toHaveBeenCalledOnce();
});


it("requires a loss acknowledgement and offers a private archive before discarding unreadable files", async () => {
  fakeMail({ deliveryAttention: vi.fn(async () => ({ entries: [{ ...accepted, fileCount: 2, fileBytes: 2048 }], next: null })), deliveryAttentionDraft: vi.fn(async () => { throw new Error("damaged"); }) });
  const user = userEvent.setup(); render(<DeliveryAttention onRecoveryChanged={vi.fn()} />);
  await user.click(await screen.findByRole("button", { name: /^Review delivery for / }));
  await screen.findByText(/saved content cannot be previewed/);
  await user.click(screen.getByRole("button", { name: "Discard recovery files" }));
  expect(screen.getByText(/2 recovery files \(2 KB\)/)).toBeVisible();
  expect(screen.getByText(/cannot restore from an archive yet/)).toBeVisible();
  const confirm = screen.getByRole("button", { name: "Confirm review" });
  expect(confirm).toBeDisabled();
  await user.click(screen.getByRole("checkbox", { name: "I understand I am discarding recovery files without creating a copy" }));
  expect(confirm).toBeEnabled();
});


it("opens the archive settings from the one-file delivery scene", async () => {
  recoveryScenes["delivery-recovery"].apply();
  const user = userEvent.setup(); render(<DeliveryAttention onRecoveryChanged={vi.fn()} />);
  await user.click((await screen.findAllByRole("button", { name: /^Review delivery for / }))[0]);
  expect(await screen.findByText("Cabinet plans")).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Discard recovery files" }));
  expect(screen.getByText(/1 recovery file \(2 KB\)/)).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Open recovery archive settings" }));
  expect(useStore.getState().settingsSection).toBe("storage");
  expect(useStore.getState().view).toBe("settings");
});


it("never replaces the selected preview with an older delivery's late content", async () => {
  const snapshot = (id: string) => ({ id, mode: "new" as const, fromAccountId: "work", to: [], cc: [], bcc: [], subject: id, body: id, bodyHtml: "", remindIfNoReply: false, updatedAt: 1, signatureId: null, fieldsOpen: true, showCc: false, showBcc: false });
  let finish!: (draft: ReturnType<typeof snapshot>) => void;
  fakeMail({ deliveryAttention: vi.fn(async () => ({ entries: [accepted, { ...accepted, id: "second", createdAt: 2 }], next: null })), deliveryAttentionDraft: vi.fn((id) => id === "accepted" ? new Promise<ReturnType<typeof snapshot>>((resolve) => { finish = resolve; }) : Promise.resolve(snapshot("second"))) });
  const user = userEvent.setup(); render(<DeliveryAttention onRecoveryChanged={vi.fn()} />);
  const buttons = await screen.findAllByRole("button", { name: /^Review delivery for / });
  await user.click(buttons[0]); await user.click(buttons[1]);
  expect(await screen.findByRole("heading", { name: "second" })).toBeVisible();
  await act(async () => finish(snapshot("obsolete")));
  expect(screen.getByRole("heading", { name: "second" })).toBeVisible();
  expect(screen.queryByText("obsolete")).not.toBeInTheDocument();
});


it("does not reset an independent recovery review when no recovery copy changed", async () => {
  const changed=vi.fn();
  fakeMail({ deliveryAttention: vi.fn(async () => ({ entries: [accepted], next: null })), resolveDelivery: vi.fn(async () => ({ status: "resolved" as const, recoveryId: null })) });
  const user=userEvent.setup();render(<DeliveryAttention onRecoveryChanged={changed} />);
  await user.click(await screen.findByRole("button", {name:/^Review delivery for /}));
  await user.click(screen.getByRole("button", {name:"Discard recovery files"}));
  await user.click(screen.getByRole("checkbox", {name:"I understand I am discarding recovery files without creating a copy"}));
  await user.click(screen.getByRole("button", {name:"Confirm review"}));
  await waitFor(() => expect(screen.queryByRole("button", {name:"Confirm review"})).not.toBeInTheDocument());
  expect(changed).not.toHaveBeenCalled();
});
