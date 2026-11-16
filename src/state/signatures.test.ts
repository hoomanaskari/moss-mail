import { beforeEach, describe, expect, it, vi } from "vitest";
import picks from "../../crates/moss-core/tests/signature-picks.json?raw";
import { fakeMail } from "../test/fakeMail";
import { lena, maya, message, personal, thread, work } from "../test/fixtures";
import {
  linkFor,
  offered,
  pickFor,
  pickReason,
  pickSignature,
  preview,
  signaturesSaved,
  signedBefore,
  useSignatures,
  usedBy,
  type PickSettings,
  type SignatureEmail,
  type SignaturePick,
} from "./signatures";

const s = () => useSignatures.getState();
/** A first email or reply, from `address` or the account's own. */
const email = (accountId: string, kind: "new" | "reply", address?: string): SignatureEmail => ({ accountId, kind, address, signedBefore: 0 });
const pristine = useSignatures.getState();
beforeEach(() => useSignatures.setState(pristine, true));

describe("signatures", () => {
  it("previews the name and the first part of the next line", () => {
    expect(preview(s().signatures.find((x) => x.id === "studio")!)).toBe("Maya Ross · Ross Studio");
    expect(preview(s().signatures.find((x) => x.id === "work-short")!)).toBe("Maya");
  });

  // Same cases as `link` in crates/moss-engine/src/compose.rs, which links them in sent mail.
  it("links the lines that are web addresses", () => {
    expect(linkFor("northwind.co")).toBe("https://northwind.co");
    expect(linkFor("www.northwind.co/team?x=1")).toBe("https://www.northwind.co/team?x=1");
    expect(linkFor("https://maya.design/work")).toBe("https://maya.design/work");
    expect(linkFor("HTTP://X.IO")).toBe("HTTP://X.IO");
    expect(linkFor("ross_studio-1.art")).toBe("https://ross_studio-1.art");
    for (const line of ["Maya Ross", "Design lead, Northwind", "maya@northwind.co", "northwind", "northwind.co is great", "ftp://x.io", "a..b", "x.io/a b", ".io", ""])
      expect(linkFor(line), line).toBeUndefined();
  });

  it("says which accounts use a signature, and for what", () => {
    const accounts = [work, personal, { ...personal, id: "studio", name: "Studio" }];
    expect(usedBy("work-full", accounts)).toBe("Used by Work · new emails");
    expect(usedBy("work-short", accounts)).toBe("Used by Work · replies");
    expect(usedBy("studio", accounts)).toBe("Used by Studio");
    expect(usedBy("personal", accounts)).toBe("Not used yet");
  });

  it("picks each account’s signature for new emails and replies", () => {
    expect(pickFor(email("work", "new")).id).toBe("work-full");
    expect(pickFor(email("work", "reply")).id).toBe("work-short");
    expect(pickFor(email("personal", "new")).id).toBeNull();
    expect(pickFor(email("unknown", "new")).id).toBeNull();
  });

  it("changes an account’s choice, and the AI switch", () => {
    s().setChoice("personal", "replies", "studio");
    expect(pickFor(email("personal", "reply")).id).toBe("studio");
    s().setChoice("work", "newEmails", null);
    expect(pickFor(email("work", "new")).id).toBeNull();
    s().setLetMossChoose(false);
    expect(s().letMossChoose).toBe(false);
  });
});

describe("picking the signature for an email", () => {
  // The same table `pick_signature` in crates/moss-core/src/signatures.rs is tested with.
  const table: {
    settings: PickSettings;
    cases: { name: string; letMossChoose?: boolean; email: SignatureEmail; pick: SignaturePick }[];
  } = JSON.parse(picks);

  it.each(table.cases)("$name", ({ letMossChoose, email, pick }) => {
    const settings = { ...table.settings, letMossChoose: letMossChoose ?? table.settings.letMossChoose };
    expect(pickSignature(settings, email)).toEqual(pick);
  });

  it("counts the messages you already sent in a thread", () => {
    // Your own address in any case is yours; Lena's isn't.
    const you = { ...maya, email: "Maya@Northwind.co" };
    const t = thread("t", { messages: [message({ from: lena }), message({ from: maya }), message({ from: lena }), message({ from: you })] });
    expect(signedBefore(t, [work])).toBe(2);
    expect(signedBefore(undefined, [work])).toBe(0);
  });

  it("picks from the saved signatures", () => {
    expect(pickFor({ accountId: "work", kind: "reply", signedBefore: 1 })).toEqual({ id: "work-short", why: "short" });
    useSignatures.setState({ letMossChoose: false });
    expect(pickFor({ accountId: "studio", kind: "reply", signedBefore: 2 })).toEqual({ id: "studio", why: "reply" });
  });

  it("says why, in a few words", () => {
    const at = "hi@rossstudio.art";
    expect(pickReason({ id: "studio", why: "new" }, at)).toEqual({
      reason: "Your signature for hi@rossstudio.art",
      why: "Picked for new emails from hi@rossstudio.art.",
    });
    expect(pickReason({ id: "studio", why: "reply" }, at)).toEqual({
      reason: "Your reply signature for hi@rossstudio.art",
      why: "Picked for replies from hi@rossstudio.art.",
    });
    expect(pickReason({ id: "studio", why: "forward" }, at)).toEqual({
      reason: "Your signature for hi@rossstudio.art",
      why: "A forward goes to someone new, so it gets your one for new emails.",
    });
    expect(pickReason({ id: "work-short", why: "short" }, at)).toEqual({
      reason: "Short one: you signed earlier in this thread",
      why: "You already signed in this thread, so Moss picked your one-line signature.",
    });
    expect(pickReason({ id: null, why: "quick" }, at)).toEqual({
      reason: "Left out: quick reply in a back-and-forth",
      why: "You already signed in this thread, so Moss left it out.",
    });
    expect(pickReason({ id: null, why: "noneSet" }, at)).toEqual({
      reason: "None set for hi@rossstudio.art",
      why: "No signature set for hi@rossstudio.art here. Pick one, or set one in Settings.",
    });
  });
});

