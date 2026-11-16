/** What sort of file an attachment is, which picks its icon and colour. */
export type FileKindName =
  | "pdf"
  | "document"
  | "spreadsheet"
  | "presentation"
  | "archive"
  | "image"
  | "audio"
  | "video"
  | "code"
  | "calendar"
  | "text"
  | "file";

export interface FileKind {
  kind: FileKindName;
  /** The badge: the file's format, e.g. "PDF" or "DOCX". */
  format: string;
  /** Read out after the name, e.g. "Word document". */
  label: string;
}

const byExtension: Record<string, [FileKindName, string]> = {
  pdf: ["pdf", "PDF document"],
  doc: ["document", "Word document"],
  docx: ["document", "Word document"],
  pages: ["document", "Pages document"],
  odt: ["document", "document"],
  rtf: ["document", "document"],
  xls: ["spreadsheet", "Excel spreadsheet"],
  xlsx: ["spreadsheet", "Excel spreadsheet"],
  numbers: ["spreadsheet", "Numbers spreadsheet"],
  ods: ["spreadsheet", "spreadsheet"],
  csv: ["spreadsheet", "spreadsheet"],
  tsv: ["spreadsheet", "spreadsheet"],
  ppt: ["presentation", "PowerPoint presentation"],
  pptx: ["presentation", "PowerPoint presentation"],
  key: ["presentation", "Keynote presentation"],
  odp: ["presentation", "presentation"],
  ics: ["calendar", "calendar invite"],
  vcs: ["calendar", "calendar invite"],
  txt: ["text", "text file"],
  md: ["text", "text file"],
  log: ["text", "text file"],
};

const groups: [FileKindName, string, string[]][] = [
  ["archive", "archive", ["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz"]],
  ["image", "image", ["jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "tif", "tiff", "bmp", "svg"]],
  ["audio", "audio", ["mp3", "m4a", "wav", "aac", "flac", "ogg", "aif", "aiff"]],
  ["video", "video", ["mp4", "mov", "m4v", "avi", "mkv", "webm"]],
  [
    "code",
    "code",
    ["js", "ts", "tsx", "jsx", "json", "html", "css", "py", "rb", "rs", "go", "java", "c", "cpp", "h", "sh", "xml", "yml", "yaml", "swift", "kt", "php", "sql"],
  ],
];
for (const [kind, label, extensions] of groups) for (const e of extensions) byExtension[e] = [kind, label];

/** The kind a content type names when the file name has no extension. */
function kindOfType(contentType: string): [FileKindName, string, string] | undefined {
  const [top, sub = ""] = contentType.toLowerCase().split(";")[0].trim().split("/");
  if (sub === "pdf") return ["pdf", "PDF document", "PDF"];
  if (sub === "calendar") return ["calendar", "calendar invite", "ICS"];
  if (top === "image" || top === "audio" || top === "video") return [top, top, sub.replace(/^x-/, "").toUpperCase()];
  return undefined;
}

/** The file's last extension, e.g. "gz" for "backup.tar.gz"; none for ".hidden" or "odd.". */
function extension(name: string): string | undefined {
  const dot = name.lastIndexOf(".");
  return dot > 0 && dot < name.length - 1 ? name.slice(dot + 1).toLowerCase() : undefined;
}

/** What sort of file `name` is: from its extension, or else its content type. */
export function fileKind(name: string, contentType = ""): FileKind {
  const ext = extension(name);
  const known = ext && byExtension[ext];
  if (known) return { kind: known[0], format: ext.toUpperCase(), label: known[1] };
  const typed = !ext && kindOfType(contentType);
  if (typed) return { kind: typed[0], format: typed[2], label: typed[1] };
  return { kind: "file", format: ext ? ext.slice(0, 6).toUpperCase() : "FILE", label: "file" };
}

/** Whether Moss draws a thumbnail of what's in the file: pictures, PDFs, videos and documents. */
export function canPreview(name: string, contentType = ""): boolean {
  const kinds: FileKindName[] = ["image", "pdf", "video", "document", "spreadsheet", "presentation"];
  return kinds.includes(fileKind(name, contentType).kind);
}
