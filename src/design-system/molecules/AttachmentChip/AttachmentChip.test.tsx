import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { AttachmentChip } from "./AttachmentChip";

describe("AttachmentChip", () => {
  it("reads out the file's name, kind and size and opens it on click", async () => {
    const onOpen = vi.fn();
    render(<AttachmentChip name="Budget.pdf" size="240 KB" onOpen={onOpen} />);
    const chip = screen.getByRole("button", { name: "Budget.pdf, PDF document, 240 KB" });
    expect(chip).toHaveTextContent("PDF");
    expect(chip).toHaveTextContent("240 KB");
    expect(chip).toHaveAttribute("title", "Budget.pdf");
    await userEvent.click(chip);
    expect(onOpen).toHaveBeenCalledOnce();
  });

  it("opens from the keyboard", async () => {
    const onOpen = vi.fn();
    render(<AttachmentChip name="Plan.docx" size="84 KB" onOpen={onOpen} />);
    await userEvent.tab();
    expect(screen.getByRole("button", { name: /Plan\.docx, Word document/ })).toHaveFocus();
    await userEvent.keyboard("{Enter}");
    expect(onOpen).toHaveBeenCalledOnce();
  });

  it("is still clickable without onOpen", async () => {
    render(<AttachmentChip name="Notes.txt" size="2 KB" />);
    await userEvent.click(screen.getByRole("button", { name: /Notes\.txt, text file/ }));
  });

  it("a file going with an email can be taken off by mouse", async () => {
    const onRemove = vi.fn();
    render(<AttachmentChip name="Plan.pdf" size="2 KB" onRemove={onRemove} />);
    await userEvent.click(screen.getByRole("button", { name: "Remove Plan.pdf" }));
    expect(onRemove).toHaveBeenCalledOnce();
  });

  it("and by keyboard: ⌫ or Delete on the file, or Tab to its remove button", async () => {
    const onRemove = vi.fn();
    render(<AttachmentChip name="Plan.pdf" size="2 KB" onRemove={onRemove} />);
    await userEvent.tab();
    expect(screen.getByRole("button", { name: /Plan\.pdf, PDF document/ })).toHaveFocus();
    await userEvent.keyboard("{Backspace}");
    await userEvent.keyboard("{Delete}");
    await userEvent.keyboard("a");
    expect(onRemove).toHaveBeenCalledTimes(2);
    await userEvent.tab();
    expect(screen.getByRole("button", { name: "Remove Plan.pdf" })).toHaveFocus();
    await userEvent.keyboard("{Enter}");
    expect(onRemove).toHaveBeenCalledTimes(3);
  });

  it("goes by the content type when the name has no extension", () => {
    render(<AttachmentChip name="scan" size="1 MB" contentType="application/pdf" />);
    expect(screen.getByRole("button", { name: "scan, PDF document, 1 MB" })).toHaveTextContent("PDF");
  });

  it("shows a thumbnail when there is one, and its icon if the picture can't load", () => {
    const { container } = render(<AttachmentChip name="Roof.jpg" size="1.9 MB" preview="data:image/jpeg;base64,AAAA" />);
    const thumbnail = container.querySelector("img");
    expect(thumbnail).toHaveAttribute("src", "data:image/jpeg;base64,AAAA");
    expect(thumbnail).toHaveAttribute("alt", "");
    expect(container.querySelector("svg")).toBeNull();
    fireEvent.error(thumbnail!);
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("svg")).not.toBeNull();
    expect(screen.getByRole("button", { name: "Roof.jpg, image, 1.9 MB" })).toHaveTextContent("JPG");
  });

  it("tries a new thumbnail after one failed", () => {
    const { container, rerender } = render(<AttachmentChip name="Roof.jpg" size="1.9 MB" preview="data:,broken" />);
    fireEvent.error(container.querySelector("img")!);
    rerender(<AttachmentChip name="Roof.jpg" size="1.9 MB" preview="data:,fixed" />);
    expect(container.querySelector("img")).toHaveAttribute("src", "data:,fixed");
  });
});
