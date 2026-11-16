import { describe, expect, it } from "vitest";
import { replySubject, stripRe } from "./subject";

// The same cases as Rust's `strip_re` (moss-mcp) and `reply_subject` (moss-ai).
describe("stripRe", () => {
  it.each([
    ["Re: RE:re: Lunch", "Lunch"],
    ["Lunch", "Lunch"],
    ["Re", "Re"],
    ["Noël", "Noël"],
    ["Re: Grüße", "Grüße"],
    ["  re:   Lunch  ", "Lunch"],
  ])("%j gives %j", (subject, want) => {
    expect(stripRe(subject)).toBe(want);
  });

  it("drops a leading Fw: or Fwd: too, mixed in with Re:", () => {
    expect(stripRe("Fwd: Re: FW: Lunch")).toBe("Lunch");
  });

  it("keeps Re: that is not at the start", () => {
    expect(stripRe("Lunch Re: Friday")).toBe("Lunch Re: Friday");
  });
});

describe("replySubject", () => {
  it.each([
    ["Re: RE:re: Lunch", "Re: Lunch"],
    ["Lunch", "Re: Lunch"],
    ["Re", "Re: Re"],
    ["Re: Grüße", "Re: Grüße"],
  ])("%j gives %j", (subject, want) => {
    expect(replySubject(subject)).toBe(want);
  });
});
