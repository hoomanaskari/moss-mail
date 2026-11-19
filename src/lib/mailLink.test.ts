import { describe, expect, it } from "vitest";
import { parseMailLink } from "./mailLink";

const emails = (url: string, field: "to" | "cc" | "bcc" = "to") => parseMailLink(url)![field].map((p) => p.email);

describe("parseMailLink", () => {
  it("reads one address", () => {
    expect(parseMailLink("mailto:lena@northwind.co")).toEqual({
      to: [{ name: "lena@northwind.co", email: "lena@northwind.co", initials: "L" }],
      cc: [],
      bcc: [],
      subject: "",
      body: "",
    });
  });

  it("reads every field a website can fill in", () => {
    const link = parseMailLink(
      "mailto:lena@northwind.co?cc=jonas@northwind.co&bcc=ines@northwind.co&subject=Q3%20plan&body=Hi%20Lena%2C%0D%0A%0D%0AShall%20we%20meet%3F",
    )!;
    expect(link.to.map((p) => p.email)).toEqual(["lena@northwind.co"]);
    expect(link.cc.map((p) => p.email)).toEqual(["jonas@northwind.co"]);
    expect(link.bcc.map((p) => p.email)).toEqual(["ines@northwind.co"]);
    expect(link.subject).toBe("Q3 plan");
    expect(link.body).toBe("Hi Lena,\n\nShall we meet?");
  });

  it("knows the scheme and the field names in any case", () => {
    const link = parseMailLink("MAILTO:lena@northwind.co?Subject=Hello&CC=jonas@northwind.co")!;
    expect(link.subject).toBe("Hello");
    expect(link.cc.map((p) => p.email)).toEqual(["jonas@northwind.co"]);
  });

  it("reads several addresses, in the address part, in to= and in each field", () => {
    expect(emails("mailto:lena@northwind.co,jonas@northwind.co?to=ines@northwind.co")).toEqual([
      "lena@northwind.co",
      "jonas@northwind.co",
      "ines@northwind.co",
    ]);
    expect(emails("mailto:?cc=a@x.io%2C%20b@x.io&cc=c@x.io", "cc")).toEqual(["a@x.io", "b@x.io", "c@x.io"]);
  });

  it("keeps a name given with the address, even one with a comma in quotes", () => {
    expect(parseMailLink('mailto:%22Vogt%2C%20Lena%22%20%3Clena@northwind.co%3E,jonas@northwind.co')!.to).toEqual([
      { name: "Vogt, Lena", email: "lena@northwind.co", initials: "VL" },
      { name: "jonas@northwind.co", email: "jonas@northwind.co", initials: "J" },
    ]);
  });

  it("lists an address once, whatever its case", () => {
    expect(emails("mailto:lena@northwind.co?to=Lena@Northwind.co")).toEqual(["lena@northwind.co"]);
  });

  it("drops what isn't an address", () => {
    expect(emails("mailto:not-an-address,,lena@northwind.co?cc=%20", "to")).toEqual(["lena@northwind.co"]);
    expect(emails("mailto:lena@northwind.co?cc=%20", "cc")).toEqual([]);
  });

  it("opens an empty email from a link with no address", () => {
    expect(parseMailLink("mailto:")).toEqual({ to: [], cc: [], bcc: [], subject: "", body: "" });
    expect(parseMailLink("mailto:?subject=Feedback")!.subject).toBe("Feedback");
  });

  it("keeps a plus and a hash as they are written", () => {
    const link = parseMailLink("mailto:lena+orders@northwind.co?subject=Order #12+13")!;
    expect(link.to[0].email).toBe("lena+orders@northwind.co");
    expect(link.subject).toBe("Order #12+13");
  });

  it("keeps text a website encoded badly instead of failing", () => {
    expect(parseMailLink("mailto:lena@northwind.co?subject=100%%20sure")!.subject).toBe("100%%20sure");
  });

  it("joins several bodies and takes the first subject", () => {
    const link = parseMailLink("mailto:?subject=One&subject=Two&body=Line%201&body=Line%202")!;
    expect(link.subject).toBe("One");
    expect(link.body).toBe("Line 1\nLine 2");
  });

  it("ignores fields Moss doesn't fill in and parts with no value", () => {
    const link = parseMailLink("mailto:lena@northwind.co?in-reply-to=%3Cabc@x%3E&subject&keywords=x&body=Hi")!;
    expect(link).toEqual({ ...parseMailLink("mailto:lena@northwind.co")!, body: "Hi" });
  });

  it("turns old Mac line ends into new lines", () => {
    expect(parseMailLink("mailto:?body=a%0Db%0Ac")!.body).toBe("a\nb\nc");
  });

  it("is nothing for a link that isn't a mail link", () => {
    expect(parseMailLink("https://northwind.co")).toBeUndefined();
    expect(parseMailLink("lena@northwind.co")).toBeUndefined();
  });
});
