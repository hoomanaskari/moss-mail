import { describe, expect, it } from "vitest";
import {
  checkRequest,
  demoContent,
  demoSummaries,
  demoView,
  MAX_INSTRUCTION,
  ruleFor,
  ruleSender,
  setSummaries,
  summaries,
  summaryKinds,
  type SummaryContent,
  type SummaryRule,
} from "./summaries";

const rule = (id: number, sender: string, kind: SummaryRule["kind"] = "short"): SummaryRule => ({ id, sender, kind, createdAt: 0 });

describe("requests, like SummaryRequest::new", () => {
  it("custom needs an instruction and other kinds drop theirs", () => {
    expect(checkRequest({ kind: "facts", instruction: "ignored" })).toEqual({ kind: "facts" });
    expect(checkRequest({ kind: "custom", instruction: "  Only the prices " })).toEqual({ kind: "custom", instruction: "Only the prices" });
    expect(() => checkRequest({ kind: "custom", instruction: "  " })).toThrow(expect.objectContaining({ kind: "invalid", message: "say how to summarise it" }));
    expect(() => checkRequest({ kind: "custom" })).toThrow(expect.objectContaining({ kind: "invalid" }));
    expect(() => checkRequest({ kind: "custom", instruction: "é".repeat(MAX_INSTRUCTION + 1) })).toThrow(
      expect.objectContaining({ message: "keep the instruction under 500 characters" }),
    );
    expect(checkRequest({ kind: "custom", instruction: "é".repeat(MAX_INSTRUCTION) }).instruction).toHaveLength(MAX_INSTRUCTION);
  });
});

describe("rule senders, like summary_rules::sender", () => {
  it("are addresses or whole domains", () => {
    expect(ruleSender("  Dana@Acme.co ")).toBe("dana@acme.co");
    expect(ruleSender("mailto:dana@acme.co")).toBe("dana@acme.co");
    expect(ruleSender("@Acme.co")).toBe("acme.co");
    expect(ruleSender("mail.acme.co")).toBe("mail.acme.co");
    for (const bad of ["", "acme", "dana@acme", "@", "dana smith@acme.co", "acme.co.", ".acme.co", "ac me.co", "a/b.co", "@acme.co@"]) {
      expect(() => ruleSender(bad), bad).toThrow(expect.objectContaining({ kind: "invalid" }));
    }
    expect(() => ruleSender("gmail.com")).toThrow(expect.objectContaining({ message: "gmail.com is a public mail service: make the rule for one address there" }));
    expect(ruleSender("sam@gmail.com")).toBe("sam@gmail.com");
    expect(() => ruleSender("acme")).toThrow(expect.objectContaining({ message: "“acme” isn’t an email address or a domain" }));
  });

  it("the most specific rule applies, like summary_rule_for", () => {
    const rules = [rule(1, "acme.co"), rule(2, "notacme.co")];
    expect(ruleFor(rules, "Dana@mail.Acme.co")?.sender).toBe("acme.co");
    rules.push(rule(3, "mail.acme.co"));
    expect(ruleFor(rules, "dana@mail.acme.co")?.sender).toBe("mail.acme.co");
    rules.push(rule(4, "dana@mail.acme.co"));
    expect(ruleFor(rules, "dana@mail.acme.co")?.sender).toBe("dana@mail.acme.co");
    expect(ruleFor(rules, "x.dana@mail.acme.co")?.sender).toBe("mail.acme.co");
    expect(ruleFor(rules, "sam@other.co")).toBeUndefined();
  });
});

