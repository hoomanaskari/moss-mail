import { useCallback, useEffect, useRef, useState } from "react";
import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import { Placeholder } from "@tiptap/extensions";
import { textToHtml } from "../../../lib/textHtml";
import styles from "./DraftEditor.module.css";

export interface DraftEditorProps {
  /** Plain text. Blank lines separate paragraphs; single line breaks stay inside a paragraph. */
  initial: string;
  placeholder?: string;
  autoFocus?: boolean;
  /** Called with plain text (paragraphs separated by blank lines) and the email-ready HTML. */
  onChange: (text: string, html: string) => void;
}

/** Body text is 18px; like every caret in the design, ours is 1.2 × the font size. */
const CARET_HEIGHT = 22;

interface CaretBox {
  x: number;
  y: number;
}

/**
 * Where to draw the caret, relative to `frame`. The browser's own caret changes height
 * with the line box (17px to 31px), so we draw a fixed-height one centred on the line.
 */
function caretBox(editor: Editor, frame: HTMLElement): CaretBox | null {
  // The view only exists while the editor is mounted; TipTap tears it down and rebuilds it
  // on remounts, and reading it in between throws. No caret is the right answer then.
  if (editor.isDestroyed || !editor.isInitialized) return null;
  try {
    const { selection } = editor.state;
    if (!editor.view.hasFocus() || !selection.empty) return null;
    const at = editor.view.coordsAtPos(selection.head);
    const origin = frame.getBoundingClientRect();
    return { x: at.left - origin.left, y: (at.top + at.bottom) / 2 - origin.top };
  } catch {
    return null;
  }
}

/**
 * The email body, built on TipTap so typing, paste and undo behave the same on every platform.
 * Paragraph spacing matches the reading view so drafts look like the sent email.
 */
export function DraftEditor({ initial, placeholder = "Write your email", autoFocus, onChange }: DraftEditorProps) {
  const frameRef = useRef<HTMLDivElement>(null);
  const [caret, setCaret] = useState<CaretBox | null>(null);
  const [moving, setMoving] = useState(false);
  const settle = useRef<number | undefined>(undefined);

  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        // Email bodies don't need these, and blockquote's ⌘⇧B is "add BCC" in Moss.
        heading: false,
        blockquote: false,
        code: false,
        codeBlock: false,
        horizontalRule: false,
        link: { openOnClick: false, autolink: true, defaultProtocol: "https" },
      }),
      Placeholder.configure({ placeholder }),
    ],
    content: textToHtml(initial),
    autofocus: autoFocus ? "end" : false,
    editorProps: {
      attributes: { class: styles.editor, role: "textbox", "aria-multiline": "true", "aria-label": "Email body" },
      // ⌘↵ sends (handled app-wide); don't also insert a line break. ⇧↵ still breaks a line.
      handleKeyDown: (_view, event) => event.key === "Enter" && (event.metaKey || event.ctrlKey),
    },
    onUpdate: ({ editor }) => onChange(editor.getText({ blockSeparator: "\n\n" }), editor.getHTML()),
  });

  const placeCaret = useCallback(() => {
    const frame = frameRef.current;
    if (!editor || !frame) return;
    setCaret(caretBox(editor, frame));
    // Hold the caret solid while typing or moving; blink once it rests.
    setMoving(true);
    window.clearTimeout(settle.current);
    settle.current = window.setTimeout(() => setMoving(false), 500);
  }, [editor]);

  useEffect(() => {
    if (!editor) return;
    const hide = () => setCaret(null);
    editor.on("selectionUpdate", placeCaret);
    editor.on("update", placeCaret);
    editor.on("focus", placeCaret);
    editor.on("blur", hide);
    const observer = new ResizeObserver(placeCaret);
    if (frameRef.current) observer.observe(frameRef.current);
    placeCaret();
    return () => {
      editor.off("selectionUpdate", placeCaret);
      editor.off("update", placeCaret);
      editor.off("focus", placeCaret);
      editor.off("blur", hide);
      observer.disconnect();
      window.clearTimeout(settle.current);
    };
  }, [editor, placeCaret]);

  return (
    <div ref={frameRef} className={styles.frame}>
      <EditorContent editor={editor} />
      {caret && (
        <span
          className={moving ? styles.caret : `${styles.caret} ${styles.blink}`}
          style={{ left: caret.x, top: caret.y - CARET_HEIGHT / 2, height: CARET_HEIGHT }}
          aria-hidden
        />
      )}
    </div>
  );
}
