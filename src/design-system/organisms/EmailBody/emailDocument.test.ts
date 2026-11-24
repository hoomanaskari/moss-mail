import { describe, expect, it } from "vitest";
import { emailDocument, linkify, prepareHtml, splitQuotedText } from "./emailDocument";

const show = { allowImages: false, showQuoted: true };

/** Parses prepared body HTML so tests can query it like a page. */
function parse(body: string) {
  const el = document.createElement("div");
  el.innerHTML = body;
  return el;
}

/** Parses a whole frame document. DOMParser documents have no window, so assertions read attributes directly. */
function parseDoc(html: string) {
  return new DOMParser().parseFromString(html, "text/html");
}

describe("prepareHtml: sanitising", () => {
  it("removes scripts, event handlers and javascript: links", () => {
    const { body } = prepareHtml(
      `<p onclick="steal()">Hi</p><script>steal()</script><a href="javascript:steal()">x</a><img src="data:image/png;base64,AA" onerror="steal()">`,
      show,
    );
    expect(body).not.toMatch(/script|onclick|onerror|javascript:/i);
    expect(body).toContain("Hi");
  });

  it("removes forms, inputs, base, meta refresh, frames, embeds and media", () => {
    const { body, styles } = prepareHtml(
      `<html><head><base href="https://evil.example/"><meta http-equiv="refresh" content="0;url=https://evil.example"></head><body>
        <form action="https://evil.example"><input name="password"><button>Go</button></form>
        <iframe src="https://evil.example"></iframe><object data="x"></object><embed src="x">
        <video poster="https://t.example/p.jpg"></video><audio src="https://t.example/a.mp3"></audio>
        <svg><image href="https://t.example/p.gif"/></svg><p>Kept</p></body></html>`,
      show,
    );
    const all = body + styles;
    expect(all).not.toMatch(/<(form|input|button|base|meta|iframe|object|embed|video|audio|svg|image)\b/i);
    expect(all).not.toContain("evil.example");
    expect(body).toContain("Kept");
  });

  it("strips CSS @import, expression() and -moz-binding from style blocks and attributes", () => {
    const { body, styles } = prepareHtml(
      `<html><head><style>@import url("https://t.example/a.css"); .a { width: expression(alert(1)); -moz-binding: url(x.xml#b); color: red; }</style></head>
       <body><p style="width: expression(alert(1)); color: blue">x</p></body></html>`,
      show,
    );
    expect(styles).not.toMatch(/@import|expression|binding/i);
    expect(styles).toContain("color: red");
    expect(body).not.toMatch(/expression/i);
    expect(body).toContain("color: blue");
  });

  it("keeps the email's own style blocks from the head and the body", () => {
    const { styles } = prepareHtml(
      `<html><head><style>.a { color: red }</style></head><body><style>.b { color: blue }</style><p class="a">x</p></body></html>`,
      show,
    );
    expect(styles).toContain(".a { color: red }");
    expect(styles).toContain(".b { color: blue }");
  });

  it("keeps web and mailto links and drops every other kind", () => {
    const { body } = prepareHtml(
      `<a href="https://a.example/x">a</a><a href="mailto:lena@northwind.co">m</a><a href="#top">h</a><a href="/relative">r</a><a href="ftp://x">f</a><a>n</a>`,
      show,
    );
    const links = [...parse(body).querySelectorAll("a")].map((a) => a.getAttribute("href"));
    expect(links).toEqual(["https://a.example/x", "mailto:lena@northwind.co", null, null, null, null]);
  });

  it("drops link pings, which tell the sender you clicked", () => {
    const { body } = prepareHtml(`<a href="https://a.example" ping="https://t.example/ping">a</a>`, show);
    expect(body).not.toContain("ping");
  });

  it("drops link targets, so a click navigates the frame (where the desktop app catches it) instead of a blocked new window", () => {
    const { body } = prepareHtml(
      `<a href="https://a.example" target="_blank">a</a><a href="https://b.example" target="_top">b</a><map><area href="https://c.example" target="_blank"></map>`,
      show,
    );
    expect(parse(body).querySelectorAll("[target]")).toHaveLength(0);
  });

  it("drops image maps, whose areas are links Moss's click handler would not see", () => {
    const { body } = prepareHtml(
      `<img src="data:image/png;base64,AA" usemap="#m"><map name="m"><area shape="rect" coords="0,0,9,9" href="https://evil.example"></map><p>Kept</p>`,
      show,
    );
    const doc = parse(body);
    expect(doc.querySelectorAll("map, area, [usemap]")).toHaveLength(0);
    expect(body).not.toContain("evil.example");
    expect(body).toContain("Kept");
  });

  it("drops every attribute that could send the frame somewhere: targets, pings, form actions and xlink:href", () => {
    const { body } = prepareHtml(
      `<a href="https://a.example" target="_top" ping="https://t.example" xlink:href="https://x.example">a</a>` +
        `<p formaction="https://f.example" action="https://g.example">p</p>`,
      show,
    );
    expect(body).not.toMatch(/target|ping|formaction|action|xlink|t\.example|x\.example|f\.example|g\.example/i);
    expect(parse(body).querySelector("a")!.getAttribute("href")).toBe("https://a.example");
  });
});