describe("an address's own signature", () => {
  it("mail from an address with its own choice uses it; the rest use their account's", () => {
    useSignatures.setState({ byAddress: { "hi@rossstudio.art": { newEmails: "studio", replies: null } } });
    expect(pickFor(email("work", "new", "Hi@RossStudio.art")).id).toBe("studio");
    expect(pickFor(email("work", "reply", "hi@rossstudio.art")).id).toBeNull();
    expect(pickFor(email("work", "reply", "other@rossstudio.art")).id).toBe("work-short");
  });

  it("setting one kind keeps the account's choice for the other, and is saved", async () => {
    const { backend } = fakeMail();
    s().setAddressChoice("work", "hi@rossstudio.art", "replies", "studio");
    expect(s().byAddress["hi@rossstudio.art"]).toEqual({ newEmails: "work-full", replies: "studio" });
    await signaturesSaved();
    expect(backend.setAddressSignatures).toHaveBeenLastCalledWith("work", "hi@rossstudio.art", { newEmails: "work-full", replies: "studio" });
  });

  it("back to the account's choice for both, it follows the account again", async () => {
    const { backend } = fakeMail();
    s().setAddressChoice("work", "hi@rossstudio.art", "newEmails", null);
    s().setAddressChoice("work", "hi@rossstudio.art", "replies", "studio");
    s().setAddressChoice("work", "hi@rossstudio.art", "newEmails", "same");
    expect(s().byAddress["hi@rossstudio.art"]).toEqual({ newEmails: "work-full", replies: "studio" });
    s().setAddressChoice("work", "hi@rossstudio.art", "replies", "same");
    expect(s().byAddress).toEqual({});
    await signaturesSaved();
    expect(backend.setAddressSignatures).toHaveBeenLastCalledWith("work", "hi@rossstudio.art", null);
  });

  it("an address of an account with no choice starts from none", () => {
    s().setAddressChoice("new-account", "x@y.co", "newEmails", "studio");
    expect(s().byAddress["x@y.co"]).toEqual({ newEmails: "studio", replies: null });
  });

  it("deleting a signature takes it off the addresses that used it", () => {
    useSignatures.setState({
      byAddress: { "hi@rossstudio.art": { newEmails: "studio", replies: "work-short" }, "me@x.io": { newEmails: "work-short", replies: "studio" } },
    });
    s().deleteSignature("studio");
    expect(s().byAddress).toEqual({
      "hi@rossstudio.art": { newEmails: null, replies: "work-short" },
      "me@x.io": { newEmails: "work-short", replies: null },
    });
  });
});

describe("editing signatures", () => {
  it("renames and rewrites one", () => {
    s().updateSignature("studio", { name: "Studio (short)", lines: ["Maya"] });
    expect(s().signatures.find((x) => x.id === "studio")).toMatchObject({ name: "Studio (short)", lines: ["Maya"] });
  });

  it("adds a new one with a fresh id, and numbers its name", () => {
    const a = s().addSignature("Maya Ross");
    const b = s().addSignature("Maya Ross");
    expect(a).not.toBe(b);
    expect(s().signatures.slice(-2).map((x) => x.name)).toEqual(["New signature", "New signature 2"]);
    expect(s().signatures.at(-1)!.lines).toEqual(["Maya Ross"]);
  });

  it("sets a choice for an account that has none yet", () => {
    s().setChoice("new", "replies", "studio");
    expect(s().byAccount.new).toEqual({ newEmails: null, replies: "studio" });
  });

  it("deletes one and stops accounts using it", () => {
    s().deleteSignature("work-short");
    expect(s().byAccount.work).toEqual({ newEmails: "work-full", replies: null });
    s().deleteSignature("work-full");
    expect(s().signatures.some((x) => x.id === "work-full")).toBe(false);
    expect(s().byAccount.work).toEqual({ newEmails: null, replies: null });
  });
});

