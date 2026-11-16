import { describe, expect, it } from "vitest";
import type { Account, Person } from "../data/types";
import { isExternal, personFromAddress, rankByPicks, recipientSummary } from "./people";
import { SEP } from "./text";

const person = (name: string, email: string): Person => ({ name, email, initials: name[0] });
const work: Account = {
  id: "work",
  name: "Work",
  letter: "W",
  address: "maya@northwind.co",
  senderName: "Maya Ross",
  color: "work",
  orgDomain: "northwind.co",
};

describe("isExternal", () => {
  const ines = person("Ines Duarte", "ines@duarte-studio.pt");
  const lena = person("Lena Vogt", "Lena@Northwind.co");

  it("flags people outside the organisation", () => {
    expect(isExternal(ines, work)).toBe(true);
    expect(isExternal(lena, work)).toBe(false);
  });

  it("has no outsiders when the account has no organisation", () => {
    expect(isExternal(ines, { ...work, orgDomain: undefined })).toBe(false);
  });

  // Same rule as Person::is_external in moss-core.
  it("ignores case in the organisation domain too", () => {
    expect(isExternal(lena, { ...work, orgDomain: "Northwind.CO" })).toBe(false);
  });

  it("does not treat a look-alike domain as inside", () => {
    expect(isExternal(person("Eve", "eve@notnorthwind.co"), work)).toBe(true);
  });
});

describe("recipientSummary", () => {
  const maya = person("Maya Ross", "maya@northwind.co");
  const lena = { ...person("Lena Vogt", "lena@northwind.co"), short: "Lena" };
  const jonas = person("Jonas", "jonas@northwind.co");

  it("calls your own address “you” and uses short names", () => {
    expect(recipientSummary([maya, lena], [], maya.email)).toBe("to you, Lena");
  });

  it("adds a cc part only when there is someone on cc", () => {
    expect(recipientSummary([lena], [jonas], maya.email)).toBe(`to Lena${SEP}cc Jonas`);
  });
});

describe("personFromAddress", () => {
  it("turns a full address into a person named by the address", () => {
    expect(personFromAddress("sam@harbour.studio")).toEqual({ name: "sam@harbour.studio", email: "sam@harbour.studio", initials: "S" });
  });

  it("trims spaces around it", () => {
    expect(personFromAddress("  lena@northwind.co ")?.email).toBe("lena@northwind.co");
  });

  it("reads a pasted “Name <address>”", () => {
    expect(personFromAddress("Lena Vogt <lena@northwind.co>")).toEqual({ name: "Lena Vogt", email: "lena@northwind.co", initials: "LV" });
    expect(personFromAddress("\"Vogt, Lena\" <lena@northwind.co>")?.name).toBe("Vogt, Lena");
    expect(personFromAddress("<lena@northwind.co>")?.name).toBe("lena@northwind.co");
  });

  it("is nothing until the address is complete", () => {
    for (const text of ["", "sam", "sam@", "sam@harbour", "sam@harbour.", "@harbour.studio", "a b@x.io", "a@b@x.io", "Lena <lena@northwind>"]) {
      expect(personFromAddress(text), text).toBeUndefined();
    }
  });
});

describe("rankByPicks", () => {
  const kindle = person("ross.maya@kindle.example", "ross.maya@kindle.example");
  const home = person("Maya Ross", "RossMaya@home.example");
  const other = person("Maya Ross", "maya@studio.example");
  const all = [kindle, home, other];
  const pick = (typed: string, email: string) => ({ typed, email });

  it("keeps the order when you never chose anyone", () => {
    expect(rankByPicks(all, "ross", [])).toEqual(all);
  });

  it("puts first whoever you chose for the same text, whatever the case of the address", () => {
    expect(rankByPicks(all, " Ross ", [pick("ross", "rossmaya@home.example")])).toEqual([home, kindle, other]);
  });

  it("counts text that starts the same as similar, either way round", () => {
    const picks = [pick("ross", "rossmaya@home.example")];
    expect(rankByPicks(all, "ro", picks)[0]).toBe(home);
    expect(rankByPicks(all, "ross.m", picks)[0]).toBe(home);
  });

  it("leaves out choices made for other text", () => {
    expect(rankByPicks(all, "ross", [pick("maya", "maya@studio.example")])).toEqual(all);
  });

  it("puts the newest choice first when several fit", () => {
    const picks = [pick("ross", "maya@studio.example"), pick("ro", "rossmaya@home.example")];
    expect(rankByPicks(all, "ross", picks)).toEqual([other, home, kindle]);
  });
});
