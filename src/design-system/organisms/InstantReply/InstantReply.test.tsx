import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ReplyOption } from "../../../data/types";
import { InstantReply, type InstantReplyProps } from "./InstantReply";

const options: ReplyOption[] = [
  { label: "Yes, works", draft: "Yes, Thursday works for me." },
  { label: "Not this week", draft: "Can we move it to next week?" },
];
const from = { letter: "W", color: "work" as const, address: "maya@northwind.co" };

function setup(props: Partial<InstantReplyProps> = {}) {
  const handlers = {
    onChoose: vi.fn(),
    onDraftChange: vi.fn(),
    onChangeIt: vi.fn(),
    onEdit: vi.fn(),
    onSend: vi.fn(),
    onStop: vi.fn(),
    onDownload: vi.fn(),
    onSettings: vi.fn(),
    onRetry: vi.fn(),
    onDiscard: vi.fn(),
  };
  const choice = props.choice ?? 0;
  render(<InstantReply options={options} choice={choice} draft={options[choice]?.draft ?? ""} from={from} {...handlers} {...props} />);
  return { ...handlers, user: userEvent.setup() };
}

describe("InstantReply", () => {
  it("is the Quick reply card, which says Moss wrote it and nothing sends yet", () => {
    setup();
    expect(screen.getByRole("region", { name: "Quick reply" })).toHaveTextContent("Moss wrote this. Nothing sends until you press Send.");
  });

  it("offers the options as radios with the chosen one checked", () => {
    setup({ choice: 1 });
    const radios = screen.getAllByRole("radio");
    expect(screen.getByRole("radiogroup", { name: "Reply options" })).toBeInTheDocument();
    expect(radios.map((r) => r.textContent)).toEqual(["Yes, works", "Not this week"]);
    expect(radios.map((r) => r.getAttribute("aria-checked"))).toEqual(["false", "true"]);
  });

  it("the draft is text you can change in place, ready to type at the end", async () => {
    const { onDraftChange, user } = setup({ choice: 1, focus: true });
    const draft = screen.getByRole("textbox", { name: "Reply" });
    expect(draft).toHaveValue("Can we move it to next week?");
    expect(draft).toHaveFocus();
    expect((draft as HTMLTextAreaElement).selectionStart).toBe(28);
    await user.type(draft, "!");
    expect(onDraftChange).toHaveBeenLastCalledWith("Can we move it to next week?!");
  });

  it("shown on its own, it leaves the keys to the reading view", () => {
    setup();
    expect(screen.getByRole("textbox", { name: "Reply" })).not.toHaveFocus();
  });

  it("Tab in the draft moves to the next option, ⇧Tab to the one before", async () => {
    const { onChoose, user } = setup({ choice: 1, focus: true });
    await user.keyboard("{Tab}");
    expect(onChoose).toHaveBeenLastCalledWith(0);
    await user.keyboard("{Shift>}{Tab}{/Shift}");
    expect(onChoose).toHaveBeenLastCalledWith(0);
    await user.keyboard("a");
    expect(onChoose).toHaveBeenCalledTimes(2);
  });

  it("choosing an option reports its index", async () => {
    const { onChoose, user } = setup();
    await user.click(screen.getByRole("radio", { name: "Not this week" }));
    expect(onChoose).toHaveBeenCalledWith(1);
  });

  it("shows the signature it goes with under the reply", () => {
    setup({ signature: <p>Best, Maya</p> });
    const reply = screen.getByRole("textbox", { name: "Reply" });
    const signature = screen.getByText("Best, Maya");
    expect(reply.compareDocumentPosition(signature) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("Discard takes it away; Esc does too once you opened it", async () => {
    const { onDiscard, user } = setup();
    await user.click(screen.getByRole("button", { name: "Discard" }));
    expect(onDiscard).toHaveBeenCalledOnce();
  });

  it("names Esc on Discard when you opened it, and while Moss writes too", () => {
    setup({ focus: true });
    expect(screen.getByRole("button", { name: "DiscardEsc" })).toBeInTheDocument();
  });

  it("shows the address it sends from", () => {
    setup();
    expect(screen.getByRole("button", { name: /From.*maya@northwind\.co/ })).toBeInTheDocument();
  });

  it("Change it, Full editor and Send call their handlers", async () => {
    const { onChangeIt, onEdit, onSend, user } = setup();
    await user.click(screen.getByRole("button", { name: "Change itCtrl+J" }));
    await user.click(screen.getByRole("button", { name: "Full editorCtrl+E" }));
    expect(onChangeIt).toHaveBeenCalledOnce();
    expect(onEdit).toHaveBeenCalledOnce();
    expect(onSend).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: /^Send/ }));
    expect(onSend).toHaveBeenCalledOnce();
  });

  it("while Moss writes, shows the writing strip with Stop and nothing to send yet", async () => {
    const { onStop, user } = setup({ options: [], draft: "", state: "writing" });
    expect(screen.getByRole("status")).toHaveTextContent("Moss is writing…");
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Send/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "StopEsc" }));
    expect(onStop).toHaveBeenCalledOnce();
  });

  it("while Moss writes, the replies so far show as they grow, with Stop and nothing to send yet", async () => {
    const { onStop, user } = setup({ options: [options[0]], state: "writing" });
    expect(screen.getByRole("radio", { name: "Yes, works" })).toHaveAttribute("aria-checked", "true");
    const reply = screen.getByRole("textbox", { name: "Reply" });
    expect(reply).toHaveValue("Yes, Thursday works for me.");
    expect(reply).toHaveAttribute("readonly");
    expect(screen.getByRole("status")).toHaveTextContent("Moss is writing…");
    expect(screen.queryByRole("button", { name: /^Send/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /^Discard/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "StopEsc" }));
    expect(onStop).toHaveBeenCalledOnce();
  });

  it("without the model or after a failure, offers the way on", async () => {
    const { onDownload, onSettings, user } = setup({ options: [], draft: "", state: "missing" });
    await user.click(screen.getByRole("button", { name: "Download↵" }));
    await user.click(screen.getByRole("button", { name: "AI settingsCtrl+," }));
    expect(onDownload).toHaveBeenCalledOnce();
    expect(onSettings).toHaveBeenCalledOnce();
  });

  it("a failure offers Try again, and no Undo: nothing was written", async () => {
    const { onRetry, user } = setup({ options: [], draft: "", state: "failed" });
    expect(screen.queryByRole("button", { name: /Undo/ })).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Try again↵" }));
    expect(onRetry).toHaveBeenCalledOnce();
  });
});
