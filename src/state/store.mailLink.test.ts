/** A mail link from a website or another app opens a new email in Moss, filled in from the link. */
import { beforeEach, expect, it } from "vitest";
import { resetStore, thread, work } from "../test/fixtures";
import { useStore } from "./store";

const s = () => useStore.getState();

beforeEach(() => resetStore([thread("t1")]));

it("opens a new email filled in from the link", () => {
  s().composeFromLink("mailto:lena@northwind.co?subject=Q3%20plan&body=Hi%20Lena%2C%0A%0AShall%20we%20meet%3F");
  expect(s().view).toBe("compose");
  const draft = s().draft!;
  expect(draft.mode).toBe("new");
  expect(draft.fromAccountId).toBe(work.id);
  expect(draft.to.map((p) => p.email)).toEqual(["lena@northwind.co"]);
  expect(draft.subject).toBe("Q3 plan");
  expect(draft.body).toBe("Hi Lena,\n\nShall we meet?");
  expect(draft.bodyHtml).toBe("<p>Hi Lena,</p><p>Shall we meet?</p>");
  expect([draft.showCc, draft.showBcc]).toEqual([false, false]);
});

it("shows Cc and Bcc when the link fills them in", () => {
  s().composeFromLink("mailto:?cc=jonas@northwind.co&bcc=ines@northwind.co");
  const draft = s().draft!;
  expect(draft.cc.map((p) => p.email)).toEqual(["jonas@northwind.co"]);
  expect(draft.bcc.map((p) => p.email)).toEqual(["ines@northwind.co"]);
  expect([draft.showCc, draft.showBcc]).toEqual([true, true]);
});

it("leaves the body empty when the link has none", () => {
  s().composeFromLink("mailto:lena@northwind.co");
  expect([s().draft!.body, s().draft!.bodyHtml]).toEqual(["", ""]);
});

it("does nothing for a link that isn't a mail link", () => {
  s().composeFromLink("https://northwind.co");
  expect(s().view).not.toBe("compose");
  expect(s().draft).toBeUndefined();
});

it("does nothing before there is an account to send from", () => {
  resetStore([], []);
  s().composeFromLink("mailto:lena@northwind.co");
  expect(s().draft).toBeUndefined();
});
