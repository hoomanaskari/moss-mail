/**
 * Markdown as AI writes it, made safe to show: HTML for the Markdown atom, or one line of plain
 * text for places with room for no more.
 */
import DOMPurify from "dompurify";
import { Marked } from "marked";

/** GitHub-flavoured, as models write it: tables, struck text, and a line break where they put one. */
const marked = new Marked({ gfm: true, breaks: true, async: false });

/** What a model's Markdown may become: text and its structure, links that open outside Moss. Never scripts, frames or images. */
const ALLOWED = {
  ALLOWED_TAGS: ["p", "br", "strong", "b", "em", "i", "del", "s", "code", "pre", "blockquote", "ul", "ol", "li", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "a", "table", "thead", "tbody", "tr", "th", "td", "span"],
  ALLOWED_ATTR: ["href", "start", "align"],
  ALLOWED_URI_REGEXP: /^(https?:|mailto:)/i,
};

/** The HTML a model's Markdown means, with only text, its structure and web or mail links left. */
export function markdownHtml(text: string, inline = false): string {
  const made = inline ? marked.parseInline(text) : marked.parse(text);
  return DOMPurify.sanitize(made as string, ALLOWED);
}

/** A model's Markdown as one line of plain text, for places with room for no more: a mail row, a header line. */
export function plainText(text: string): string {
  const spaced = markdownHtml(text).replace(/<br\s*\/?>|<\/(p|h[1-6]|li|blockquote|pre|tr|th|td)>/gi, " ");
  const doc = new DOMParser().parseFromString(spaced, "text/html");
  return String(doc.body.textContent).replace(/\s+/g, " ").trim();
}
