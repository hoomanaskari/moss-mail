/**
 * Turns an email's HTML into a self-contained document that is safe to show in a sandboxed frame:
 * sanitised, with remote content held back for privacy and quoted history collapsible.
 */
import DOMPurify from "dompurify";
import figtreeLatinExt from "@fontsource-variable/figtree/files/figtree-latin-ext-wght-normal.woff2?url";
import figtreeLatin from "@fontsource-variable/figtree/files/figtree-latin-wght-normal.woff2?url";
import tokens from "../../tokens/tokens.css?raw";
import type { ReadingSize, Theme } from "../../../state/store";

export interface PrepareOptions {
  /** Whether policy permits remote images for this message. */
  allowImages: boolean;
  /** Keep quoted history in the body instead of collapsing it. */
  showQuoted: boolean;
  /**
   * Nothing earlier in the thread holds what this email quotes (it is the thread's first email, often a forward):
   * its outermost quote stays in the body, and only the history inside that quote collapses.
   */
  quoteIsNew?: boolean;
  /** Your address: an image whose address carries it is reporting that you opened the email. */
  recipient?: string;
}

export interface PreparedHtml {
  /** Sanitised HTML for inside <body>. */
  body: string;
  /** The email's own CSS, cleaned. */
  styles: string;
  /** The email asks for remote images or backgrounds (held back unless allowed). */
  remoteContent: boolean;
  /** The email has quoted history that can be collapsed. */
  hasQuoted: boolean;
  /** Laid out like a newsletter, so it keeps its own look instead of Moss's reading typography. */
  designed: boolean;
  /** The email brings its own dark-mode styles (`@media (prefers-color-scheme: dark)`), so it can follow a dark theme. */
  darkVersion: boolean;
  /** Images matching possible-tracker heuristics, whether or not images are shown. */
  trackers: number;
}

const forbiddenTags = [
  "form", "input", "button", "select", "option", "textarea", "base", "meta", "link",
  "iframe", "frame", "frameset", "object", "embed", "applet", "audio", "video", "source", "track", "svg", "math",
  // An image map's <area> is a link the frame would follow itself, so maps go, links are only <a>.
  "map", "area",
];

/** Attributes that could send a click, or the frame itself, somewhere other than through Moss. */
const forbiddenAttributes = ["ping", "action", "formaction", "usemap", "xlink:href", "target"];

/** "On Sun, 1 Nov 2026 at 09:12, Lena Vogt <lena@…> wrote:" */
const attribution = /^\s*On\b[\s\S]{0,300}\bwrote:\s*$/i;
const forwarded = /forwarded message|begin forwarded/i;
const quoteMark = "data-moss-quote";
/** A transparent pixel: stands in for a blocked image so it keeps its size without a broken-image icon. */
const placeholder = "data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

type UrlKind = "remote" | "inline" | "other";

