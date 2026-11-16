import { expect, it } from "vitest";
import { textToHtml } from "./textHtml";

it("turns plain text into paragraphs, escaped, with line breaks kept", () => {
  expect(textToHtml("")).toBe("");
  expect(textToHtml("Hi <Dana> & co,\n\nYes.\nBest")).toBe("<p>Hi &lt;Dana&gt; &amp; co,</p><p>Yes.<br>Best</p>");
});
