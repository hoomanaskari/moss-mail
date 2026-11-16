import { describe, expect, it } from "vitest";
import { canPreview, fileKind } from "./fileKind";

describe("fileKind", () => {
  it.each([
    ["Budget.pdf", "pdf", "PDF", "PDF document"],
    ["Plan.docx", "document", "DOCX", "Word document"],
    ["Old plan.DOC", "document", "DOC", "Word document"],
    ["Letter.pages", "document", "PAGES", "Pages document"],
    ["Notes.rtf", "document", "RTF", "document"],
    ["Costs.xlsx", "spreadsheet", "XLSX", "Excel spreadsheet"],
    ["Costs.numbers", "spreadsheet", "NUMBERS", "Numbers spreadsheet"],
    ["export.csv", "spreadsheet", "CSV", "spreadsheet"],
    ["Pitch.pptx", "presentation", "PPTX", "PowerPoint presentation"],
    ["Pitch.key", "presentation", "KEY", "Keynote presentation"],
    ["Brand assets.zip", "archive", "ZIP", "archive"],
    ["backup.tar.gz", "archive", "GZ", "archive"],
    ["Roof.jpg", "image", "JPG", "image"],
    ["IMG_0042.HEIC", "image", "HEIC", "image"],
    ["Voice memo.m4a", "audio", "M4A", "audio"],
    ["Walkthrough.mov", "video", "MOV", "video"],
    ["config.json", "code", "JSON", "code"],
    ["invite.ics", "calendar", "ICS", "calendar invite"],
    ["README.txt", "text", "TXT", "text file"],
    ["notes.md", "text", "MD", "text file"],
    ["export.dat", "file", "DAT", "file"],
  ])("%s is a %s file shown as %s and read out as %s", (name, kind, format, label) => {
    expect(fileKind(name)).toEqual({ kind, format, label });
  });

  it("goes by the content type when the name has no extension", () => {
    expect(fileKind("scan", "application/pdf")).toEqual({ kind: "pdf", format: "PDF", label: "PDF document" });
    expect(fileKind("photo", "image/png")).toEqual({ kind: "image", format: "PNG", label: "image" });
    expect(fileKind("clip", "video/quicktime")).toEqual({ kind: "video", format: "QUICKTIME", label: "video" });
    expect(fileKind("invite", "text/calendar")).toEqual({ kind: "calendar", format: "ICS", label: "calendar invite" });
  });

  it("calls a file with neither just a file", () => {
    expect(fileKind("noname")).toEqual({ kind: "file", format: "FILE", label: "file" });
    expect(fileKind("blob", "application/octet-stream")).toEqual({ kind: "file", format: "FILE", label: "file" });
    expect(fileKind(".hidden")).toEqual({ kind: "file", format: "FILE", label: "file" });
  });

  it("keeps a long or odd extension to a short badge", () => {
    expect(fileKind("data.verylongext").format).toBe("VERYLO");
    expect(fileKind("odd.name.").format).toBe("FILE");
  });
});

// The same cases as `previewable` in crates/moss-engine/src/preview.rs.
describe("canPreview", () => {
  it.each([
    ["Roof.jpg", "image/jpeg", true],
    ["Budget.pdf", "application/pdf", true],
    ["Walkthrough.mov", "video/quicktime", true],
    ["Plan.docx", "application/octet-stream", true],
    ["Pitch.key", "application/octet-stream", true],
    ["Costs.xlsx", "application/octet-stream", true],
    ["scan", "application/pdf", true],
    ["Brand assets.zip", "application/zip", false],
    ["Voice memo.m4a", "audio/mp4", false],
    ["invite.ics", "text/calendar", false],
    ["export.dat", "application/octet-stream", false],
  ])("%s (%s): %s", (name, type, expected) => {
    expect(canPreview(name, type)).toBe(expected);
  });
});
