import { describe, expect, it } from "vitest";
import { domainOf, isPublicDomain } from "./senders";

// The same cases as `classify.rs` in moss-store.
describe("isPublicDomain", () => {
  it("knows free mail services, including per-country ones", () => {
    for (const d of ["gmail.com", "GMAIL.com", "icloud.com", "yahoo.co.uk", "hotmail.fr", "outlook.com", "proton.me"])
      expect(isPublicDomain(d), d).toBe(true);
  });

  it("treats company domains, lookalikes and subdomains as companies", () => {
    for (const d of ["growthleads.example", "northwind.co", "notgmail.com", "mail.bigcorp.com", ""])
      expect(isPublicDomain(d), d).toBe(false);
  });
});

describe("domainOf", () => {
  it("is lowercase and after the last @", () => {
    expect(domainOf("Jordan@GrowthLeads.example")).toBe("growthleads.example");
    expect(domainOf(" a@b@c.co ")).toBe("c.co");
    expect(domainOf("nobody")).toBeUndefined();
    expect(domainOf("x@")).toBeUndefined();
  });
});
