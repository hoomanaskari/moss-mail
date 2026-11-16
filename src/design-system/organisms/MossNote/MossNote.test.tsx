import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { MossNote, type MossNoteProps } from "./MossNote";

function setup(props: Partial<MossNoteProps> & Pick<MossNoteProps, "state">) {
  const handlers = {
    onChangeIt: vi.fn(),
    onUndo: vi.fn(),
    onStop: vi.fn(),
    onDownload: vi.fn(),
    onSettings: vi.fn(),
    onRetry: vi.fn(),
  };
  render(<MossNote {...handlers} {...props} />);
  return { ...handlers, user: userEvent.setup() };
}

describe("MossNote", () => {
  it("says what Moss wrote and from which note, with Change it and Undo", async () => {
    const { onChangeIt, onUndo, user } = setup({
      state: { kind: "wrote", lead: "Moss wrote this from your note", note: "yes to the 17th" },
    });
    const note = screen.getByRole("status");
    expect(note).toHaveTextContent("Moss wrote this from your note");
    expect(note).toHaveTextContent("“yes to the 17th”");
    await user.click(screen.getByRole("button", { name: "Change itCtrl+J" }));
    await user.click(screen.getByRole("button", { name: "UndoCtrl+Z" }));
    expect(onChangeIt).toHaveBeenCalledOnce();
    expect(onUndo).toHaveBeenCalledOnce();
  });

  it("a menu action has only its lead line", () => {
    setup({ state: { kind: "wrote", lead: "Moss made it shorter" } });
    expect(screen.getByRole("status").textContent).toMatch(/^Moss made it shorter/);
    expect(screen.queryByText(/“/)).not.toBeInTheDocument();
  });

  it("while writing, keeps your words and offers Stop", async () => {
    const { onStop, user } = setup({ state: { kind: "writing" } });
    expect(screen.getByRole("status")).toHaveTextContent("Moss is writing…Your words stay as they are until it’s done.");
    await user.click(screen.getByRole("button", { name: "StopEsc" }));
    expect(onStop).toHaveBeenCalledOnce();
  });

  it("without the writing model, offers the download (ready for ↵) and AI settings", async () => {
    const { onDownload, onSettings, user } = setup({ state: { kind: "missing" } });
    expect(screen.getByRole("status")).toHaveTextContent("Writing needs the writing model");
    const download = screen.getByRole("button", { name: "Download↵" });
    expect(download).toHaveFocus();
    await user.click(download);
    await user.click(screen.getByRole("button", { name: "AI settingsCtrl+," }));
    expect(onDownload).toHaveBeenCalledOnce();
    expect(onSettings).toHaveBeenCalledOnce();
  });

  it("when it fails, says the draft is unchanged and offers Try again (ready for ↵) and Undo", async () => {
    const { onRetry, onUndo, user } = setup({ state: { kind: "failed" } });
    expect(screen.getByRole("status")).toHaveTextContent("Couldn’t write this oneThe writing model stopped. Your draft is unchanged.");
    expect(screen.getByRole("button", { name: "Try again↵" })).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(onRetry).toHaveBeenCalledOnce();
    await user.click(screen.getByRole("button", { name: "UndoCtrl+Z" }));
    expect(onUndo).toHaveBeenCalledOnce();
  });

  it("leaves out actions it has no handler for", () => {
    render(<MossNote state={{ kind: "failed" }} onRetry={() => {}} />);
    expect(screen.getAllByRole("button").map((b) => b.textContent)).toEqual(["Try again↵"]);
  });
});
