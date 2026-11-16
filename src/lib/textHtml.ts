/** Text made safe to put in HTML. */
export function escapeHtml(s: string) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
}

/** Plain text as email HTML: blank lines split paragraphs, single line breaks stay inside one. */
export function textToHtml(text: string) {
  if (!text) return "";
  return text
    .split(/\n{2,}/)
    .map((p) => `<p>${escapeHtml(p).replace(/\n/g, "<br>")}</p>`)
    .join("");
}