describe("the demo", () => {
  it("keeps one summary per email: the kind asked for last replaces it", async () => {
    const demo = demoSummaries();
    expect(await demo.kept("q4-budget")).toEqual({});
    for (const kind of summaryKinds) {
      const request = kind === "custom" ? { kind, instruction: "As a list" } : { kind };
      const view = await demo.summarize("q4-budget", request);
      expect(view).toMatchObject({ ...request, content: demoContent[kind], why: "asked", stale: false });
      expect((await demo.kept("q4-budget")).summary).toBe(view);
    }
    const short = await demo.summarize("q4-budget", { kind: "short" });
    expect(await demo.summarize("q4-budget", { kind: "short" })).toBe(short);
    const again = await demo.summarize("q4-budget", { kind: "short" }, true);
    expect(again).not.toBe(short);
    expect((await demo.kept("q4-budget")).summary).toBe(again);
    await expect(demo.summarize("q4-budget", { kind: "custom" })).rejects.toMatchObject({ kind: "invalid" });
  });

  it("streams each kind in, a part first, then gives the whole", async () => {
    const demo = demoSummaries();
    for (const kind of summaryKinds) {
      const request = kind === "custom" ? { kind, instruction: "As a list" } : { kind };
      const heard: SummaryContent[] = [];
      await demo.summarize("q4-budget", request, true, (c) => heard.push(c));
      const whole = demoContent[kind];
      expect(heard).toHaveLength(2);
      expect(heard[1]).toEqual(whole);
      const part = heard[0];
      if (part.format === "text" && whole.format === "text") expect(whole.text.startsWith(part.text) && part.text.length < whole.text.length).toBe(true);
      else expect(part).toEqual({ ...whole, items: whole.format === "text" ? [] : whole.items.slice(0, 1) });
    }
    // One kept and fresh isn't written again, so nothing streams.
    const heard: SummaryContent[] = [];
    await demo.summarize("q4-budget", { kind: "custom", instruction: "As a list" }, false, (c) => heard.push(c));
    expect(heard).toEqual([]);
  });

  it("can stay part-way through writing", async () => {
    const heard: SummaryContent[] = [];
    const writing = demoSummaries({ state: "writing" }).summarize("q4-budget", { kind: "actions" }, false, (c) => heard.push(c));
    const raced = await Promise.race([writing, new Promise((r) => setTimeout(() => r("still writing"), 10))]);
    expect(raced).toBe("still writing");
    expect(heard).toEqual([{ format: "actions", items: [demoContent.actions.format === "actions" && demoContent.actions.items[0]] }]);
  });

  it("discards an email's summary", async () => {
    const demo = demoSummaries({ kept: { "q4-budget": demoView({ kind: "facts" }) } });
    await demo.discard("q4-budget");
    expect(await demo.kept("q4-budget")).toEqual({});
    await demo.discard("nothing-kept");
    expect(await demo.kept("nothing-kept")).toEqual({});
  });

  it("can start with a stale summary kept, which asks to be written again", async () => {
    const stale = demoView({ kind: "actions" }, { stale: true, why: "rule" });
    const demo = demoSummaries({ kept: { "q4-budget": stale } });
    expect((await demo.kept("q4-budget")).summary).toEqual(stale);
    expect((await demo.summarize("q4-budget", { kind: "actions" })).stale).toBe(false);
  });

  it("shows the waiting, missing-model and failed states", async () => {
    const loading = demoSummaries({ state: "loading" }).summarize("q4-budget", { kind: "short" });
    const raced = await Promise.race([loading, new Promise((r) => setTimeout(() => r("still waiting"), 10))]);
    expect(raced).toBe("still waiting");
    await expect(demoSummaries({ state: "notReady" }).summarize("q4-budget", { kind: "facts" })).rejects.toEqual({
      kind: "notReady",
      message: "the AI model isn't downloaded yet",
    });
    await expect(demoSummaries({ state: "failed" }).summarize("q4-budget", { kind: "facts" })).rejects.toMatchObject({ kind: "model" });
  });

  it("keeps rules, and a summary in its rule's kind says the rule wrote it", async () => {
    const demo = demoSummaries();
    const facts = await demo.saveRule({ sender: "Northwind.co", kind: "facts" });
    expect(facts).toMatchObject({ id: 1, sender: "northwind.co", kind: "facts" });
    expect(facts).not.toHaveProperty("name");
    const daniel = await demo.saveRule({ sender: "daniel@northwind.co", kind: "short" });
    expect(daniel.name).toBe("Daniel Okafor");
    await demo.removeRule(daniel.id);
    expect((await demo.kept("q4-budget")).rule).toEqual(facts);
    expect((await demo.summarize("q4-budget", { kind: "facts" })).why).toBe("rule");
    expect((await demo.summarize("q4-budget", { kind: "short" })).why).toBe("asked");
    // The same sender again changes its rule; by id, it can move to another sender.
    const actions = await demo.saveRule({ sender: "northwind.co", kind: "actions" });
    expect(actions).toMatchObject({ id: facts.id, kind: "actions", createdAt: facts.createdAt });
    const dana = await demo.saveRule({ sender: "dana@acme.co", kind: "custom", instruction: "Only prices" });
    expect(await demo.rules()).toEqual([dana, actions]);
    expect(demoContent.detailed).toMatchObject({ format: "text" });
    const moved = await demo.saveRule({ id: dana.id, sender: "northwind.co", kind: "short" });
    expect(await demo.rules()).toEqual([moved]);
    await expect(demo.saveRule({ id: 99, sender: "x.co", kind: "short" })).rejects.toMatchObject({ kind: "store" });
    await expect(demo.saveRule({ sender: "gmail.com", kind: "short" })).rejects.toMatchObject({ kind: "invalid" });
    expect(await demo.removeRule(moved.id)).toBe(true);
    expect(await demo.removeRule(moved.id)).toBe(false);
    expect((await demo.kept("nowhere")).rule).toBeUndefined();
  });

  it("starts with rules and the automatic setting as given", async () => {
    const demo = demoSummaries({ rules: [rule(4, "acme.co", "facts")], automatic: false, senderOf: () => "dana@acme.co" });
    expect((await demo.kept("any")).rule?.id).toBe(4);
    expect((await demo.saveRule({ sender: "b.co", kind: "short" })).id).toBe(5);
    expect(await demo.automatic()).toBe(false);
    await demo.setAutomatic(true);
    expect(await demo.automatic()).toBe(true);
    expect(await demoSummaries().automatic()).toBe(true);
    const stop = demo.onWritten(() => {});
    expect(stop()).toBeUndefined();
  });

  it("is the backend until another is set", () => {
    const other = demoSummaries();
    setSummaries(other);
    expect(summaries()).toBe(other);
  });
});
