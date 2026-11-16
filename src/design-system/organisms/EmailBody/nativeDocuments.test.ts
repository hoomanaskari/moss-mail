import { writeFileSync } from "node:fs";
import { env } from "node:process";
import { expect, it } from "vitest";
import { createElement } from "react";
import { render } from "@testing-library/react";
import { useRemoteImages } from "../../../state/remoteImages";
import { EmailBody } from "./EmailBody";
import { emailDocument, prepareHtml } from "./emailDocument";

/** Optional export for native WKWebView verification. Every byte is synthetic. */
it("builds the native hostile-email corpus with the real sanitizer and CSP", () => {
  const origin = "http://127.0.0.1:12345";
  const fixtures = [false, true].map((allowed) => {
    const name = allowed ? "allowed" : "blocked";
    const target = `${origin}/${name}`;
    const raw = `<p>Native fixture ${name}</p>
<img src="${target}/image.png" width="30" height="30" onload="parent.compromised=true">
<img src="${target}/pixel.png" width="1" height="1">
<script>parent.compromised=true;fetch('${target}/fetch')</script>
<iframe src="${target}/frame"></iframe><object data="${target}/object"></object>
<form action="${target}/form"><input autofocus onfocus="parent.compromised=true"></form>
<meta http-equiv="refresh" content="0;url=${target}/refresh">
<link rel="stylesheet" href="${target}/style.css">
<style>@import url('${target}/import.css');p {background-image:url('${target}/background.png')}</style>`;
    const prepared = prepareHtml(raw, { allowImages: allowed, showQuoted: true });
    useRemoteImages.setState({ policy: "ask", saved: true });
    const view = render(createElement(EmailBody, {
      message: { id: name, html: raw, paragraphs: [] }, theme: "light", readingSize: "default",
      imagesAllowed: true, imagesShown: allowed,
      onShowImages: () => {}, onAllowImages: () => {}, onOpenLink: () => {},
    }));
    const frame = view.getByTitle("Email") as HTMLIFrameElement;
    const html = frame.srcdoc;
    const sandbox = frame.getAttribute("sandbox")!;
    view.unmount();
    useRemoteImages.setState({ policy: "familiar", saved: true });
    expect(sandbox).toBe("allow-same-origin allow-scripts");
    expect(prepared.body).not.toMatch(/<(script|iframe|object|form|meta|link)\b/i);
    expect(prepared.body).not.toContain("onload");
    expect(prepared.body).not.toContain("pixel.png");
    expect(prepared.body.includes("image.png")).toBe(allowed);
    return { name, html, allowed, sandbox };
  });
  // Defense in depth: feed hostile markup directly to the document wrapper.
  // This deliberately bypasses sanitization to verify native CSP enforcement.
  for (const allowed of [false, true]) {
    const name = allowed ? "csp-allowed" : "csp-blocked";
    const target = `${origin}/${name}`;
    const html = emailDocument({ body: `<p>Native fixture ${name}</p><img src="${target}/image.png" width="30" height="30" onload="parent.compromised=true"><script>parent.compromised=true;fetch('${target}/fetch')</script><script src="${target}/script.js"></script><iframe src="${target}/frame"></iframe><object data="${target}/object"></object>`, styles: `@import url('${target}/import.css');p {background-image:url('${target}/background.png')}`, designed: false, darkVersion: false, theme: "light", readingSize: "default", allowImages: allowed });
    expect(html).toContain("default-src 'none'");
    fixtures.push({ name, html, allowed, sandbox: fixtures[0].sandbox });
  }
  const output = env.MOSS_NATIVE_EMAIL_FIXTURES;
  if (output) writeFileSync(output, JSON.stringify(fixtures), { mode: 0o600 });
  for (const fixture of fixtures) expect(fixture.html).toContain("script-src 'none'");
  expect(fixtures.map(f => f.name)).toEqual(["blocked", "allowed", "csp-blocked", "csp-allowed"]);
});
