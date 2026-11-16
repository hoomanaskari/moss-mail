import {
  CalendarDays,
  File,
  FileArchive,
  FileCode,
  FileImage,
  FileMusic,
  FilePenLine,
  FilePlay,
  FileSpreadsheet,
  FileText,
  NotepadText,
  Presentation,
  X,
  type LucideIcon,
} from "lucide-react";
import { useState } from "react";
import { fileKind, type FileKindName } from "../../../lib/fileKind";
import { Icon } from "../../atoms";
import styles from "./AttachmentChip.module.css";

export interface AttachmentChipProps {
  name: string;
  size: string;
  /** The MIME type, used when the name has no extension. */
  contentType?: string;
  /** A thumbnail of what's in the file, as an image URL. */
  preview?: string;
  onOpen?: () => void;
  /** For a file going with an email being written: a remove button, and ⌫ on the card. */
  onRemove?: () => void;
}

const glyphs: Record<FileKindName, LucideIcon> = {
  pdf: FileText,
  document: FilePenLine,
  spreadsheet: FileSpreadsheet,
  presentation: Presentation,
  archive: FileArchive,
  image: FileImage,
  audio: FileMusic,
  video: FilePlay,
  code: FileCode,
  calendar: CalendarDays,
  text: NotepadText,
  file: File,
};

/** A file attached to an email: a thumbnail of it, or its kind's icon, with a badge for its format. */
export function AttachmentChip({ name, size, contentType, preview, onOpen, onRemove }: AttachmentChipProps) {
  const { kind, format, label } = fileKind(name, contentType);
  const [failed, setFailed] = useState<string>();
  const thumbnail = preview && preview !== failed ? preview : undefined;
  const card = (
    <button
      type="button"
      className={styles.card}
      data-kind={kind}
      onClick={onOpen}
      onKeyDown={
        onRemove &&
        ((e) => {
          if (e.key !== "Backspace" && e.key !== "Delete") return;
          e.preventDefault();
          onRemove();
        })
      }
      title={name}
      aria-label={`${name}, ${label}, ${size}`}
    >
      <span className={styles.preview}>
        {thumbnail ? (
          <img className={styles.thumbnail} src={thumbnail} alt="" onError={() => setFailed(thumbnail)} />
        ) : (
          <Icon icon={glyphs[kind]} size={32} tone="inherit" />
        )}
        <span className={styles.badge}>{format}</span>
      </span>
      <span className={styles.footer}>
        <span className={styles.name}>{name}</span>
        <span className={styles.size}>{size}</span>
      </span>
    </button>
  );
  if (!onRemove) return card;
  return (
    <span className={styles.removable}>
      {card}
      <button type="button" className={styles.remove} onClick={onRemove} aria-label={`Remove ${name}`} title={`Remove ${name} (⌫)`}>
        <Icon icon={X} size={14} tone="inherit" />
      </button>
    </span>
  );
}
