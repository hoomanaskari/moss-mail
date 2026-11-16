import { create } from "zustand";
import type { SenderRule } from "../data/types";
import { domainOf } from "../lib/senders";
import { errorText } from "../services/errorText";
import { mail } from "../services/mail";
import { loadBlocked, reloadThreads, useStore } from "./store";

interface SenderRulesState {
  /** Everyone you blocked or accepted, newest first, once loaded. */
  rules?: SenderRule[];
  /** Why the last change didn't save, as a sentence. */
  problem?: string;
  load(): Promise<void>;
  /** Forgets a rule, so Moss asks about that sender again. Mail it blocked comes back. */
  forget(rule: SenderRule): Promise<void>;
}

const same = (a: SenderRule, b: SenderRule) => a.accountId === b.accountId && a.sender === b.sender;

/** Blocked mail the rule kept out goes back to the inbox at once, from a new sender again. */
function bringBack(rule: SenderRule) {
  const caught = (email: string) => (rule.wholeDomain ? domainOf(email) === rule.sender : email.toLowerCase() === rule.sender);
  useStore.setState((s) => {
    const back = s.blockedThreads.filter((t) => t.accountId === rule.accountId && caught(t.from.email));
    return {
      blockedThreads: s.blockedThreads.filter((t) => !back.includes(t)),
      threads: [...s.threads, ...back.map((t) => ({ ...t, sender: "new" as const }))],
    };
  });
}

/** Settings → Senders, from the mail engine. */
export const useSenderRules = create<SenderRulesState>()((set, get) => ({
  load: async () => set({ rules: await mail().senderRules(), problem: undefined }),
  forget: async (rule) => {
    const before = get().rules;
    set({ rules: before?.filter((r) => !same(r, rule)) });
    try {
      await mail().forgetSenderRule(rule.accountId, rule.sender);
    } catch (error) {
      set({ problem: errorText(error), rules: before });
      return;
    }
    set({ problem: undefined });
    if (!rule.blocked) return;
    bringBack(rule);
    if (mail().live) await Promise.all([reloadThreads(), loadBlocked()]);
  },
}));
