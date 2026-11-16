import { useRemoteImages } from "../../../state/remoteImages";
import { useEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import { ChevronUp, Ellipsis, ImageOff, ShieldCheck } from "lucide-react";
import { cx } from "../../../lib/cx";
import type { Message } from "../../../data/types";
import type { ReadingSize, Theme } from "../../../state/store";
import { Button } from "../../atoms";
import { Notice } from "../../molecules";
import { emailDocument, linkify, prepareHtml, splitQuotedText } from "./emailDocument";
import { hardToRead } from "./readability";
import styles from "./EmailBody.module.css";

export interface EmailBodyProps {
  message: Pick<Message, "id" | "html" | "text" | "paragraphs">;
  theme: Theme;
  /** Settings → Appearance → Reading size. The frame is its own document, so it is told rather than inheriting it. */
  readingSize: ReadingSize;
  /** Automatic sender-trust permission. Strict mode overrides it. */
  imagesAllowed: boolean;
  /** The sender is trusted, but the mail server didn't confirm this email is from them: say so. */
  senderUnconfirmed?: boolean;
  /** Explicit permission for this message, including strict mode. */
  imagesShown?: boolean;
  /** Your address, to spot trackers that carry it. */
  recipient?: string;
  /** Just this once. */
  onShowImages: () => void;
  /** From this sender, from now on. */
  onAllowImages: () => void;
  /** A link was clicked: open it outside Moss, in the system browser. */
  onOpenLink: (url: string) => void;
}

/** What a message says: HTML in a sandboxed frame, or plain text in Moss's reading type. */
export function EmailBody(props: EmailBodyProps) {
  const { html, text, paragraphs } = props.message;
  if (html) return <HtmlBody {...props} html={html} />;
  if (text !== undefined) return <TextBody text={text} onOpenLink={props.onOpenLink} />;
  return (
    <div className={styles.body}>
      <Paragraphs paragraphs={paragraphs} onOpenLink={props.onOpenLink} />
    </div>
  );
}

function QuoteToggle({ shown, onToggle }: { shown: boolean; onToggle: () => void }) {
  return (
    <Button variant="mini" icon={shown ? ChevronUp : Ellipsis} aria-expanded={shown} onClick={onToggle} className={styles.toggle}>
      {shown ? "Hide quoted text" : "Show quoted text"}
    </Button>
  );
}

function Paragraphs({ paragraphs, onOpenLink }: { paragraphs: string[]; onOpenLink: (url: string) => void }) {
  const open = (href: string) => (e: MouseEvent) => {
    e.preventDefault();
    onOpenLink(href);
  };
  return paragraphs.map((p, i) => (
    <p key={i} className={styles.paragraph}>
      {linkify(p).map((part, j) =>
        typeof part === "string" ? (
          part
        ) : (
          <a key={j} href={part.href} onClick={open(part.href)} className={styles.link}>
            {part.text}
          </a>
        ),
      )}
    </p>
  ));
}

const toParagraphs = (text: string) => text.split(/\n[ \t]*\n/).filter((p) => p.trim());

function TextBody({ text, onOpenLink }: { text: string; onOpenLink: (url: string) => void }) {
  const [showQuoted, setShowQuoted] = useState(false);
  const { body, quoted } = useMemo(() => splitQuotedText(text), [text]);
  return (
    <div className={styles.body}>
      <Paragraphs paragraphs={toParagraphs(body)} onOpenLink={onOpenLink} />
      {quoted && <QuoteToggle shown={showQuoted} onToggle={() => setShowQuoted(!showQuoted)} />}
      {quoted && showQuoted && (
        <div className={styles.quoted}>
          <Paragraphs paragraphs={toParagraphs(quoted)} onOpenLink={onOpenLink} />
        </div>
      )}
    </div>
  );
}

const trackerCount = (n: number) => `${n} possible ${n === 1 ? "tracker" : "trackers"} removed`;

function HtmlBody(props: EmailBodyProps & { html: string }) {
  const { html, theme, readingSize, recipient, senderUnconfirmed, onShowImages, onAllowImages, onOpenLink } = props;
  const strict = useRemoteImages((s) => s.policy === "ask");
  const imagesAllowed = props.imagesShown || (!strict && props.imagesAllowed);
  const [showQuoted, setShowQuoted] = useState(false);
  const [height, setHeight] = useState<number>();
  const frame = useRef<HTMLIFrameElement>(null);
  const stopWatching = useRef<() => void>(undefined);
  const openLink = useRef(onOpenLink);
  openLink.current = onOpenLink;

  const prepared = useMemo(
    () => prepareHtml(html, { allowImages: imagesAllowed, showQuoted, recipient }),
    [html, imagesAllowed, showQuoted, recipient],
  );
  // The email whose dark version turned out unreadable (dark text left on dark paper): it shows its light version.
  const [lightOnly, setLightOnly] = useState<string>();
  const darkVersion = prepared.darkVersion && lightOnly !== html;
  const darkPaper = prepared.designed && darkVersion && theme === "dark";
  const srcDoc = useMemo(
    () => emailDocument({ ...prepared, darkVersion, theme, readingSize, allowImages: imagesAllowed }),
    [prepared, darkVersion, theme, readingSize, imagesAllowed],
  );

  useEffect(() => () => stopWatching.current?.(), []);

  /**
   * The frame is same-origin and its CSP runs none of the email's scripts, so Moss wires it up from outside:
   * size it to its content, send link clicks to the browser and pass keys on to the app's shortcuts.
   */
  const onLoad = () => {
    stopWatching.current?.();
    const iframe = frame.current!;
    const doc = iframe.contentDocument;
    const root = doc?.getElementById("moss-root");
    const fit = doc?.getElementById("moss-fit");
    // The frame can report the blank document it starts with (or none) before the email has loaded.
    if (!doc || !root || !fit) return;
    if (darkPaper && hardToRead(doc)) {
      setLightOnly(html);
      return;
    }

    // An image its server won't send (gone, or refusing) is left out, as if the email never had it.
    // Placeholders for images held back for privacy stay.
    const leaveOut = (img: Element) => {
      if (!img.hasAttribute("data-moss-blocked")) img.setAttribute("data-moss-broken", "");
    };
    for (const img of doc.images) if (img.complete && img.naturalWidth === 0) leaveOut(img);
    doc.addEventListener("error", (e) => leaveOut(e.target as Element), true);

    const measure = () => {
      // A layout wider than the column (fixed-width tables) shrinks to fit, as mail apps on a phone do, instead of being cut off.
      fit.style.zoom = "";
      const scale = fit.clientWidth / fit.scrollWidth;
      fit.style.zoom = scale < 1 ? String(scale) : "";
      setHeight(Math.ceil(root.getBoundingClientRect().height));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(root);
    doc.addEventListener("load", measure, true);
    window.addEventListener("resize", measure);

    doc.addEventListener("click", (e) => {
      // <area> is a link too: the sanitiser drops image maps, and this catches any that get through.
      const link = (e.target as Element).closest("a, area");
      if (!link) return;
      e.preventDefault();
      const href = link.getAttribute("href");
      if (href) openLink.current(href);
    });

    // Key presses and releases (holding ⌘ shows the shortcut hints) go on to the window, where every shortcut listens.
    const passOn = (e: KeyboardEvent) => {
      const { key, code, shiftKey, altKey, metaKey, ctrlKey, repeat } = e;
      const copy = new KeyboardEvent(e.type, { key, code, shiftKey, altKey, metaKey, ctrlKey, repeat, bubbles: true, cancelable: true });
      iframe.dispatchEvent(copy);
      if (copy.defaultPrevented) e.preventDefault();
    };
    doc.addEventListener("keydown", passOn);
    doc.addEventListener("keyup", passOn);

    stopWatching.current = () => {
      observer.disconnect();
      window.removeEventListener("resize", measure);
    };
  };

  const blocked = prepared.remoteContent && !imagesAllowed;

  return (
    <div className={styles.body}>
      {blocked ? (
        <Notice
          variant="inset"
          icon={ImageOff}
          action={
            <>
              <Button variant="quiet" shortcut="⌥I" onClick={onShowImages}>
                Just this once
              </Button>
              {!strict && !senderUnconfirmed && <Button variant="outlineSm" shortcut="⇧I" onClick={onAllowImages}>
                Always for this sender
              </Button>}
            </>
          }
        >
          {senderUnconfirmed ? "Images are off because Moss couldn't confirm who sent this." : "Images are hidden"}
          {prepared.trackers > 0 && (
            <>
              <span className={styles.trackers} aria-hidden>
                ·
              </span>
              <span className={styles.trackers}>{trackerCount(prepared.trackers)}</span>
            </>
          )}
        </Notice>
      ) : (
        prepared.trackers > 0 && (
          <Notice variant="footnote" icon={ShieldCheck}>
            {trackerCount(prepared.trackers)} from this email
          </Notice>
        )
      )}
      <div
        className={cx(styles.frameWrap, prepared.designed && styles.paper, darkPaper && styles.darkPaper)}
        data-testid={prepared.designed ? "email-paper" : undefined}
      >
        <iframe
          ref={frame}
          title="Email"
          // allow-scripts is for Moss's own listeners above: without it WebKit (the Mac app) never calls them,
          // so keys and link clicks inside the email went nowhere. The email's CSP (script-src 'none') still blocks its scripts.
          sandbox="allow-same-origin allow-scripts"
          srcDoc={srcDoc}
          onLoad={onLoad}
          className={styles.frame}
          style={{ height }}
        />
      </div>
      {prepared.hasQuoted && <QuoteToggle shown={showQuoted} onToggle={() => setShowQuoted(!showQuoted)} />}
    </div>
  );
}