describe("recently used", () => {
  it("moves a signature to the front when it is used", () => {
    expect(s().recent[0]).toBe("work-short");
    s().markUsed("studio");
    expect(s().recent).toEqual(["studio", "work-short", "work-full", "personal"]);
  });
});

describe("the picker's offer", () => {
  it("puts Moss's pick first, then recently used ones, then any saved one not used yet", () => {
    useSignatures.setState({ recent: ["work-full", "gone"] });
    expect(offered("studio").map((x) => x.id)).toEqual(["studio", "work-full", "work-short", "personal"]);
    expect(offered(null).map((x) => x.id)).toEqual(["work-full", "work-short", "studio", "personal"]);
  });

  it("is empty with no saved signatures", () => {
    useSignatures.setState({ signatures: [], recent: [] });
    expect(offered(null)).toEqual([]);
  });
});

describe("saving", () => {
  it("saves each account’s choice and the AI switch", async () => {
    const { backend } = fakeMail();
    s().setChoice("personal", "replies", "studio");
    await signaturesSaved();
    expect(backend.setSignatureChoice).toHaveBeenCalledWith("personal", "replies", "studio");
    s().setChoice("work", "newEmails", null);
    await signaturesSaved();
    expect(backend.setSignatureChoice).toHaveBeenLastCalledWith("work", "newEmails", null);
    s().setLetMossChoose(false);
    await signaturesSaved();
    expect(backend.setLetMossChoose).toHaveBeenCalledWith(false);
  });

  it("saves a signature as you edit it", async () => {
    const { backend } = fakeMail();
    s().updateSignature("studio", { name: "Studio (short)" });
    await signaturesSaved();
    expect(backend.saveSignature).toHaveBeenCalledWith({ id: "studio", name: "Studio (short)", lines: s().signatures.find((x) => x.id === "studio")!.lines });
    s().updateSignature("gone", { name: "Nothing" });
    await signaturesSaved();
    expect(backend.saveSignature).toHaveBeenCalledTimes(1);
  });

  it("saves a new one, and offers it last in the composer until it is used", async () => {
    const { backend } = fakeMail();
    const id = s().addSignature("Sam Lee");
    await signaturesSaved();
    expect(backend.saveSignature).toHaveBeenCalledWith({ id, name: "New signature", lines: ["Sam Lee"] });
    expect(s().recent.at(-1)).toBe(id);
  });

  it("never reuses the id of a signature saved before a restart", () => {
    fakeMail();
    const saved = Array.from({ length: 50 }, (_, i) => ({ id: `signature-${i + 1}`, name: "Saved", lines: [] }));
    useSignatures.setState({ signatures: saved });
    const id = s().addSignature("Sam Lee");
    expect(saved.map((x) => x.id)).not.toContain(id);
  });

  it("deletes a signature for good, and from the recently used", async () => {
    const { backend } = fakeMail();
    s().deleteSignature("work-short");
    await signaturesSaved();
    expect(backend.deleteSignature).toHaveBeenCalledWith("work-short");
    expect(s().recent).not.toContain("work-short");
  });

  it("remembers which one you used last", async () => {
    const { backend } = fakeMail();
    s().markUsed("studio");
    await signaturesSaved();
    expect(backend.markSignatureUsed).toHaveBeenCalledWith("studio");
  });

  it("saves changes in the order you made them, even when one is slow", async () => {
    let finish = () => {};
    const { backend } = fakeMail({ saveSignature: vi.fn(() => new Promise<void>((resolve) => (finish = resolve))) });
    const id = s().addSignature("Sam Lee");
    s().setChoice("work", "newEmails", id);
    await new Promise((r) => setTimeout(r));
    expect(backend.setSignatureChoice).not.toHaveBeenCalled();
    finish();
    await signaturesSaved();
    expect(backend.setSignatureChoice).toHaveBeenCalledWith("work", "newEmails", id);
  });

  it("keeps saving after one save fails", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const { backend } = fakeMail({ deleteSignature: vi.fn(async () => Promise.reject(new Error("disk full"))) });
    s().deleteSignature("studio");
    // Waiting on a failed save still resolves.
    await signaturesSaved();
    s().markUsed("work-full");
    await signaturesSaved();
    expect(backend.markSignatureUsed).toHaveBeenCalledWith("work-full");
    expect(warn).toHaveBeenCalledWith("Moss:", new Error("disk full"));
    warn.mockRestore();
  });
});
