import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useRemoteImages } from "../../../state/remoteImages";
import type { Message } from "../../../data/types";
import { EmailBody, type EmailBodyProps } from "./EmailBody";

type Body = Pick<Message, "id" | "html" | "text" | "paragraphs">;

function setup(body: Partial<Body>, props: Partial<EmailBodyProps> = {}) {
  const onShowImages = vi.fn();
  const onAllowImages = vi.fn();
  const onOpenLink = vi.fn();
  const all = {
    message: { id: "m1", paragraphs: [], ...body },
    theme: "dark" as const,
    readingSize: "default" as const,
    imagesAllowed: false,
    recipient: "maya@northwind.co",
    onShowImages,
    onAllowImages,
    onOpenLink,
    ...props,
  };
  const view = render(<EmailBody {...all} />);
  return { ...view, onShowImages, onAllowImages, onOpenLink, user: userEvent.setup(), props: all };
}

const frame = () => screen.getByTitle("Email") as HTMLIFrameElement;

/** jsdom has no layout, so a test says how wide the email's layout is and how much room it has. */
function setWidths(fit: HTMLElement, { layout, room }: { layout: number; room: number }) {
  Object.defineProperty(fit, "scrollWidth", { configurable: true, value: layout });
  Object.defineProperty(fit, "clientWidth", { configurable: true, value: room });
}

/** jsdom doesn't load srcdoc, so write it into the frame by hand and fire load, as a browser would. */
function loadFrame(height = 0, widths?: { layout: number; room: number }, loaded?: (doc: Document) => void) {
  const iframe = frame();
  const doc = iframe.contentDocument!;
  doc.open();
  doc.write(iframe.srcdoc);
  doc.close();
  loaded?.(doc);
  const root = doc.getElementById("moss-root")!;
  root.getBoundingClientRect = () => ({ height }) as DOMRect;
  const fit = doc.getElementById("moss-fit")!;
  if (widths) setWidths(fit, widths);
  fireEvent.load(iframe);
  return { doc, root, fit };
}

afterEach(() => vi.unstubAllGlobals());

describe("EmailBody: paragraphs (demo mail)", () => {
  it("shows each paragraph", () => {
    setup({ paragraphs: ["Hi Maya,", "Notes attached."] });
    expect(screen.getByText("Hi Maya,").tagName).toBe("P");
    expect(screen.getByText("Notes attached.")).toBeInTheDocument();
    expect(screen.queryByTitle("Email")).not.toBeInTheDocument();
  });
});

