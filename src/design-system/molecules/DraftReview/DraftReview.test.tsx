import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { person } from "../../../test/fixtures";
import { DraftReview } from "./DraftReview";

it("shows the exact sending address alongside full recipients and files", () => {
  render(<DraftReview from="maya@example.test" to={[person("Dana", "dana@example.test")]} cc={[]} bcc={[]} />);
  expect(screen.getByText("From")).toBeInTheDocument();
  expect(screen.getByText("maya@example.test")).toBeInTheDocument();
  expect(screen.getByText("Dana <dana@example.test>")).toBeInTheDocument();
});

it.each(["\u061c", "\u200e", "\u200f", "\u202a", "\u202b", "\u202c", "\u202d", "\u202e", "\u2066", "\u2067", "\u2068", "\u2069"])("removes direction-changing controls from names without hiding the real address or extension: %s", (control) => {
  render(<DraftReview from="maya@example.test" to={[person(`Dana${control}`, "real@example.test")]} cc={[]} bcc={[]}
    attachments={[{ id: "f1", name: `invoice${control}fdp.exe`, contentType: "application/octet-stream", bytes: 12 }]} />);
  expect(screen.getByText("Dana <real@example.test>")).toBeInTheDocument();
  expect(screen.getByText("invoicefdp.exe")).toBeInTheDocument();
});

it("isolates every recipient so a right-to-left name cannot reorder a neighboring address", () => {
  render(<DraftReview from="maya@example.test" to={[person("שלום", "first@example.test"), person("Dana", "second@example.test")]} cc={[]} bcc={[]} />);
  const first = screen.getByText("שלום <first@example.test>");
  const second = screen.getByText("Dana <second@example.test>");
  expect(first.closest("bdi")).toHaveAttribute("dir", "ltr");
  expect(second.closest("bdi")).toHaveAttribute("dir", "ltr");
  expect(first.closest("bdi")).not.toBe(second.closest("bdi"));
});

it("keeps a right-to-left filename and its extension in a left-to-right review field", () => {
  render(<DraftReview from="maya@example.test" to={[]} cc={[]} bcc={[]}
    attachments={[{ id: "rtl-file", name: "שלום.exe", contentType: "application/octet-stream", bytes: 12 }]} />);
  expect(screen.getByText("שלום.exe").closest("bdi")).toHaveAttribute("dir", "ltr");
});
