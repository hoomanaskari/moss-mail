import { describe, expect, it } from "vitest";
import { hardToRead } from "./readability";

/** A frame document on dark paper, as Moss draws an email's own dark version. */
function page(body: string, paper = "rgb(28, 28, 28)") {
  const frame = document.createElement("iframe");
  document.body.replaceChildren(frame);
  const doc = frame.contentDocument!;
  doc.documentElement.style.backgroundColor = paper;
  doc.body.innerHTML = body;
  return doc;
}

const words = (n: number) => Array.from({ length: n }, () => "word").join(" ");

describe("hardToRead", () => {
  it("finds dark text left on dark paper", () => {
    expect(hardToRead(page(`<p style="color: rgb(10, 11, 13)">${words(20)}</p>`))).toBe(true);
  });

  it("is happy with light text on dark paper", () => {
    expect(hardToRead(page(`<p style="color: rgb(255, 255, 255)">${words(20)}</p>`))).toBe(false);
  });

  it("reads dark text on a white card as fine", () => {
    const doc = page(`<table style="background-color: rgb(255, 255, 255)"><tbody><tr><td><p style="color: rgb(10, 11, 13)">${words(20)}</p></td></tr></tbody></table>`);
    expect(hardToRead(doc)).toBe(false);
  });

  it("sees through a see-through card to the paper under it", () => {
    const doc = page(`<div style="background-color: rgba(255, 255, 255, 0)"><p style="color: rgb(10, 11, 13)">${words(20)}</p></div>`);
    expect(hardToRead(doc)).toBe(true);
  });

  it("mixes a half see-through card with what is under it", () => {
    // Half white over near-black is a mid grey: black text on it still reads.
    const doc = page(`<div style="background-color: rgba(255, 255, 255, 0.5)"><p style="color: rgb(0, 0, 0)">${words(20)}</p></div>`);
    expect(hardToRead(doc)).toBe(false);
  });

  it("mixes half see-through text with the card under it", () => {
    const doc = page(`<p style="color: rgba(255, 255, 255, 0.05)">${words(20)}</p>`);
    expect(hardToRead(doc)).toBe(true);
  });

  it("ignores hidden text, such as the preview line", () => {
    const doc = page(
      `<div style="display: none"><span style="color: rgb(10, 11, 13)">${words(40)}</span></div>` +
        `<div style="visibility: hidden; color: rgb(10, 11, 13)">${words(40)}</div>` +
        `<div style="opacity: 0; color: rgb(10, 11, 13)">${words(40)}</div>` +
        `<p style="color: rgb(255, 255, 255)">${words(5)}</p>`,
    );
    expect(hardToRead(doc)).toBe(false);
  });

  it("can't judge text over a background picture, so leaves it out", () => {
    const doc = page(`<div style="background-image: url(data:image/png;base64,AA==)"><p style="color: rgb(10, 11, 13)">${words(20)}</p></div>`);
    expect(hardToRead(doc)).toBe(false);
  });

  it("puts up with a little dark small print among readable text", () => {
    const doc = page(`<p style="color: rgb(255, 255, 255)">${words(100)}</p><p style="color: rgb(10, 11, 13)">${words(5)}</p>`);
    expect(hardToRead(doc)).toBe(false);
  });

  it("counts only letters and digits, not spacers", () => {
    const doc = page(`<p style="color: rgb(10, 11, 13)">&nbsp;&zwnj;&nbsp;&zwnj; · — </p><p style="color: rgb(255, 255, 255)">ok</p>`);
    expect(hardToRead(doc)).toBe(false);
  });

  it("finds nothing wrong with an email that has no text", () => {
    expect(hardToRead(page(`<img alt="">`))).toBe(false);
  });

  it("skips text whose colour it can't read", () => {
    expect(hardToRead(page(`<p style="color: color(display-p3 0 0 0)">${words(20)}</p>`))).toBe(false);
  });

  it("can't judge text with no solid paper under it", () => {
    const doc = page(`<div style="background-color: rgba(0, 0, 0, 0.5)"><p style="color: rgb(10, 11, 13)">${words(20)}</p></div>`, "transparent");
    expect(hardToRead(doc)).toBe(false);
  });
});