describe("prepareHtml: remote content", () => {
  const html = `<html><head><style>.hero { background: url("https://t.example/bg.png") no-repeat; }</style></head><body>
    <img src="https://t.example/open.gif" width="1" height="1" alt="">
    <img src="http://img.example/hero.jpg" srcset="https://img.example/hero@2x.jpg 2x" alt="Autumn sale">
    <img src="//img.example/proto.jpg" alt="p">
    <img src="data:image/png;base64,AAAA" alt="inline">
    <img src="cid:logo@example" alt="logo">
    <img src="/app/secret.png" alt="relative">
    <table background="https://t.example/table.png"><tr><td style="background-image: url(https://t.example/cell.png); color: red">x</td></tr></table>
    <picture><source srcset="https://img.example/a.webp"><img src="https://img.example/a.jpg" alt="pic"></picture>
  </body></html>`;

  it("holds back remote images, backgrounds and CSS urls by default", () => {
    const { body, styles, remoteContent } = prepareHtml(html, show);
    expect(remoteContent).toBe(true);
    expect(body + styles).not.toMatch(/t\.example|img\.example/);
    expect(styles).toContain("no-repeat");
    expect(body).toContain("color: red");
  });

  it("shows blocked images as an empty placeholder of the same size", () => {
    const hero = parse(prepareHtml(`<img src="https://img.example/hero.jpg" width="600" height="260" alt="Autumn sale">`, show).body).querySelector("img")!;
    expect(hero.getAttribute("src")).toMatch(/^data:image\/gif;base64,/);
    expect(hero).toHaveAttribute("data-moss-blocked");
    expect(hero).toHaveAttribute("width", "600");
    expect(hero).toHaveAttribute("height", "260");
  });

  describe("trackers", () => {
    const trackers = [
      `<img src="https://t.example/a.gif" width="1" height="1" alt="">`,
      `<img src="https://t.example/b.gif" style="width:0" height="2" alt="">`,
      `<img src="https://t.example/c.gif" style="display: none" alt="">`,
      `<img src="https://acme.us1.list-manage.com/track/open.php?u=1&id=2" alt="">`,
      `<img src="https://u1.ct.sendgrid.net/wf/open?upn=abc" alt="">`,
      `<img src="https://email.mg.acme.com/o/eyJhbGciOiJIUzI1NiJ9abcdefghijk" alt="">`,
      `<img src="https://news.acme.com/logo.png?r=maya%40northwind.co" alt="">`,
      `<img src="https://news.acme.com/b.png?who=MAYA@northwind.co" alt="">`,
    ];
    const hero = `<img src="https://cdn.acme.com/open-day/hero.jpg" width="600" alt="Open day">`;
    const options = { ...show, recipient: "maya@northwind.co" };

    it.each([false, true])("removes and counts them whether or not images are shown (shown: %s)", (allowImages) => {
      const got = prepareHtml(`<p>x</p>${trackers.join("")}${hero}`, { ...options, allowImages });
      expect(got.trackers).toBe(trackers.length);
      expect(parse(got.body).querySelectorAll("img")).toHaveLength(1);
      expect(got.remoteContent).toBe(true);
    });

    it("needs no Show images when trackers were the only remote content", () => {
      const got = prepareHtml(`<p>x</p>${trackers[0]}`, options);
      expect(got).toMatchObject({ trackers: 1, remoteContent: false });
    });

    it("keeps ordinary images whose address only looks a little like tracking", () => {
      const images = `<img src="https://cdn.acme.com/opening-hours.png" alt=""><img src="https://cdn.acme.com/o/logo.png" alt="">`;
      expect(prepareHtml(images, { ...options, allowImages: true }).trackers).toBe(0);
    });

    it("still finds your address in an image address with a broken escape", () => {
      const img = `<img src="https://news.acme.com/c.png?bad=%E0%A4%A&to=maya@northwind.co" alt="">`;
      expect(prepareHtml(img, options).trackers).toBe(1);
    });

    it("finds the recipient's address only when it knows it", () => {
      expect(prepareHtml(trackers[6], show).trackers).toBe(0);
    });
  });

  it("keeps inline data: images and drops relative ones, which would load from the app", () => {
    const doc = parse(prepareHtml(html, show).body);
    expect(doc.querySelector('img[alt="inline"]')).toHaveAttribute("src", "data:image/png;base64,AAAA");
    expect(doc.querySelector('img[alt="relative"]')).not.toHaveAttribute("src");
    expect(doc.querySelector('img[alt="relative"]')).not.toHaveAttribute("data-moss-blocked");
  });

  it("keeps cid: images for attachments shown inline", () => {
    const doc = parse(prepareHtml(html, show).body);
    expect(doc.querySelector('img[alt="logo"]')).toHaveAttribute("src", "cid:logo@example");
  });

  it("loads remote images and backgrounds once allowed", () => {
    const { body, styles, remoteContent } = prepareHtml(html, { ...show, allowImages: true });
    expect(remoteContent).toBe(true);
    const doc = parse(body);
    expect(doc.querySelector('img[alt="Autumn sale"]')).toHaveAttribute("src", "https://img.example/hero.jpg");
    expect(doc.querySelector('img[alt="Autumn sale"]')).toHaveAttribute("srcset", "https://img.example/hero@2x.jpg 2x");
    expect(doc.querySelector('img[alt="p"]')).toHaveAttribute("src", "https://img.example/proto.jpg");
    expect(doc.querySelector("table")).toHaveAttribute("background", "https://t.example/table.png");
    expect(body).toContain("url(https://t.example/cell.png)");
    expect(styles).toContain("https://t.example/bg.png");
  });

  it("asks for plain http and scheme-less images over https, since Moss only lets https images load", () => {
    const { body, styles } = prepareHtml(
      `<style>.a { background: url('HTTP://img.example/s.png') }</style>
      <img src=" http://img.example/logo.png" srcset="http://img.example/l1.png 1x, //img.example/l2.png 2x" alt="logo">
      <table background="//img.example/t.png"><tr><td style="background: url(http://img.example/c.png)">x</td></tr></table>`,
      { ...show, allowImages: true },
    );
    const doc = parse(body);
    expect(doc.querySelector("img")).toHaveAttribute("src", "https://img.example/logo.png");
    expect(doc.querySelector("img")).toHaveAttribute("srcset", "https://img.example/l1.png 1x, https://img.example/l2.png 2x");
    expect(doc.querySelector("table")).toHaveAttribute("background", "https://img.example/t.png");
    expect(body).toContain("url(https://img.example/c.png)");
    expect(styles).toContain("url('https://img.example/s.png')");
    expect(body + styles).not.toMatch(/http:|\(\/\/|"\/\//i);
  });

  it("leaves https and inline image addresses as they are", () => {
    const { body } = prepareHtml(`<img src="https://img.example/a.png" alt="a"><img src="data:image/png;base64,AA" alt="b">`, {
      ...show,
      allowImages: true,
    });
    const doc = parse(body);
    expect(doc.querySelector('img[alt="a"]')).toHaveAttribute("src", "https://img.example/a.png");
    expect(doc.querySelector('img[alt="b"]')).toHaveAttribute("src", "data:image/png;base64,AA");
  });

  it("drops a non-image srcset even when allowed", () => {
    const { body } = prepareHtml(`<img src="https://a.example/a.jpg" srcset="/x.jpg 2x" alt="a">`, { ...show, allowImages: true });
    expect(parse(body).querySelector("img")).not.toHaveAttribute("srcset");
  });

  it("keeps inline data: urls in CSS and drops relative ones", () => {
    const { body, remoteContent } = prepareHtml(
      `<div style="background: url(data:image/png;base64,AA)">a</div><div style="background: url('/app/x.png')">b</div><table background="bg.png"><tr><td>c</td></tr></table>`,
      show,
    );
    expect(remoteContent).toBe(false);
    expect(body).toContain("url(data:image/png;base64,AA)");
    expect(body).not.toContain("/app/x.png");
    expect(body).not.toContain("bg.png");
  });

  it("leaves an image with no address alone", () => {
    const { body, remoteContent } = prepareHtml(`<img alt="nothing">`, show);
    expect(remoteContent).toBe(false);
    expect(parse(body).querySelector("img")).toHaveAttribute("alt", "nothing");
  });

  it("reports no remote content for a self-contained email", () => {
    expect(prepareHtml(`<p>Just text</p><img src="data:image/png;base64,AA" alt="">`, show).remoteContent).toBe(false);
  });
});

describe("prepareHtml: quoted history", () => {
  it("collapses a Gmail quote, attribution included", () => {
    const html = `<div dir="ltr">Sounds good, see you Friday.</div><br><div class="gmail_quote"><div class="gmail_attr">On Sun, 1 Nov 2026 at 09:12, Lena Vogt &lt;lena@northwind.co&gt; wrote:<br></div><blockquote class="gmail_quote">Can we meet Friday?</blockquote></div>`;
    const hidden = prepareHtml(html, { allowImages: false, showQuoted: false });
    expect(hidden.hasQuoted).toBe(true);
    expect(hidden.body).toContain("Sounds good");
    expect(hidden.body).not.toContain("Can we meet");
    expect(hidden.body).not.toContain("wrote:");
    const shown = prepareHtml(html, show);
    expect(shown.hasQuoted).toBe(true);
    expect(shown.body).toContain("Can we meet");
  });

  it("collapses an Apple Mail cite and the “On … wrote:” line before it", () => {
    const html = `<div>Thanks!</div><div><br><div>On 1 Nov 2026, at 09:12, Lena Vogt wrote:</div><br><blockquote type="cite"><div>Notes attached.</div></blockquote></div>`;
    const { body, hasQuoted } = prepareHtml(html, { allowImages: false, showQuoted: false });
    expect(hasQuoted).toBe(true);
    expect(body).toContain("Thanks!");
    expect(body).not.toMatch(/Notes attached|wrote:/);
  });

  it("collapses a plain blockquote only when an “On … wrote:” line introduces it", () => {
    const quoted = `<p>Agreed.</p><p>On Sunday, Lena wrote:</p><blockquote><p>Shall we?</p></blockquote>`;
    expect(prepareHtml(quoted, { allowImages: false, showQuoted: false }).body).not.toContain("Shall we?");
    const plain = `<p>As Ada Lovelace put it:</p><blockquote><p>That brain of mine is something more than merely mortal.</p></blockquote>`;
    const kept = prepareHtml(plain, { allowImages: false, showQuoted: false });
    expect(kept.hasQuoted).toBe(false);
    expect(kept.body).toContain("merely mortal");
  });

  it("collapses an “On … wrote:” text line directly before a blockquote", () => {
    const html = `<div>Yes.<br><br>On Sun, Lena wrote:<blockquote>Lunch?</blockquote></div>`;
    const { body, hasQuoted } = prepareHtml(html, { allowImages: false, showQuoted: false });
    expect(hasQuoted).toBe(true);
    expect(body).toContain("Yes.");
    expect(body).not.toMatch(/Lunch\?|wrote:/);
  });

  it("collapses an Outlook reply header and everything after it", () => {
    const html = `<div>Approved.</div><hr style="display:inline-block;width:98%"><div id="divRplyFwdMsg"><b>From:</b> Daniel</div><div>The old message</div>`;
    const { body, hasQuoted } = prepareHtml(html, { allowImages: false, showQuoted: false });
    expect(hasQuoted).toBe(true);
    expect(body).toContain("Approved.");
    expect(body).not.toMatch(/From:|old message|<hr/);
  });

  it("drops the blank lines left at the end once the quote is gone", () => {
    const html = `<div dir="ltr">Thanks,<br>Priya</div><br>\n<div><br></div><div class="gmail_quote">On Sun, Maya wrote:<blockquote>Hi</blockquote></div>`;
    expect(prepareHtml(html, { allowImages: false, showQuoted: false }).body).toBe(`<div dir="ltr">Thanks,<br>Priya</div>`);
  });

  it("collapses an Outlook reply header that has no rule before it", () => {
    const html = `<div>Approved.</div><div id="divRplyFwdMsg"><b>From:</b> Daniel</div><div>The old message</div>`;
    const { body } = prepareHtml(html, { allowImages: false, showQuoted: false });
    expect(body).toBe("<div>Approved.</div>");
  });

  it("collapses a nested quote once, as part of its outer quote", () => {
    const html = `<p>Top</p><blockquote type="cite">Mid<blockquote type="cite">Deep</blockquote></blockquote>`;
    const { body } = prepareHtml(html, { allowImages: false, showQuoted: false });
    expect(body).toContain("Top");
    expect(body).not.toMatch(/Mid|Deep/);
  });

  it("never hides a forwarded message", () => {
    const html = `<div>FYI</div><div class="gmail_quote"><div class="gmail_attr">---------- Forwarded message ---------<br>From: Ines</div><div>The plan</div></div>`;
    const { body, hasQuoted } = prepareHtml(html, { allowImages: false, showQuoted: false });
    expect(hasQuoted).toBe(false);
    expect(body).toContain("The plan");
  });

  it("shows the quote when it is the whole email", () => {
    const html = `<div class="gmail_quote">Only a quote</div>`;
    const { body, hasQuoted } = prepareHtml(html, { allowImages: false, showQuoted: false });
    expect(hasQuoted).toBe(false);
    expect(body).toContain("Only a quote");
  });

  it("keeps an email whose only other content is an image", () => {
    const html = `<img src="data:image/png;base64,AA" alt=""><blockquote type="cite">Old</blockquote>`;
    expect(prepareHtml(html, { allowImages: false, showQuoted: false }).hasQuoted).toBe(true);
  });
});

describe("prepareHtml: layout", () => {
  it("treats a short reply as simple, so it gets the reading typography", () => {
    expect(prepareHtml(`<div dir="ltr">Hi Maya,<br><br>Notes attached.</div>`, show).designed).toBe(false);
  });

  it("treats an Outlook reply with Office styles as simple", () => {
    const html = `<html><head><style>p.MsoNormal { margin: 0; font-family: Calibri; }</style></head><body><p class="MsoNormal" style="color:black">Approved.</p></body></html>`;
    expect(prepareHtml(html, show).designed).toBe(false);
  });

  it.each([
    ["a background colour attribute", `<table bgcolor="#f4f4f4"><tr><td>x</td></tr></table>`],
    ["a coloured inline background", `<div style="background-color: #1a73e8">x</div>`],
    ["several layout tables", `<table><tr><td><table><tr><td><table><tr><td>x</td></tr></table></td></tr></table></td></tr></table>`],
    ["a wide image", `<img src="https://img.example/hero.jpg" width="600" alt="">`],
  ])("treats an email with %s as designed", (_, html) => {
    expect(prepareHtml(html, show).designed).toBe(true);
  });

  it("keeps a background set on <body>, which newsletters often use", () => {
    const { body, designed } = prepareHtml(`<html><body bgcolor="#f4f4f4" style="margin: 0"><p>x</p></body></html>`, show);
    expect(designed).toBe(true);
    expect(parse(body).querySelector("div")).toHaveAttribute("bgcolor", "#f4f4f4");
  });

  it("treats a reply whose signature highlights a few words as simple", () => {
    const html = `<div dir="ltr">Sounds good, let me know.</div><div class="gmail_signature">
      <p style="text-align:center"><font style="background-color:rgb(0,51,0)"><strong>Fernhill Builders</strong></font></p>
      <p><span style="background-color:rgb(147,196,125);color:rgb(0,0,0)"><b>Open</b> Monday to Friday</span></p>
      <p><mark>New</mark> <a style="background:#ffe08a" href="https://studio-build.example">studio-build.example</a></p></div>`;
    expect(prepareHtml(html, show).designed).toBe(false);
  });

  it("closes up runs of empty lines in simple email, such as the gap a removed quote leaves", () => {
    const html = `<div>AM</div><br><div class="gmail_quote">On Sat, Alex wrote:<blockquote>old</blockquote></div><div><br clear="all"></div><div><br></div><p> </p><span>-- </span><br><div>Alex</div>`;
    const { body } = prepareHtml(html, { allowImages: false, showQuoted: false });
    expect(body).toBe(`<div>AM</div><br><span>-- </span><br><div>Alex</div>`);
  });

  it("leaves empty cells and lines alone in designed email, where they space the layout", () => {
    const html = `<table bgcolor="#f4f4f4"><tr><td>a</td></tr></table><div><br></div><div><br></div><p>b</p>`;
    expect(prepareHtml(html, show).body).toContain(`<div><br></div><div><br></div>`);
  });

  it("does not count white or transparent backgrounds as design", () => {
    const html = `<div style="background-color: #ffffff">a</div><div style="background: white">b</div><div style="background-color: transparent">c</div><table bgcolor="#FFF"><tr><td>d</td></tr></table>`;
    expect(prepareHtml(html, show).designed).toBe(false);
  });
});

describe("prepareHtml: dark version", () => {
  it("notices an email that brings its own dark version", () => {
    const html = `<html><head><style>.card { background: #fff } @media (prefers-color-scheme: dark) { .card { background: #111 } }</style></head><body><p class="card">x</p></body></html>`;
    expect(prepareHtml(html, show).darkVersion).toBe(true);
  });

  it.each([
    ["no colour scheme rules", `<style>.card { background: #fff }</style><p class="card">x</p>`],
    ["only a light colour scheme rule", `<style>@media (prefers-color-scheme: light) { .card { background: #fff } }</style><p class="card">x</p>`],
    ["only a color-scheme meta tag", `<html><head><meta name="color-scheme" content="light dark"></head><body><p>x</p></body></html>`],
  ])("finds no dark version in an email with %s", (_, html) => {
    expect(prepareHtml(html, show).darkVersion).toBe(false);
  });

  it("keeps the classes on the email's own <html> and <body>, which its dark rules hang from", () => {
    // An email builder: `.email-builder-canvas .color-fg { color: white }`, with the class on <html>, which the frame replaces.
    const html = `<html class="email-builder-canvas" dir="rtl" lang="fa" style="height: 100%"><head>
      <style>@media (prefers-color-scheme: dark) { .email-builder-canvas .color-fg { color: #fff !important } }</style>
      </head><body class="email-body" id="top"><table bgcolor="#0052ff"><tr><td><p class="color-fg" style="color: #0a0b0d">x</p></td></tr></table></body></html>`;
    const text = parse(prepareHtml(html, show).body).querySelector(".color-fg")!;
    const page = text.closest(".email-builder-canvas")!;
    expect(page).toHaveAttribute("dir", "rtl");
    expect(page).toHaveAttribute("lang", "fa");
    expect(page).not.toHaveAttribute("style");
    expect(text.closest(".email-body")).toHaveAttribute("id", "top");
    expect(page.contains(text.closest(".email-body"))).toBe(true);
  });

  it.each([
    ["carry nothing", `<html><body><p>x</p></body></html>`],
    ["only name a simple email, which follows Moss's colours anyway", `<html lang="en" class="mail"><body class="WordSection1" dir="ltr"><p>x</p></body></html>`],
  ])("adds no wrappers when <html> and <body> %s", (_, html) => {
    expect(prepareHtml(html, show).body).toBe("<p>x</p>");
  });
});

describe("emailDocument", () => {
  const base = {
    body: "<p>Hi</p>",
    styles: "",
    designed: false,
    darkVersion: false,
    theme: "dark" as const,
    readingSize: "default" as const,
    allowImages: false,
  };

  it("is a whole document with the body inside a measurable root", () => {
    expect(parseDoc(emailDocument(base)).getElementById("moss-root")!.textContent).toBe("Hi");
  });

  it("locks the document down with a CSP: no scripts, no network, images only once allowed", () => {
    const csp = (allowImages: boolean) =>
      parseDoc(emailDocument({ ...base, allowImages }))
        .querySelector('meta[http-equiv="Content-Security-Policy"]')!
        .getAttribute("content")!;
    // The CSP is the only thing stopping the email's own scripts (the frame allows scripts so Moss's listeners run),
    // so it must sit in <head>, before anything from the email.
    const doc = parseDoc(emailDocument({ ...base, body: "<p>Hi</p>", styles: "p{color:red}" }));
    expect(doc.head.firstElementChild!.nextElementSibling!.getAttribute("http-equiv")).toBe("Content-Security-Policy");
    const blocked = csp(false);
    expect(blocked).toContain("default-src 'none'");
    expect(blocked).toContain("script-src 'none'");
    expect(blocked).toContain("form-action 'none'");
    expect(blocked).toMatch(/img-src data: cid:;/);
    expect(csp(true)).toMatch(/img-src data: cid: https: http:;/);
  });

  it("follows the app's theme and reading size for simple email", () => {
    const root = parseDoc(emailDocument({ ...base, theme: "light", readingSize: "larger" })).documentElement;
    expect(root.dataset).toMatchObject({ theme: "light", reading: "larger" });
    expect(root.dataset.font).toBeUndefined();
    expect(root.className).toBe("simple");
  });

  // The frame is its own document: the app's <html data-reading> doesn't reach it, so it carries its own.
  it("sizes simple email with the reading size tokens, not a fixed size", () => {
    const html = emailDocument(base);
    expect(html).toMatch(/html\.simple body \{[^}]*font-size: var\(--reading-size\); line-height: var\(--reading-leading\);/);
    expect(html).not.toMatch(/font-size: 18px; line-height: 31px/);
  });

  it("scales the type around the body (headings, code, signatures) with the reading size", () => {
    const html = emailDocument(base);
    for (const px of [22, 15]) expect(html).toContain(`calc(${px}px * var(--reading-scale))`);
    expect(html).not.toMatch(/font-size: (22|15)px/);
    expect(html).toMatch(/line-height: calc\(24px \* var\(--reading-scale\)\)/);
  });

  it("zooms a designed email with the reading size, keeping its own layout in proportion", () => {
    expect(emailDocument({ ...base, designed: true })).toMatch(/html\.designed #moss-root \{[^}]*zoom: var\(--reading-scale\)/);
  });

  it("puts a designed email on light paper, whatever the theme", () => {
    const root = parseDoc(emailDocument({ ...base, designed: true })).documentElement;
    expect(root.className).toBe("designed");
    expect(root.dataset.theme).toBe("light");
  });

  describe("an email's own dark version", () => {
    const styles = `.card { background: #fff } @media (prefers-color-scheme: dark) { .card { background: #111 } } @media screen and (prefers-color-scheme: light) { .card { color: #000 } }`;
    /** The conditions of the email's @media rules, in order, as the frame will read them. */
    const conditions = (html: string) => [...parseDoc(html).querySelectorAll("style")].flatMap((s) => [...s.textContent!.matchAll(/@media ([^{]+?)\s*\{/g)].map((m) => m[1]));
    const on = "(min-width: 0px)";
    const off = "((max-width: 0px) and (min-width: 1px))";

    it("shows a designed email's own dark version when Moss is dark", () => {
      const html = emailDocument({ ...base, designed: true, darkVersion: true, styles });
      expect(parseDoc(html).documentElement.dataset.theme).toBe("dark");
      expect(conditions(html)).toEqual([on, `screen and ${off}`]);
    });

    it("shows the light version when Moss is light, even if the Mac is dark", () => {
      const html = emailDocument({ ...base, theme: "light", designed: true, darkVersion: true, styles });
      expect(parseDoc(html).documentElement.dataset.theme).toBe("light");
      expect(conditions(html)).toEqual([off, `screen and ${on}`]);
    });

    it("reads “not dark” the right way round", () => {
      const html = emailDocument({ ...base, designed: true, darkVersion: true, styles: `@media not (prefers-color-scheme: dark) { .a { color: red } }` });
      expect(conditions(html)).toEqual([`not ${on}`]);
    });

    it("keeps an email without a dark version on light paper and its light rules", () => {
      const html = emailDocument({ ...base, designed: true, styles: `@media (prefers-color-scheme: light) { .a { color: red } }` });
      expect(parseDoc(html).documentElement.dataset.theme).toBe("light");
      expect(conditions(html)).toEqual([on]);
    });

    it("leaves other @media rules alone", () => {
      const html = emailDocument({ ...base, designed: true, darkVersion: true, styles: `@media (max-width: 600px) { .a { width: 100% } }` });
      expect(conditions(html)).toEqual(["(max-width: 600px)"]);
    });
  });

  it("sets a signature smaller than the message, so what was written stands out", () => {
    const frame = document.body.appendChild(document.createElement("iframe"));
    const doc = frame.contentDocument!;
    doc.open();
    doc.write(
      emailDocument({ ...base, body: `<p>Sounds good.</p><div class="gmail_signature"><p><font size="4">Fernhill</font></p></div>` }),
    );
    doc.close();
    const view = frame.contentWindow!;
    // jsdom keeps calc() as written; 15px at the default size, scaled with the rest.
    const signature = "calc(15px * var(--reading-scale))";
    expect(view.getComputedStyle(doc.querySelector("font")!).fontSize).toBe(signature);
    expect(view.getComputedStyle(doc.querySelector("p")!).fontSize).not.toBe(signature);
    frame.remove();
  });

  it("keeps a designed email off the paper's edges, in a sans-serif when it names no font", () => {
    // A parsed document has no window to compute styles in; a frame does.
    const frame = document.body.appendChild(document.createElement("iframe"));
    const doc = frame.contentDocument!;
    doc.open();
    doc.write(emailDocument({ ...base, designed: true }));
    doc.close();
    const view = frame.contentWindow!;
    expect(view.getComputedStyle(doc.getElementById("moss-root")!).padding).toBe("24px");
    expect(view.getComputedStyle(doc.body).fontFamily).toBe("Arial, Helvetica, sans-serif");
    frame.remove();
  });

  it("loads images without telling the sender where they were opened", () => {
    const referrer = parseDoc(emailDocument(base)).querySelector('meta[name="referrer"]')!;
    expect(referrer.getAttribute("content")).toBe("no-referrer");
  });

  it("includes the email's own styles, and cannot be broken out of with </style>", () => {
    const html = emailDocument({ ...base, styles: ".a { color: red } </style><script>x</script>" });
    expect(html).toContain(".a { color: red }");
    expect(html).not.toContain("</style><script>");
  });

  it("brings the app's fonts along, since the frame cannot see the app's stylesheets", () => {
    expect(emailDocument(base)).toMatch(/@font-face\s*{\s*font-family: "Figtree Variable"/);
    expect(emailDocument(base)).not.toContain("Atkinson");
  });
});

describe("splitQuotedText", () => {
  it("separates a trailing quote and its “On … wrote:” line", () => {
    const text = "Sounds good.\n\nMaya\n\nOn Sun, 1 Nov 2026, Lena Vogt wrote:\n> Can we meet?\n>\n> Lena\n";
    expect(splitQuotedText(text)).toEqual({ body: "Sounds good.\n\nMaya", quoted: "On Sun, 1 Nov 2026, Lena Vogt wrote:\n> Can we meet?\n>\n> Lena" });
  });

  it("includes an attribution Gmail wrapped onto two lines, and the blank line after it", () => {
    const text = "Yes.\n\nOn Sun, 1 Nov 2026 at 09:12, Lena Vogt <lena@northwind.co>\nwrote:\n\n> Lunch?";
    expect(splitQuotedText(text)).toEqual({ body: "Yes.", quoted: "On Sun, 1 Nov 2026 at 09:12, Lena Vogt <lena@northwind.co>\nwrote:\n\n> Lunch?" });
  });

  it("separates a trailing quote with no attribution line", () => {
    expect(splitQuotedText("Yes.\n> Lunch?\n> Friday")).toEqual({ body: "Yes.", quoted: "> Lunch?\n> Friday" });
  });

  it("leaves text with no trailing quote alone", () => {
    expect(splitQuotedText("Hi.\n> one quoted line\nmy answer")).toEqual({ body: "Hi.\n> one quoted line\nmy answer" });
    expect(splitQuotedText("Just text\n")).toEqual({ body: "Just text" });
  });

  it("keeps a message that is nothing but a quote", () => {
    expect(splitQuotedText("> all quote")).toEqual({ body: "> all quote" });
  });
});

describe("linkify", () => {
  it("finds web links and leaves trailing punctuation out", () => {
    expect(linkify("See https://a.example/x?y=1, then www.b.example. (Or http://c.example/a_b)")).toEqual([
      "See ",
      { text: "https://a.example/x?y=1", href: "https://a.example/x?y=1" },
      ", then ",
      { text: "www.b.example", href: "https://www.b.example" },
      ". (Or ",
      { text: "http://c.example/a_b", href: "http://c.example/a_b" },
      ")",
    ]);
  });

  it("returns plain text untouched", () => {
    expect(linkify("No links here")).toEqual(["No links here"]);
    expect(linkify("")).toEqual([]);
  });
});

describe("prepareHtml: a quote that is new to you", () => {
  const fresh = { allowImages: false, showQuoted: false, quoteIsNew: true };
  // Apple Mail in Turkish: no English "forwarded message" anywhere.
  const appleForward = `<div dir="ltr"></div><div dir="ltr"><br></div><div dir="ltr"><br>İleti başlangıcı:<br><br></div><blockquote type="cite"><div dir="ltr"><b>Kimden:</b> SDG &lt;billing@sdg.example&gt;<br><b>Konu:</b> Invoice 5674<br><br></div></blockquote><blockquote type="cite"><div dir="ltr">Your invoice is due.</div></blockquote>`;

  it("keeps a forward open in any language when it starts the thread", () => {
    const { body, hasQuoted } = prepareHtml(appleForward, fresh);
    expect(hasQuoted).toBe(false);
    expect(body).toContain("Kimden:");
    expect(body).toContain("Your invoice is due.");
  });

  it("still collapses the same quote further down a thread, where you can read it above", () => {
    const { body, hasQuoted } = prepareHtml(appleForward, { allowImages: false, showQuoted: false });
    expect(hasQuoted).toBe(true);
    expect(body).not.toContain("Your invoice is due.");
  });

  it("keeps the newest forwarded email open and collapses the history inside it", () => {
    const html = `<div>See below.</div><blockquote type="cite"><div>Latest news.</div><div>On Sat, Ines wrote:</div><blockquote type="cite">Older news.</blockquote></blockquote>`;
    const hidden = prepareHtml(html, fresh);
    expect(hidden.hasQuoted).toBe(true);
    expect(hidden.body).toContain("Latest news.");
    expect(hidden.body).not.toContain("Older news.");
    expect(hidden.body).not.toContain("Ines wrote:");
    expect(prepareHtml(html, { ...fresh, showQuoted: true }).body).toContain("Older news.");
  });

  it("keeps a Gmail quote whole: its “On … wrote:” line and the text under it", () => {
    const html = `<div dir="ltr">Sounds good.</div><div class="gmail_quote"><div class="gmail_attr">On Sun, Lena wrote:<br></div><blockquote class="gmail_quote">Can we meet Friday?<div class="gmail_quote"><div class="gmail_attr">On Sat, Maya wrote:</div><blockquote class="gmail_quote">Lunch?</blockquote></div></blockquote></div>`;
    const { body, hasQuoted } = prepareHtml(html, fresh);
    expect(hasQuoted).toBe(true);
    expect(body).toContain("On Sun, Lena wrote:");
    expect(body).toContain("Can we meet Friday?");
    expect(body).not.toContain("Lunch?");
    expect(body).not.toContain("On Sat, Maya wrote:");
  });

  it("keeps an Outlook forward open and collapses the older email under it", () => {
    const html = `<div>FYI</div><hr><div id="divRplyFwdMsg"><b>From:</b> Ines</div><div>The plan</div><hr><div id="divRplyFwdMsg"><b>From:</b> Jonas</div><div>The old plan</div>`;
    const { body, hasQuoted } = prepareHtml(html, fresh);
    expect(hasQuoted).toBe(true);
    expect(body).toContain("The plan");
    expect(body).not.toContain("The old plan");
    expect(body).not.toContain("Jonas");
  });

  it("collapses the history inside a forward wherever it is in the thread", () => {
    const html = `<div>FYI</div><div class="gmail_quote"><div class="gmail_attr">---------- Forwarded message ---------<br>From: Ines</div><div>The plan<div class="gmail_quote"><div class="gmail_attr">On Sat, Jonas wrote:</div><blockquote class="gmail_quote">The old plan</blockquote></div></div></div>`;
    const { body, hasQuoted } = prepareHtml(html, { allowImages: false, showQuoted: false });
    expect(hasQuoted).toBe(true);
    expect(body).toContain("The plan");
    expect(body).not.toContain("The old plan");
  });
});
