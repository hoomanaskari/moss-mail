import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

// Vitest hands CSS over empty (`?raw`) or as class-name maps (modules), so read the source from disk.
const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");
const tokens = read("./tokens.css");
const threadRow = read("../molecules/ThreadRow/ThreadRow.module.css");
const threadHeader = read("../organisms/ThreadHeader/ThreadHeader.module.css");
const messageView = read("../organisms/MessageView/MessageView.module.css");
const earlierMessages = read("../organisms/EarlierMessages/EarlierMessages.module.css");
const forwardedMessage = read("../organisms/ForwardedMessage/ForwardedMessage.module.css");
const emailBody = read("../organisms/EmailBody/EmailBody.module.css");
const threadScreen = read("../../screens/ThreadScreen.module.css");
const askPanel = read("../organisms/AskPanel/AskPanel.module.css");
const markdown = read("../atoms/Markdown/Markdown.module.css");

/**
 * Settings → Appearance → Reading size sets <html data-reading>. jsdom has no CSS engine, so these tests read the
 * stylesheets: every text size on the screens you read mail on must follow the setting, not stay a fixed px.
 */
const surfaces = {
  "inbox list (ThreadRow)": threadRow,
  "reading view (ThreadHeader)": threadHeader,
  "reading view (MessageView)": messageView,
  "reading view (EarlierMessages)": earlierMessages,
  "reading view (ForwardedMessage)": forwardedMessage,
  "reading view (ThreadScreen)": threadScreen,
  "email body (EmailBody)": emailBody,
};

/** The value of a custom property in the rules whose selector is exactly `selector`. */
function tokenIn(selector: string, name: string): string | undefined {
  for (const [, sel, body] of tokens.matchAll(/([^{}]+)\{([^}]*)\}/g)) {
    if (sel.replace(/\/\*[\s\S]*?\*\//g, "").trim() !== selector) continue;
    const value = body.match(new RegExp(`${name}:\\s*([^;]+);`))?.[1];
    if (value) return value.trim();
  }
}

describe("Reading size", () => {
  it("defines a scale for each size, in proportion to the email text (16 / 18 / 20 px)", () => {
    expect(tokenIn(":root", "--reading-scale")).toBe("1");
    expect(tokenIn(':root[data-reading="smaller"]', "--reading-scale")).toBe("calc(16 / 18)");
    expect(tokenIn(':root[data-reading="larger"]', "--reading-scale")).toBe("calc(20 / 18)");
  });

  it.each(Object.entries(surfaces))("scales every text size in the %s", (_, css) => {
    const sizes = [...css.matchAll(/font-size:\s*([^;]+);/g)].map((m) => m[1]);
    expect(sizes.length).toBeGreaterThan(0);
    for (const size of sizes) expect(size).toMatch(/^(calc\(\d+px \* var\(--reading-scale\)\)|var\(--reading-size\))$/);
  });

  it("the Ask Moss chat reads a step smaller than email: 14 / 16 / 18px, each with its own line height", () => {
    expect([":root", ':root[data-reading="smaller"]', ':root[data-reading="larger"]'].map((s) => [tokenIn(s, "--chat-size"), tokenIn(s, "--chat-leading")])).toEqual([
      ["16px", "28px"],
      ["14px", "24px"],
      ["18px", "31px"],
    ]);
    const rule = (selector: string, css: string) => css.match(new RegExp(`\\n${selector.replace(".", "\\.")} \\{([^}]*)\\}`))?.[1] ?? "";
    const value = (property: string, css: string) => css.match(new RegExp(`${property}:\\s*([^;]+);`))?.[1];
    // The answer, in the email body's colour and letter spacing; your question the same size.
    const paragraph = rule(".paragraph", emailBody);
    for (const selector of [".answer", ".bubble"]) {
      const css = rule(selector, askPanel);
      expect(value("font-size", css), selector).toBe("var(--chat-size)");
      expect(value("line-height", css), selector).toBe("var(--chat-leading)");
      expect(value("letter-spacing", css), selector).toBe(value("letter-spacing", paragraph));
    }
    expect(value("color", rule(".answer", askPanel))).toBe(value("color", paragraph));
    // Points, table cells and headings take it too: the global 1.2 line height would cramp them.
    expect(markdown).toMatch(/\.markdown \*,\s*\.inline \* \{[^}]*line-height: inherit;/);
  });

  it("grows the inbox rows with their text, so larger text still fits", () => {
    expect(threadRow).toMatch(/\.row \{[^}]*height: calc\(58px \* var\(--reading-scale\)\)/);
    expect(threadRow).toMatch(/\.later \{[^}]*height: calc\(52px \* var\(--reading-scale\)\)/);
  });
});
