import type { MouseEvent } from "react";
import { cx } from "../../../lib/cx";
import { openLink } from "../../../lib/links";
import { markdownHtml } from "../../../lib/markdown";
import styles from "./Markdown.module.css";

export interface MarkdownProps {
  /** What the model wrote. */
  text: string;
  /** In a line of its own (a to-do, a heading): emphasis and code only, no paragraphs or lists. */
  inline?: boolean;
  className?: string;
}

/** Text a model wrote, with its Markdown shown as it means: on the neutral surface in normal ink, like the rest of Moss's AI text. */
export function Markdown({ text, inline, className }: MarkdownProps) {
  // Links leave Moss through the opener, never by moving the window off the app.
  const click = (e: MouseEvent<HTMLElement>) => {
    const link = (e.target as HTMLElement).closest("a");
    if (!link) return;
    e.preventDefault();
    openLink(link.getAttribute("href") ?? "");
  };
  const Tag = inline ? "span" : "div";
  return <Tag className={cx(inline ? styles.inline : styles.markdown, className)} onClick={click} dangerouslySetInnerHTML={{ __html: markdownHtml(text, inline ?? false) }} />;
}
