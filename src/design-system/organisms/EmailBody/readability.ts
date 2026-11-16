/**
 * Checks that an email's own dark version can be read. Some dark versions are only half done:
 * they darken the paper but leave the text dark, so it sits black on black. Moss then shows the light version instead.
 */

interface Rgba {
  r: number;
  g: number;
  b: number;
  a: number;
}

/** Below this contrast text can't be read at all (WCAG asks 4.5 for body text; a broken dark version sits near 1). */
const minContrast = 2;
/** More than this share of the letters unreadable means the dark version is broken, not just some small print. */
const maxUnreadable = 0.2;

/** A computed colour, `rgb(…)` or `rgba(…)`. Anything else can't be judged. */
function parse(color: string): Rgba | undefined {
  const m = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)(?:\s*[,/]\s*([\d.]+))?\s*\)$/i.exec(color);
  if (!m) return undefined;
  return { r: +m[1], g: +m[2], b: +m[3], a: m[4] === undefined ? 1 : +m[4] };
}

/** `top` laid over the solid `under`. */
const over = (top: Rgba, under: Rgba): Rgba => ({
  r: top.r * top.a + under.r * (1 - top.a),
  g: top.g * top.a + under.g * (1 - top.a),
  b: top.b * top.a + under.b * (1 - top.a),
  a: 1,
});

function luminance({ r, g, b }: Rgba) {
  const lin = (v: number) => {
    const c = v / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrast(a: Rgba, b: Rgba) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** What the text sits on: the backgrounds from the element down to the paper, mixed. Undefined over a picture or with no paper. */
function backdrop(el: Element, style: (el: Element) => CSSStyleDeclaration): Rgba | undefined {
  const layers: Rgba[] = [];
  for (let n: Element | null = el; n; n = n.parentElement) {
    const s = style(n);
    if (s.backgroundImage && s.backgroundImage !== "none") return undefined;
    const bg = parse(s.backgroundColor);
    if (!bg || bg.a === 0) continue;
    layers.push(bg);
  }
  const base = layers.pop();
  if (!base || base.a < 1) return undefined;
  return layers.reduceRight((under, top) => over(top, under), base);
}

function hidden(el: Element, style: (el: Element) => CSSStyleDeclaration) {
  if (style(el).visibility === "hidden") return true;
  for (let n: Element | null = el; n; n = n.parentElement) {
    const s = style(n);
    if (s.display === "none" || s.opacity === "0") return true;
  }
  return false;
}

/** Much of the email's text can't be read against what it sits on. */
export function hardToRead(doc: Document): boolean {
  const view = doc.defaultView!;
  const cache = new Map<Element, CSSStyleDeclaration>();
  const style = (el: Element) => {
    let s = cache.get(el);
    if (!s) cache.set(el, (s = view.getComputedStyle(el)));
    return s;
  };

  let letters = 0;
  let unreadable = 0;
  const walker = doc.createTreeWalker(doc.body, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const count = node.textContent!.match(/[\p{L}\p{N}]/gu)?.length ?? 0;
    const el = node.parentElement!;
    if (!count || hidden(el, style)) continue;
    const ink = parse(style(el).color);
    const under = backdrop(el, style);
    if (!ink || !under) continue;
    letters += count;
    if (contrast(over(ink, under), under) < minContrast) unreadable += count;
  }
  return letters > 0 && unreadable / letters > maxUnreadable;
}