/** Remote URLs load from the internet; inline ones are inside the email; anything else (relative paths) would load from the app. */
function urlKind(url: string): UrlKind {
  const u = url.trim();
  if (/^(https?:)?\/\//i.test(u)) return "remote";
  if (/^(data:image\/|cid:)/i.test(u)) return "inline";
  return "other";
}

/**
 * A remote address asked for over https. The app only lets https images load (no plain http in its CSP),
 * and a scheme-less `//` address would resolve against the app's own scheme. Browsers upgrade the same way.
 */
const secure = (url: string) => url.trim().replace(/^(http:)?\/\//i, "https://");

/** Removes CSS that can run code or pull in more CSS, and remote url()s unless allowed. */
function cleanCss(css: string, allowImages: boolean): { css: string; remote: boolean } {
  let remote = false;
  const out = css
    .replace(/@import[^;]*;?/gi, "")
    .replace(/expression\s*\([^)]*\)/gi, "")
    .replace(/(-moz-binding|behavior)\s*:[^;}]*;?/gi, "")
    .replace(/url\(\s*(['"]?)([^'")]*)\1\s*\)/gi, (match, q: string, url: string) => {
      const kind = urlKind(url);
      if (kind === "remote") remote = true;
      if (kind === "remote" && allowImages) return `url(${q}${secure(url)}${q})`;
      return url.trim().startsWith("data:") ? match : "none";
    });
  return { css: out, remote };
}

const plainBackground = /^\s*(#f{3}|#f{6}|white|transparent|none|inherit|initial|rgba?\(\s*255\s*,\s*255\s*,\s*255\s*(,\s*1(\.0+)?\s*)?\))?\s*(!important)?\s*$/i;

/** Highlighting a few words (often in a signature) colours text, not the layout. */
const inlineTags = new Set(["A", "B", "BIG", "CODE", "EM", "FONT", "I", "MARK", "SMALL", "SPAN", "STRONG", "SUB", "SUP", "U"]);

/** Heuristic: newsletters use coloured backgrounds, nested layout tables or wide images. Replies and simple notes don't. */
function isDesigned(body: HTMLElement): boolean {
  if (body.querySelectorAll("table").length >= 3) return true;
  for (const el of body.querySelectorAll("[bgcolor]")) if (!plainBackground.test(el.getAttribute("bgcolor")!)) return true;
  for (const el of body.querySelectorAll("[style]")) {
    if (inlineTags.has(el.tagName)) continue;
    const bg = /background(?:-color|-image)?\s*:\s*([^;]+)/i.exec(el.getAttribute("style")!);
    if (bg && !plainBackground.test(bg[1])) return true;
  }
  for (const img of body.querySelectorAll("img")) if (Number(img.getAttribute("width")) >= 300) return true;
  return false;
}

/** 1×1 (or hidden) images only exist to report that you opened the email. */
function isPixel(img: Element): boolean {
  const tiny = (attr: string) => img.hasAttribute(attr) && Number(img.getAttribute(attr)) <= 2;
  return tiny("width") || tiny("height") || /(^|;)\s*(width|height)\s*:\s*[0-2](px)?\s*(;|$)/i.test(img.getAttribute("style") ?? "");
}

const hidden = /(^|;)\s*(display\s*:\s*none|visibility\s*:\s*hidden|opacity\s*:\s*0(\.0+)?\s*(;|$))/i;
/** Open-tracking addresses: /track/open.php, /wf/open, /pixel.gif, and Mailgun-style /o/<token>. */
const trackingPath = /\/(open|opened|track|tracking|trk|pixel|beacon|impression)(\.[a-z]+)?(\/|\?|$)|\/o\/[\w=-]{20,}/i;

/** Possible tracking images, using fallible size/style/URL heuristics. See docs/research/remote-images.md. */
function isTracker(img: Element, recipient?: string): boolean {
  if (isPixel(img) || hidden.test(img.getAttribute("style") ?? "")) return true;
  const src = img.getAttribute("src")!;
  if (trackingPath.test(src)) return true;
  if (!recipient) return false;
  let decoded = src;
  try {
    decoded = decodeURIComponent(src);
  } catch {
    // A malformed escape: look at the address as written.
  }
  return decoded.toLowerCase().includes(recipient.toLowerCase());
}

const isBlank = (n: Node) => (n.nodeType === Node.TEXT_NODE && !n.textContent!.trim()) || n.nodeName === "BR";

const isEmpty = (n: Node) =>
  isBlank(n) || (n.nodeType === Node.ELEMENT_NODE && n.nodeName !== "IMG" && !n.textContent!.trim() && !(n as Element).querySelector("img"));

/** The "On … wrote:" line before a quote, and the line breaks between them. */
function attributionBefore(node: Node): Node[] {
  const between: Node[] = [];
  let prev = node.previousSibling;
  while (prev && isBlank(prev)) {
    between.push(prev);
    prev = prev.previousSibling;
  }
  return prev && attribution.test(prev.textContent!) ? [prev, ...between] : [];
}

/**
 * The nodes that make up each piece of quoted history to collapse, outermost only.
 * A quote you can't read anywhere else stays: a forwarded message, or (`quoteIsNew`) the outermost quote of
 * a thread's first email. Only the older history inside such a quote collapses.
 */
function findQuotes(body: HTMLElement, quoteIsNew: boolean): Node[][] {
  const groups: Node[][] = [];
  const seen: Node[] = [];
  const kept: Node[] = [];
  for (const el of body.querySelectorAll(".gmail_quote, blockquote, #divRplyFwdMsg")) {
    if (seen.some((s) => s.contains(el))) continue;
    // Gmail puts the quoted text in a blockquote inside the quote's own block: one quote, not history within it.
    if (el.matches(".gmail_quote > blockquote.gmail_quote") && kept.includes(el.parentElement!)) continue;
    let group: Node[];
    if (el.id === "divRplyFwdMsg") {
      // Outlook: a rule, the From/Sent header, then the old message as following siblings.
      group = [el];
      const hr = el.previousElementSibling;
      if (hr?.tagName === "HR") group.unshift(hr);
      for (let next = el.nextSibling; next; next = next.nextSibling) group.push(next);
    } else {
      const before = attributionBefore(el);
      const cited = el.classList.contains("gmail_quote") || el.getAttribute("type") === "cite";
      if (!cited && !before.length) continue;
      group = [...before, el];
    }
    const outermost = !kept.some((k) => k.contains(el));
    if ((quoteIsNew && outermost) || forwarded.test(group.map((n) => n.textContent).join(" "))) {
      kept.push(...group);
      continue;
    }
    seen.push(...group);
    groups.push(group);
  }
  return groups;
}

/** Marks quoted history so it can be measured and removed; bare text nodes get a span to carry the mark. */
function markQuotes(body: HTMLElement, quoteIsNew: boolean): boolean {
  const groups = findQuotes(body, quoteIsNew);
  for (const node of groups.flat()) {
    if (node.nodeType === Node.ELEMENT_NODE) (node as Element).setAttribute(quoteMark, "");
    else {
      const span = body.ownerDocument.createElement("span");
      span.setAttribute(quoteMark, "");
      node.parentNode!.replaceChild(span, node);
      span.append(node);
    }
  }
  if (!groups.length) return false;
  const rest = body.cloneNode(true) as HTMLElement;
  rest.querySelectorAll(`[${quoteMark}]`).forEach((n) => n.remove());
  return !!rest.textContent!.trim() || !!rest.querySelector("img");
}

/** Keeps one empty line where several follow each other. Designed email keeps them: there they space the layout. */
function closeUpEmptyLines(root: HTMLElement) {
  for (const parent of [root, ...root.querySelectorAll("*")]) {
    let previousEmpty = false;
    for (const node of [...parent.childNodes]) {
      if (node.nodeType === Node.TEXT_NODE && !node.textContent!.trim()) continue;
      const empty = isEmpty(node);
      if (empty && previousEmpty) node.remove();
      previousEmpty = empty;
    }
  }
}

/** Wraps everything in `body` in a div that carries these attributes of `from`, if it has any. */
function keepOn(body: HTMLElement, from: Element, attributes: string[]) {
  const kept = attributes.filter((a) => from.hasAttribute(a));
  if (!kept.length) return;
  const wrapper = body.ownerDocument.createElement("div");
  for (const a of kept) wrapper.setAttribute(a, from.getAttribute(a)!);
  wrapper.append(...body.childNodes);
  body.append(wrapper);
}

/** Sanitises an email's HTML and applies the privacy and quote choices. */
export function prepareHtml(html: string, { allowImages, showQuoted, quoteIsNew = false, recipient }: PrepareOptions): PreparedHtml {
  const root = DOMPurify.sanitize(html, {
    WHOLE_DOCUMENT: true,
    RETURN_DOM: true,
    FORBID_TAGS: forbiddenTags,
    FORBID_ATTR: forbiddenAttributes,
  }) as HTMLElement;
  const body = root.querySelector("body")!;
  const page = body.parentElement!;
  // Looks inside <html>, so a colour on <body> itself counts.
  const designed = isDesigned(page);

  // The frame has its own <html> and <body>, so what the email put on its own goes on wrappers instead:
  // newsletters colour <body>, and designed email hangs its styles, its dark version too, from classes on both.
  const looks = ["bgcolor", "background", "style"];
  const names = ["class", "id", "dir"];
  keepOn(body, body, designed ? [...looks, ...names] : looks);
  if (designed) keepOn(body, page, [...names, "lang"]);
  let remoteContent = false;

  const styles: string[] = [];
  for (const style of root.querySelectorAll("style")) {
    const clean = cleanCss(style.textContent!, allowImages);
    remoteContent ||= clean.remote;
    styles.push(clean.css);
    style.remove();
  }
  for (const el of body.querySelectorAll("[style]")) {
    const clean = cleanCss(el.getAttribute("style")!, allowImages);
    remoteContent ||= clean.remote;
    el.setAttribute("style", clean.css);
  }

  let trackers = 0;
  for (const img of body.querySelectorAll("img")) {
    const kind = urlKind(img.getAttribute("src") ?? "");
    if (kind === "remote" && isTracker(img, recipient)) {
      img.remove();
      trackers++;
      continue;
    }
    if (kind === "remote") {
      remoteContent = true;
      if (!allowImages) {
        img.setAttribute("src", placeholder);
        img.setAttribute("data-moss-blocked", "");
      } else img.setAttribute("src", secure(img.getAttribute("src")!));
    } else if (kind === "other") img.removeAttribute("src");
    const srcset = img.getAttribute("srcset");
    if (srcset !== null) {
      const parts = srcset.split(",").map((part) => part.trim().split(/\s+/));
      const kinds = parts.map(([url]) => urlKind(url));
      remoteContent ||= kinds.includes("remote");
      if (!allowImages || kinds.some((k) => k !== "remote")) img.removeAttribute("srcset");
      else img.setAttribute("srcset", parts.map(([url, ...size]) => [secure(url), ...size].join(" ")).join(", "));
    }
  }
  for (const el of body.querySelectorAll("[background]")) {
    const url = el.getAttribute("background")!;
    const kind = urlKind(url);
    if (kind === "remote") remoteContent = true;
    if (kind !== "remote" || !allowImages) el.removeAttribute("background");
    else el.setAttribute("background", secure(url));
  }

  for (const a of body.querySelectorAll("a[href]")) {
    if (!/^(https?:|mailto:)/i.test(a.getAttribute("href")!.trim())) a.removeAttribute("href");
  }

  const hasQuoted = markQuotes(body, quoteIsNew);
  for (const el of body.querySelectorAll(`[${quoteMark}]`)) {
    if (hasQuoted && !showQuoted) el.remove();
    else el.removeAttribute(quoteMark);
  }

  // Line breaks and empty blocks left at the end (often where the quote was).
  for (let last = body.lastChild; last && isEmpty(last); last = body.lastChild) last.remove();
  if (!designed) closeUpEmptyLines(body);

  const css = styles.join("\n");
  return { body: body.innerHTML, styles: css, remoteContent, hasQuoted, designed, darkVersion: darkRule.test(css), trackers };
}

export interface EmailDocumentOptions {
  body: string;
  styles: string;
  designed: boolean;
  darkVersion: boolean;
  theme: Theme;
  readingSize: ReadingSize;
  allowImages: boolean;
}

const darkRule = /prefers-color-scheme\s*:\s*dark/i;
const schemeQuery = /\(\s*prefers-color-scheme\s*:\s*(dark|light)\s*\)/gi;

/**
 * Answers the email's `prefers-color-scheme` queries with Moss's theme rather than the Mac's.
 * Each query becomes one that is always true or never true, so it works whatever the frame's engine reads it as.
 */
function matchScheme(css: string, scheme: Theme): string {
  return css.replace(schemeQuery, (_, asked: string) =>
    asked.toLowerCase() === scheme ? "(min-width: 0px)" : "((max-width: 0px) and (min-width: 1px))",
  );
}

const latinRange =
  "U+0000-00FF,U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+0304,U+0308,U+0329,U+2000-206F,U+20AC,U+2122,U+2191,U+2193,U+2212,U+2215,U+FEFF,U+FFFD";
const latinExtRange =
  "U+0100-02BA,U+02BD-02C5,U+02C7-02CC,U+02CE-02D7,U+02DD-02FF,U+0304,U+0308,U+0329,U+1D00-1DBF,U+1E00-1E9F,U+1EF2-1EFF,U+2020,U+20A0-20AB,U+20AD-20C0,U+2113,U+2C60-2C7F,U+A720-A7FF";

const fontFace = (family: string, url: string, range: string) =>
  `@font-face { font-family: "${family}"; font-weight: 300 900; font-display: swap; src: url("${new URL(url, location.href).href}") format("woff2-variations"); unicode-range: ${range}; }`;

/** The frame can't see the app's stylesheets, so it gets its own copy of the font. */
const fontFaces = [
  fontFace("Figtree Variable", figtreeLatinExt, latinExtRange),
  fontFace("Figtree Variable", figtreeLatin, latinRange),
].join("\n");

/**
 * Simple email reads like the rest of Moss: reading-size type (18/31 by default) in the body colour, following the theme.
 * Designed email is zoomed by the same ratio, so its own layout stays in proportion.
 * The `!important`s win over the inline colours and fonts that mail apps add (Outlook's black Calibri).
 * Designed email keeps its own look on white paper.
 */
const baseCss = `
html, body { margin: 0; overflow: hidden; }
#moss-root { display: flow-root; }
#moss-root img { max-width: 100%; }
/* Placeholders keep the size the email gave them; the pixel inside would make height:auto square. */
#moss-root img:not([data-moss-blocked]) { height: auto; }
#moss-root img[data-moss-blocked] { background: var(--raise); }
#moss-root img[data-moss-broken] { display: none !important; }
html.designed { background: #ffffff; color-scheme: light; }
/* An email with its own dark version draws it on dark paper; text it leaves uncoloured turns light. */
html.designed[data-theme="dark"] { background: var(--surface); color-scheme: dark; }
/* Like Gmail: a margin inside the paper, and a sans-serif for email that names no font. */
html.designed body { overflow-x: auto; font-family: Arial, Helvetica, sans-serif; }
html.designed #moss-root { padding: 24px; zoom: var(--reading-scale); }
html.simple { background: transparent; }
html.simple body {
  background: transparent !important; color: var(--body) !important; font-family: var(--font); font-size: var(--reading-size); line-height: var(--reading-leading);
  letter-spacing: 0.2px; overflow-wrap: anywhere; -webkit-font-smoothing: antialiased;
}
html.simple #moss-root * { color: inherit !important; background-color: transparent !important; font-family: inherit !important; }
html.simple #moss-root :is(p, div, span, font, td, th, li, a, b, strong, i, em, u, blockquote, center) {
  font-size: inherit !important; line-height: inherit !important; letter-spacing: inherit !important;
}
html.simple #moss-root :is(pre, code, tt, kbd, samp) { font-family: ui-monospace, monospace !important; font-size: calc(15px * var(--reading-scale)); }
html.simple #moss-root pre { white-space: pre-wrap; }
html.simple #moss-root :is(h1, h2, h3, h4) { color: var(--ink) !important; font-size: calc(22px * var(--reading-scale)); line-height: 1.4; margin: 0 0 16px; }
html.simple #moss-root a { color: var(--ai) !important; text-decoration: underline; text-underline-offset: 3px; }
html.simple #moss-root :is(p, ul, ol) { margin: 0 0 16px; }
html.simple #moss-root blockquote { margin: 0 0 16px; padding-left: 16px; border-left: 2px solid var(--line); color: var(--ink-2) !important; }
html.simple #moss-root hr { border: 0; border-top: 1px solid var(--line); }
html.simple #moss-root table { max-width: 100%; }
html.simple #moss-fit > :last-child { margin-bottom: 0; }
/* Signatures step back, so what was written stands out. */
html.simple #moss-root .gmail_signature, html.simple #moss-root .gmail_signature * {
  color: var(--ink-3) !important; font-size: calc(15px * var(--reading-scale)) !important; line-height: calc(24px * var(--reading-scale)) !important;
}
html.simple #moss-root .gmail_signature :is(p, ul, ol) { margin: 0 0 4px; }
html.simple #moss-root .gmail_signature a { color: var(--ink-2) !important; }
html.simple #moss-root .gmail_signature_prefix { color: var(--ink-3) !important; }
`;

/** A complete document for the frame's srcdoc. The CSP allows no scripts, forms or network, except images once allowed. */
export function emailDocument({ body, styles, designed, darkVersion, theme, readingSize, allowImages }: EmailDocumentOptions): string {
  // Designed email is drawn for white paper unless it brings its own dark version: forcing dark colours onto it breaks it.
  const scheme = designed && !darkVersion ? "light" : theme;
  const csp = [
    "default-src 'none'",
    "script-src 'none'",
    "object-src 'none'",
    "base-uri 'none'",
    "form-action 'none'",
    `img-src data: cid:${allowImages ? " https: http:" : ""}`,
    "style-src 'unsafe-inline'",
    `font-src ${location.origin} data:`,
  ].join("; ");
  return `<!doctype html><html class="${designed ? "designed" : "simple"}" data-theme="${scheme}" data-reading="${readingSize}"><head>
<meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${csp}"><meta name="referrer" content="no-referrer">
<style>${fontFaces}\n${tokens}</style>
<style>${matchScheme(styles, scheme).replace(/<\/style/gi, "<\\/style")}</style>
<style>${baseCss}</style>
</head><body><div id="moss-root"><div id="moss-fit">${body}</div></div></body></html>`;
}

/** Splits a plain-text email into what was written and the quoted history at its end ("On … wrote:" and `>` lines). */
export function splitQuotedText(text: string): { body: string; quoted?: string } {
  const all = text.replace(/\r\n?/g, "\n").trimEnd();
  const lines = all.split("\n");
  const isQuote = (l: string) => l.startsWith(">");
  let start = lines.length;
  while (start > 0 && (isQuote(lines[start - 1]) || !lines[start - 1].trim())) start--;
  while (start < lines.length && !lines[start].trim()) start++;
  if (start === lines.length) return { body: all };

  let k = start - 1;
  while (k >= 0 && !lines[k].trim()) k--;
  if (k >= 0 && attribution.test(lines[k])) start = k;
  // Gmail wraps a long attribution: "On …, Lena Vogt <lena@…>" / "wrote:".
  else if (k > 0 && /^\s*wrote:\s*$/i.test(lines[k]) && /^\s*On\b/.test(lines[k - 1])) start = k - 1;

  const body = lines.slice(0, start).join("\n").trimEnd();
  if (!body.trim()) return { body: all };
  return { body, quoted: lines.slice(start).join("\n") };
}

export type TextPart = string | { text: string; href: string };

/** Splits text into plain runs and web links. Trailing punctuation and unbalanced brackets stay out of links. */
export function linkify(text: string): TextPart[] {
  const parts: TextPart[] = [];
  let last = 0;
  for (const m of text.matchAll(/\b(?:https?:\/\/|www\.)[^\s<>"']+/gi)) {
    let url = m[0];
    for (;;) {
      const end = url.at(-1)!;
      const unbalanced = (open: string, close: string) => end === close && url.split(open).length < url.split(close).length;
      if (/[.,;:!?]/.test(end) || unbalanced("(", ")") || unbalanced("[", "]")) url = url.slice(0, -1);
      else break;
    }
    if (m.index > last) parts.push(text.slice(last, m.index));
    parts.push({ text: url, href: /^www\./i.test(url) ? `https://${url}` : url });
    last = m.index + url.length;
  }
  if (last < text.length) parts.push(text.slice(last));
  return parts;
}
