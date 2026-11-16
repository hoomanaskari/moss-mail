import { beforeEach, expect, it, vi } from "vitest";
import type { SenderRule } from "../data/types";
import { fakeMail } from "../test/fakeMail";
import { person, resetStore, thread } from "../test/fixtures";
import { useStore } from "./store";
import { useSenderRules } from "./senderRules";

const s = () => useSenderRules.getState();
const rule = (sender: string, blocked: boolean, accountId = "work"): SenderRule => ({
  accountId,
  sender,
  wholeDomain: !sender.includes("@"),
  blocked,
  createdAt: 1,
});
const jordan = person("Jordan Blake", "jordan@growthleads.example");
const sam = person("Sam Lee", "Sam@GrowthLeads.example");

beforeEach(() => {
  useSenderRules.setState({ rules: undefined, problem: undefined });
  resetStore([]);
});

it("loads who you blocked or accepted", async () => {
  fakeMail({ senderRules: vi.fn(async () => [rule("jordan@growthleads.example", true)]) });
  await s().load();
  expect(s().rules).toEqual([rule("jordan@growthleads.example", true)]);
});

it("forgetting a rule takes it off the list at once and tells the engine", async () => {
  const { backend } = fakeMail({ senderRules: vi.fn(async () => [rule("alex@studio-build.example.test", false)]) });
  useSenderRules.setState({ rules: [rule("accounts.google.com", false), rule("alex@studio-build.example.test", false)] });
  const done = s().forget(rule("accounts.google.com", false));
  expect(s().rules).toEqual([rule("alex@studio-build.example.test", false)]);
  await done;
  expect(backend.forgetSenderRule).toHaveBeenCalledWith("work", "accounts.google.com");
  expect(s().rules).toEqual([rule("alex@studio-build.example.test", false)]);
  expect(backend.threads).not.toHaveBeenCalled();
});

it("forgetting a block brings that mail back to the inbox at once, asking about the sender again", async () => {
  useStore.setState({
    blockedThreads: [
      thread("a", { from: jordan, sender: "blocked" }),
      thread("c", { from: sam, sender: "blocked" }),
      thread("p", { from: jordan, sender: "blocked", accountId: "personal" }),
      thread("x", { from: person("Alex", "alex@seo-boost.example"), sender: "blocked" }),
    ],
  });
  await s().forget(rule("growthleads.example", true));
  expect(useStore.getState().blockedThreads.map((t) => t.id)).toEqual(["p", "x"]);
  expect(useStore.getState().threads.map((t) => [t.id, t.sender])).toEqual([["a", "new"], ["c", "new"]]);
  await s().forget(rule("alex@seo-boost.example", true));
  expect(useStore.getState().blockedThreads.map((t) => t.id)).toEqual(["p"]);
});

it("with real mail, the inbox and Blocked list are read again from the engine", async () => {
  const { backend } = fakeMail({ threads: vi.fn(async () => [thread("a", { from: jordan, sender: "new" as const })]) });
  useStore.setState({ blockedThreads: [thread("a", { from: jordan, sender: "blocked" })] });
  await s().forget(rule("jordan@growthleads.example", true));
  expect(backend.threads).toHaveBeenCalled();
  expect(backend.blockedThreads).toHaveBeenCalled();
  expect(useStore.getState().threads.map((t) => t.id)).toEqual(["a"]);
});

it("a failure puts the rule back and says why", async () => {
  fakeMail({
    senderRules: vi.fn(async () => [rule("alex@studio-build.example.test", false)]),
    forgetSenderRule: vi.fn(() => Promise.reject({ kind: "store", message: "The disk is full." })),
  });
  useSenderRules.setState({ rules: [rule("alex@studio-build.example.test", false)] });
  await s().forget(rule("alex@studio-build.example.test", false));
  expect(s().rules).toEqual([rule("alex@studio-build.example.test", false)]);
  expect(s().problem).toBe("The disk is full.");
  await s().load();
  expect(s().problem).toBeUndefined();
});