describe("EmailBody: plain text", () => {
  it("keeps line breaks and splits paragraphs on blank lines", () => {
    setup({ text: "Hi Maya,\n\nLine one\nLine two", paragraphs: ["ignored"] });
    expect(screen.getByText("Hi Maya,")).toBeInTheDocument();
    expect(screen.getByText(/Line one/).textContent).toBe("Line one\nLine two");
    expect(screen.queryByText("ignored")).not.toBeInTheDocument();
  });

  it("escapes markup instead of rendering it", () => {
    const { container } = setup({ text: "<b>not bold</b> <img src=x onerror=alert(1)>" });
    expect(screen.getByText(/<b>not bold<\/b>/)).toBeInTheDocument();
    expect(container.querySelector("b, img")).toBeNull();
  });

  it("turns web addresses into links that open in the browser", async () => {
    const { user, onOpenLink } = setup({ text: "Agenda: https://northwind.co/agenda." });
    const link = screen.getByRole("link", { name: "https://northwind.co/agenda" });
    expect(link).toHaveAttribute("href", "https://northwind.co/agenda");
    await user.click(link);
    expect(onOpenLink).toHaveBeenCalledWith("https://northwind.co/agenda");
  });

  it("collapses quoted history behind a toggle", async () => {
    const { user } = setup({ text: "Sounds good.\n\nOn Sun, Lena Vogt wrote:\n> Can we meet?" });
    expect(screen.getByText("Sounds good.")).toBeInTheDocument();
    expect(screen.queryByText(/Can we meet/)).not.toBeInTheDocument();
    const toggle = screen.getByRole("button", { name: "Show quoted text" });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    await user.click(toggle);
    expect(screen.getByText(/Can we meet/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Hide quoted text" })).toHaveAttribute("aria-expanded", "true");
  });

  it("has no toggle without quoted history", () => {
    setup({ text: "Just this." });
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
});

describe("EmailBody: HTML", () => {
  it("shows the email in a frame that can run no scripts", () => {
    setup({ html: "<p>Hello <b>there</b></p><script>alert(1)</script>" });
    const iframe = frame();
    // WebKit (the Mac app) never runs Moss's own listeners in a frame without allow-scripts, so keys and
    // link clicks inside the email went nowhere. The email's CSP is what keeps its scripts from running.
    expect(iframe.getAttribute("sandbox")).toBe("allow-same-origin allow-scripts");
    expect(iframe.srcdoc).toContain("<b>there</b>");
    expect(iframe.srcdoc).not.toContain("alert(1)");
    expect(iframe.srcdoc).toContain("script-src 'none'");
  });

  it("prefers the HTML body over the plain text", () => {
    setup({ html: "<p>From HTML</p>", text: "From text", paragraphs: ["From paragraphs"] });
    expect(frame().srcdoc).toContain("From HTML");
    expect(screen.queryByText("From text")).not.toBeInTheDocument();
  });

  it("follows the theme and reading size", () => {
    const { rerender, props } = setup({ html: "<p>Hi</p>" });
    expect(frame().srcdoc).toContain('data-theme="dark" data-reading="default"');
    rerender(<EmailBody {...props} theme="light" readingSize="smaller" />);
    expect(frame().srcdoc).toContain('data-theme="light" data-reading="smaller"');
  });

  it("puts designed email on a paper card", () => {
    setup({ html: `<table bgcolor="#1a73e8"><tr><td>Sale</td></tr></table>` });
    expect(frame().srcdoc).toContain('class="designed"');
    expect(screen.getByTestId("email-paper")).toBeInTheDocument();
  });

  it("shows a newsletter's own dark version in the dark theme, and keeps others light", () => {
    const table = `<table bgcolor="#1a73e8"><tr><td>Sale</td></tr></table>`;
    const { rerender, props } = setup({ html: `<style>@media (prefers-color-scheme: dark) { td { color: #fff } }</style>${table}` });
    expect(frame().srcdoc).toContain('class="designed" data-theme="dark"');
    expect(screen.getByTestId("email-paper")).toBeInTheDocument();
    rerender(<EmailBody {...props} theme="light" />);
    expect(frame().srcdoc).toContain('class="designed" data-theme="light"');
    rerender(<EmailBody {...props} message={{ id: "m2", paragraphs: [], html: table }} />);
    expect(frame().srcdoc).toContain('class="designed" data-theme="light"');
  });

  describe("a dark version that leaves its text dark", () => {
    const words = Array.from({ length: 20 }, () => "word").join(" ");
    const dark = `<style>@media (prefers-color-scheme: dark) { .card { border: 0 } }</style>`;
    const card = (ink: string) => `${dark}<table style="background-color: #111111"><tr><td style="color: ${ink}">${words}</td></tr></table>`;

    it("shows the light version instead, so it can be read", () => {
      setup({ html: card("#0a0b0d") });
      expect(frame().srcdoc).toContain('class="designed" data-theme="dark"');
      loadFrame();
      expect(frame().srcdoc).toContain('class="designed" data-theme="light"');
    });

    it("keeps a dark version that can be read", () => {
      setup({ html: card("#ffffff") });
      loadFrame();
      expect(frame().srcdoc).toContain('class="designed" data-theme="dark"');
    });

    it("gives the next email its own chance at dark", () => {
      const { rerender, props } = setup({ html: card("#0a0b0d") });
      loadFrame();
      rerender(<EmailBody {...props} message={{ id: "m2", paragraphs: [], html: card("#fafafa") }} />);
      expect(frame().srcdoc).toContain('class="designed" data-theme="dark"');
    });
  });

  it("does not use the paper card for simple email", () => {
    setup({ html: "<p>Hi</p>" });
    expect(screen.queryByTestId("email-paper")).not.toBeInTheDocument();
  });

  describe("remote images", () => {
    const html = `<p>Sale</p><img src="https://img.example/hero.jpg" alt="Hero">`;
    const pixel = `<img src="https://t.example/open.gif" width="1" height="1" alt="">`;

    it("hides them and offers to show them from this sender, or just this once", async () => {
      const { user, onShowImages, onAllowImages } = setup({ html });
      expect(screen.getByText("Images are hidden")).toBeInTheDocument();
      expect(frame().srcdoc).not.toContain("img.example");
      await user.click(screen.getByRole("button", { name: "Always for this senderShift+I" }));
      expect(onAllowImages).toHaveBeenCalledOnce();
      await user.click(screen.getByRole("button", { name: "Just this onceAlt+I" }));
      expect(onShowImages).toHaveBeenCalledOnce();
    });

    it("says why when the sender is one you trust but Moss couldn't confirm the email is from them", async () => {
      const { user, onShowImages } = setup({ html }, { imagesAllowed: false, senderUnconfirmed: true });
      expect(screen.getByText("Images are off because Moss couldn't confirm who sent this.")).toBeInTheDocument();
      expect(screen.queryByText("Images are hidden")).not.toBeInTheDocument();
      expect(frame().srcdoc).not.toContain("img.example");
      // They're allowed already, so only this email can be let through.
      expect(screen.queryByRole("button", { name: /Always for this sender/ })).not.toBeInTheDocument();
      await user.click(screen.getByRole("button", { name: "Just this onceAlt+I" }));
      expect(onShowImages).toHaveBeenCalledOnce();
    });

    it("says how many trackers it removed, while hidden and once shown", () => {
      const { rerender, props } = setup({ html: html + pixel + pixel.replace("open", "o2") });
      expect(screen.getByText("2 possible trackers removed")).toBeInTheDocument();
      rerender(<EmailBody {...props} imagesAllowed />);
      expect(screen.getByText("2 possible trackers removed from this email")).toBeInTheDocument();
      expect(frame().srcdoc).toContain("https://img.example/hero.jpg");
      expect(frame().srcdoc).not.toContain("t.example");
      rerender(<EmailBody {...props} message={{ ...props.message, html: html + pixel }} imagesAllowed />);
      expect(screen.getByText("1 possible tracker removed from this email")).toBeInTheDocument();
    });

    it("needs no notice when trackers were the only remote content", () => {
      setup({ html: "<p>Hi</p>" + pixel });
      expect(screen.queryByText(/Images are hidden/)).not.toBeInTheDocument();
      expect(screen.getByText("1 possible tracker removed from this email")).toBeInTheDocument();
    });

    it("finds trackers that carry your address", () => {
      setup({ html: `<p>Hi</p><img src="https://news.acme.com/b.png?e=maya%40northwind.co" alt="">` }, { imagesAllowed: true });
      expect(frame().srcdoc).not.toContain("news.acme.com");
    });

    /** jsdom loads no images: say how each one went. A width of 0 is an image that couldn't load. */
    const images = (doc: Document, outcome: Record<string, { complete: boolean; width: number }>) => {
      for (const img of doc.querySelectorAll("img")) {
        const { complete, width } = outcome[img.alt] ?? { complete: true, width: 0 };
        Object.defineProperty(img, "complete", { value: complete });
        Object.defineProperty(img, "naturalWidth", { value: width });
      }
    };

    it("leaves out an image its server won't send, as if the email never had it", () => {
      const imgs = `<img src="https://img.example/logo.png" alt="Logo" width="155"><img src="https://img.example/hero.jpg" alt="Hero"><img src="https://img.example/late.png" alt="Late">`;
      setup({ html: `<p>Sale</p>${imgs}` }, { imagesAllowed: true });
      const { doc } = loadFrame(0, undefined, (doc) =>
        images(doc, { Logo: { complete: true, width: 0 }, Hero: { complete: true, width: 600 }, Late: { complete: false, width: 0 } }),
      );
      const img = (alt: string) => doc.querySelector(`img[alt="${alt}"]`)!;
      expect(img("Logo")).not.toBeVisible();
      expect(img("Hero")).toBeVisible();
      // Still loading when the email was ready, then failing.
      expect(img("Late")).toBeVisible();
      fireEvent.error(img("Late"));
      expect(img("Late")).not.toBeVisible();
      // Other things that fail to load stay as they are.
      fireEvent.error(doc.body);
      expect(doc.body).toBeVisible();
    });

    it("keeps the placeholders of images it is holding back", () => {
      setup({ html });
      const { doc } = loadFrame(0, undefined, (doc) => images(doc, {}));
      expect(doc.querySelector("img")).toBeVisible();
    });

    it("loads them once allowed, and the notice goes", () => {
      setup({ html }, { imagesAllowed: true });
      expect(screen.queryByText(/Images are hidden/)).not.toBeInTheDocument();
      expect(screen.queryByText(/removed/)).not.toBeInTheDocument();
      expect(frame().srcdoc).toContain("https://img.example/hero.jpg");
    });

    it("shows no notice when there is nothing remote", () => {
      setup({ html: "<p>Hi</p>" });
      expect(screen.queryByText(/Images are hidden/)).not.toBeInTheDocument();
    });
  });

  it("collapses quoted history behind a toggle", async () => {
    const { user } = setup({
      html: `<div>Sounds good.</div><div class="gmail_quote"><div class="gmail_attr">On Sun, Lena wrote:</div><blockquote>Can we meet?</blockquote></div>`,
    });
    expect(frame().srcdoc).not.toContain("Can we meet?");
    await user.click(screen.getByRole("button", { name: "Show quoted text" }));
    expect(frame().srcdoc).toContain("Can we meet?");
    await user.click(screen.getByRole("button", { name: "Hide quoted text" }));
    expect(frame().srcdoc).not.toContain("Can we meet?");
  });

  it("grows to the height of its content, so the thread scrolls as one page", () => {
    setup({ html: "<p>Hi</p>" });
    loadFrame(420.4);
    expect(frame().style.height).toBe("421px");
  });

  it("shrinks a layout wider than the column to fit, instead of cutting it off", () => {
    setup({ html: `<table width="800"><tr><td>Account</td><td>Customer Care</td></tr></table>` });
    const { fit } = loadFrame(100, { layout: 800, room: 600 });
    expect(fit.style.zoom).toBe("0.75");
    // A wider window has room for it at full size again.
    setWidths(fit, { layout: 800, room: 800 });
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(fit.style.zoom).toBe("");
  });

  it("leaves a layout that fits at full size", () => {
    setup({ html: "<p>Hi</p>" });
    expect(loadFrame(100, { layout: 600, room: 600 }).fit.style.zoom).toBe("");
  });

  it("measures again when an image loads or the window resizes", () => {
    setup({ html: `<p>Hi</p><img src="data:image/png;base64,AA" alt="">` });
    const { doc, root } = loadFrame(100);
    root.getBoundingClientRect = () => ({ height: 300 }) as DOMRect;
    act(() => {
      doc.querySelector("img")!.dispatchEvent(new Event("load"));
    });
    expect(frame().style.height).toBe("300px");
    root.getBoundingClientRect = () => ({ height: 200 }) as DOMRect;
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(frame().style.height).toBe("200px");
  });

  it("measures again when the content changes size, and stops watching when it unmounts", () => {
    const observers: { callback: () => void; disconnect: ReturnType<typeof vi.fn> }[] = [];
    vi.stubGlobal(
      "ResizeObserver",
      class {
        disconnect = vi.fn();
        constructor(public callback: () => void) {
          observers.push(this);
        }
        observe() {}
      },
    );
    const { unmount } = setup({ html: "<p>Hi</p>" });
    const { root } = loadFrame(100);
    root.getBoundingClientRect = () => ({ height: 250 }) as DOMRect;
    act(() => observers[0].callback());
    expect(frame().style.height).toBe("250px");
    unmount();
    expect(observers[0].disconnect).toHaveBeenCalled();
  });

  it("stops watching the old document when the frame reloads", () => {
    const resize = vi.spyOn(window, "removeEventListener");
    setup({ html: "<p>Hi</p>" });
    loadFrame();
    loadFrame();
    expect(resize).toHaveBeenCalledWith("resize", expect.any(Function));
  });

  it("opens clicked links in the browser instead of inside the frame", () => {
    const { onOpenLink } = setup({ html: `<p><a href="https://northwind.co/agenda"><b>Agenda</b></a> <a>no address</a></p>` });
    const { doc } = loadFrame();
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    doc.querySelector("b")!.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    expect(onOpenLink).toHaveBeenCalledWith("https://northwind.co/agenda");

    const bare = new MouseEvent("click", { bubbles: true, cancelable: true });
    doc.querySelectorAll("a")[1].dispatchEvent(bare);
    expect(bare.defaultPrevented).toBe(true);
    expect(onOpenLink).toHaveBeenCalledOnce();
  });

  it("catches a click on an image map area too, so it can never navigate the frame itself", () => {
    const { onOpenLink } = setup({ html: "<p>Map</p>" });
    // The sanitiser drops maps; this one stands for any that got through.
    const { doc } = loadFrame(0, undefined, (d) => {
      d.body.insertAdjacentHTML("beforeend", `<map name="m"><area href="https://evil.example"><area></map>`);
    });
    const [web, bare] = doc.querySelectorAll("area");
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    web.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(true);
    expect(onOpenLink).toHaveBeenCalledWith("https://evil.example");

    const empty = new MouseEvent("click", { bubbles: true, cancelable: true });
    bare.dispatchEvent(empty);
    expect(empty.defaultPrevented).toBe(true);
    expect(onOpenLink).toHaveBeenCalledOnce();
  });

  it("waits for the frame's document when the frame reports loading before it has one", () => {
    setup({ html: "<p>Hi</p>" });
    const before = frame().style.height;
    Object.defineProperty(frame(), "contentDocument", { configurable: true, value: null });
    expect(() => fireEvent.load(frame())).not.toThrow();
    expect(frame().style.height).toBe(before);
  });

  it("ignores clicks that are not on links", () => {
    const { onOpenLink } = setup({ html: "<p>Text</p>" });
    const { doc } = loadFrame();
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    doc.querySelector("p")!.dispatchEvent(click);
    expect(click.defaultPrevented).toBe(false);
    expect(onOpenLink).not.toHaveBeenCalled();
  });

  it("passes keys pressed inside the frame on to the app's shortcuts", () => {
    const seen: KeyboardEvent[] = [];
    const listener = (e: KeyboardEvent) => {
      seen.push(e);
      if (e.key === "j") e.preventDefault();
    };
    window.addEventListener("keydown", listener);
    setup({ html: "<p>Text</p>" });
    const { doc } = loadFrame();
    const j = new KeyboardEvent("keydown", { key: "j", code: "KeyJ", bubbles: true, cancelable: true });
    doc.body.dispatchEvent(j);
    const i = new KeyboardEvent("keydown", { key: "I", code: "KeyI", shiftKey: true, bubbles: true, cancelable: true });
    doc.body.dispatchEvent(i);
    window.removeEventListener("keydown", listener);
    expect(seen.map((e) => [e.key, e.code, e.shiftKey])).toEqual([
      ["j", "KeyJ", false],
      ["I", "KeyI", true],
    ]);
    expect(j.defaultPrevented).toBe(true);
    expect(i.defaultPrevented).toBe(false);
  });

  it("passes key releases on too, so holding ⌘ inside the email still shows the shortcut hints", () => {
    const seen: string[] = [];
    const listener = (e: KeyboardEvent) => seen.push(`${e.type}:${e.key}`);
    window.addEventListener("keyup", listener);
    setup({ html: "<p>Text</p>" });
    const { doc } = loadFrame();
    doc.body.dispatchEvent(new KeyboardEvent("keyup", { key: "Meta", bubbles: true }));
    window.removeEventListener("keyup", listener);
    expect(seen).toEqual(["keyup:Meta"]);
  });
});


describe("strict remote images", () => {
  afterEach(() => useRemoteImages.setState({ policy: "familiar", saved: true }));
  it("blocks familiar content immediately, permits a per-message choice, and hides sender exemptions", () => {
    const { rerender, props } = setup({ html: '<img src="https://images.example.test/photo.jpg" width="200" height="100">' }, { imagesAllowed: true });
    expect(frame().srcdoc).toContain('src="https://images.example.test/photo.jpg"');
    act(() => useRemoteImages.setState({ policy: "ask" }));
    expect(frame().srcdoc).not.toContain('src="https://images.example.test/photo.jpg"');
    expect(screen.getByText("Images are hidden")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Always for this sender/ })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Just this once/ })).toBeInTheDocument();
    rerender(<EmailBody {...props} imagesShown />);
    expect(frame().srcdoc).toContain('src="https://images.example.test/photo.jpg"');
    rerender(<EmailBody {...props} />);
    act(() => useRemoteImages.setState({ policy: "familiar" }));
    expect(frame().srcdoc).toContain('src="https://images.example.test/photo.jpg"');
  });
});
