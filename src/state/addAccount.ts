import { create } from "zustand";
import { needsICloudLogin, providers } from "../data/providers";
import { accountColorOrder, type Account, type AccountColor, type ProviderId } from "../data/types";
import { accountSetup, type ResendDomain, type Server, type ServerSettings, type SetupFailure } from "../services/accountSetup";
import { useStore } from "./store";
import { useAiSettings } from "./aiSettings";
import type { AiPlan } from "../services/ai";
import { openLink } from "../lib/links";

export type Stage = "provider" | "sign-in" | "sync" | "ai";

/** The first-run AI choices, in the order the step lists them. */
export const aiChoices: AiPlan[] = ["recommended", "essentials", "skip"];
/** "failed" is anything but a wrong password (no connection, say); `errorMessage` explains it. */
export type SetupError = "rejected" | "admin-approval" | "failed";

export interface ResendChoice extends ResendDomain {
  chosen: boolean;
  /** Receive mail for one address only, e.g. "me"; null for all addresses. */
  only: string | null;
}

export interface Connected {
  id: string;
  name: string;
  letter: string;
  address: string;
  color: AccountColor;
}

type Field = "email" | "password" | "login" | "clientId" | "clientSecret";

interface AddAccountState {
  stage: Stage;
  /** Row under the cursor on the Choose provider list. */
  cursor: number;
  provider?: ProviderId;
  /** Gmail with your own OAuth client, or Outlook with your own client ID. */
  advanced: boolean;
  /** How many setup steps are done; the next one is highlighted. */
  stepsDone: number;
  email: string;
  password: string;
  /** iCloud custom domains: the @icloud.com address to log in with. */
  login: string;
  clientId: string;
  clientSecret: string;
  busy: boolean;
  /** Outlook: the browser is open and Moss is waiting for it. */
  waiting: boolean;
  error?: SetupError;
  errorMessage?: string;
  /** The account as the mail engine saved it, in the desktop app. */
  saved?: Account;
  imap: { status: "idle" | "found" | "not-found"; servers: ServerSettings };
  /** `problem`: what's wrong with the key, when it's "bad". */
  resend: { key: string; status: "idle" | "works" | "bad"; problem?: string; domains: ResendChoice[]; cursor: number };
  connected?: Connected;
  /** On the AI step: the choice under the cursor (recommended first). */
  aiChoice: number;

  moveCursor(step: 1 | -1): void;
  /** Continue with the provider at `index`, or the one under the cursor. */
  pick(index?: number): void;
  /** Esc: leave the advanced way, then go back to the list, then close. */
  back(): void;
  toggleAdvanced(): void;
  setField(field: Field, value: string): void;
  openStep(index: number, url: string): void;
  connect(): Promise<void>;
  lookUpServers(email: string): Promise<void>;
  setServer(which: keyof ServerSettings, patch: Partial<Server>): void;
  checkResendKey(key: string): Promise<void>;
  toggleDomain(index: number): void;
  setDomainAddresses(index: number, only: string | null): void;
  moveDomainCursor(step: 1 | -1): void;
  cycleColor(step: 1 | -1): void;
  /** Close and open the new account's inbox; the first time, ask about AI on this Mac first. */
  finish(): void;
  addAnother(): void;
  moveAiChoice(step: 1 | -1): void;
  /** Gets the AI chosen (or the one under the cursor), then opens the inbox. */
  chooseAi(plan?: AiPlan): Promise<void>;
}

const blankServers = (): ServerSettings => ({
  incoming: { host: "", port: "993", security: "ssl" },
  outgoing: { host: "", port: "465", security: "ssl" },
});

const fresh = () => ({
  stage: "provider" as Stage,
  cursor: 0,
  aiChoice: 0,
  provider: undefined,
  advanced: false,
  stepsDone: 0,
  email: "",
  password: "",
  login: "",
  clientId: "",
  clientSecret: "",
  busy: false,
  waiting: false,
  error: undefined,
  errorMessage: undefined,
  saved: undefined,
  imap: { status: "idle" as const, servers: blankServers() },
  resend: { key: "", status: "idle" as const, domains: [], cursor: 0 },
  connected: undefined,
});

/** Bumped whenever the flow moves on, so a sign-in that answers late is ignored. */
let attempt = 0;

const accountNames: Record<ProviderId, string> = {
  gmail: "Gmail",
  outlook: "Outlook",
  icloud: "iCloud",
  yahoo: "Yahoo",
  fastmail: "Fastmail",
  resend: "Resend",
  imap: "",
};

/** The account a successful sign-in becomes: named after the provider, in the first colour no account uses yet. */
function makeAccount(provider: ProviderId, address: string): Connected {
  const { accounts } = useStore.getState();
  const domain = address.split("@")[1] ?? "";
  const label = domain.split(".")[0];
  const name = accountNames[provider] || label.charAt(0).toUpperCase() + label.slice(1);
  const base = name.toLowerCase();
  let id = base;
  for (let n = 2; accounts.some((a) => a.id === id); n++) id = `${base}-${n}`;
  const used = new Set(accounts.map((a) => a.color));
  const color = accountColorOrder.find((c) => !used.has(c)) ?? accountColorOrder[accounts.length % accountColorOrder.length];
  return { id, name, letter: name.charAt(0).toUpperCase(), address, color };
}

function summary(s: AddAccountState) {
  switch (s.provider) {
    case "gmail":
      return s.advanced ? "Gmail · Own OAuth client" : "Gmail · App password";
    case "outlook":
      return s.advanced ? "Microsoft 365 · Own client ID" : "Microsoft 365";
    case "resend": {
      const n = s.resend.domains.filter((d) => d.chosen).length;
      return `Resend · ${n} ${n === 1 ? "domain" : "domains"}`;
    }
    case "imap":
      return `IMAP · ${s.imap.servers.incoming.host}`;
    default:
      return `${accountNames[s.provider!]} · App password`;
  }
}

/** Adds the connected account to the account row, with the colour picked while it synced. */
function keep(s: AddAccountState) {
  const c = s.connected!;
  const store = useStore.getState();
  store.addAccount({
    id: c.id,
    name: c.name,
    letter: c.letter,
    address: c.address,
    senderName: s.saved?.senderName ?? store.accounts[0]?.senderName ?? "",
    color: c.color,
    connection: s.saved?.connection ?? { provider: s.provider!, summary: summary(s) },
  });
  if (s.saved && s.saved.color !== c.color) store.updateAccount(c.id, { color: c.color });
}

const failure = (r: SetupFailure | { ok: false; reason: "admin-approval" }) =>
  r.reason === "failed" ? { error: "failed" as const, errorMessage: r.message } : { error: r.reason, errorMessage: undefined };

export const useAddAccount = create<AddAccountState>()((set, get) => {
  const connected = (address: string, saved?: Account) =>
    set({
      busy: false,
      waiting: false,
      stage: "sync",
      saved,
      connected: saved
        ? { id: saved.id, name: saved.name, letter: saved.letter, address: saved.address, color: saved.color }
        : makeAccount(get().provider!, address),
    });

  /** Browser sign-in: Outlook, or Gmail with your own client. */
  async function signIn() {
    const s = get();
    const run = ++attempt;
    set({ busy: true, waiting: true, error: undefined });
    const result = await accountSetup().signInWithBrowser({
      provider: s.provider as "gmail" | "outlook",
      ...(s.advanced ? { clientId: s.clientId, clientSecret: s.clientSecret || undefined } : {}),
    });
    if (run !== attempt) return;
    if (result.ok) connected(result.address, result.account);
    else set({ busy: false, waiting: false, ...failure(result) });
  }

  return {
    ...fresh(),

    moveCursor: (step) => set((s) => ({ cursor: Math.min(providers.length - 1, Math.max(0, s.cursor + step)) })),

    pick: (index) => {
      const i = index ?? get().cursor;
      attempt++;
      set({ ...fresh(), cursor: i, stage: "sign-in", provider: providers[i].id });
      if (providers[i].id === "outlook") void signIn();
    },

    back: () => {
      const s = get();
      attempt++;
      if (s.stage === "sign-in" && s.advanced) {
        set({ advanced: false, stepsDone: 0, error: undefined, busy: false, waiting: false });
      } else if (s.stage === "sign-in") {
        set({ ...fresh(), cursor: s.cursor });
      } else if (useStore.getState().accounts.length) {
        set(fresh());
        useStore.getState().leave();
      }
      // With no account yet there is nothing to go back to: the flow stays open.
    },

    toggleAdvanced: () => {
      attempt++;
      set((s) => ({ advanced: !s.advanced, stepsDone: 0, error: undefined, busy: false, waiting: false }));
    },

    setField: (field, value) => set({ [field]: value, error: undefined, errorMessage: undefined }),

    openStep: (index, url) => {
      openLink(url);
      set((s) => ({ stepsDone: Math.max(s.stepsDone, index + 1) }));
    },

    connect: async () => {
      const s = get();
      if (s.busy) return;
      if (s.provider === "outlook" || (s.provider === "gmail" && s.advanced)) {
        if (s.advanced && !s.clientId) return;
        return signIn();
      }
      if (s.provider === "resend") {
        const chosen = s.resend.domains.filter((d) => d.chosen);
        if (s.resend.status !== "works" || !chosen.length) return;
        const first = chosen[0];
        const address = `${first.only ?? first.addresses[0] ?? "hello"}@${first.name}`;
        const run = ++attempt;
        set({ busy: true, error: undefined, errorMessage: undefined });
        const result = await accountSetup().connectResend({ key: s.resend.key, address });
        if (run !== attempt) return;
        if (result.ok) return connected(address, result.account);
        return set({ busy: false, ...failure(result) });
      }
      const login = s.provider === "icloud" && needsICloudLogin(s.email) ? s.login.trim() : undefined;
      if (!s.email || !s.password || login === "") return;
      const run = ++attempt;
      set({ busy: true, error: undefined, errorMessage: undefined });
      const result = await accountSetup().connectWithPassword({
        provider: s.provider!,
        email: s.email,
        password: s.password,
        login,
        servers: s.provider === "imap" ? s.imap.servers : undefined,
      });
      if (run !== attempt) return;
      if (result.ok) connected(s.email, result.account);
      else set({ busy: false, ...failure(result) });
    },

    lookUpServers: async (email) => {
      if (!/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(email)) {
        set({ imap: { status: "idle", servers: get().imap.servers } });
        return;
      }
      const found = await accountSetup().findServerSettings(email);
      set({ imap: found ? { status: "found", servers: found } : { status: "not-found", servers: blankServers() } });
    },

    setServer: (which, patch) =>
      set((s) => ({ imap: { ...s.imap, servers: { ...s.imap.servers, [which]: { ...s.imap.servers[which], ...patch } } } })),

    checkResendKey: async (key) => {
      set((s) => ({ resend: { ...s.resend, key, status: "idle" } }));
      const domains = await accountSetup().checkResendKey(key);
      set((s) => ({
        resend:
          typeof domains === "string"
            ? { ...s.resend, status: "bad", problem: domains, domains: [] }
            : { ...s.resend, status: "works", problem: undefined, domains: domains.map((d) => ({ ...d, chosen: false, only: null })) },
      }));
    },

    toggleDomain: (index) =>
      set((s) => ({
        resend: { ...s.resend, domains: s.resend.domains.map((d, i) => (i === index ? { ...d, chosen: !d.chosen } : d)) },
      })),

    setDomainAddresses: (index, only) =>
      set((s) => ({ resend: { ...s.resend, domains: s.resend.domains.map((d, i) => (i === index ? { ...d, only } : d)) } })),

    moveDomainCursor: (step) =>
      set((s) => ({
        resend: { ...s.resend, cursor: Math.min(s.resend.domains.length - 1, Math.max(0, s.resend.cursor + step)) },
      })),

    cycleColor: (step) => {
      const c = get().connected;
      if (!c) return;
      const i = accountColorOrder.indexOf(c.color);
      const color = accountColorOrder[(i + step + accountColorOrder.length) % accountColorOrder.length];
      set({ connected: { ...c, color } });
    },

    finish: () => {
      const s = get();
      if (!s.connected) return;
      if (s.stage === "sync") keep(s);
      // The first account: AI on this Mac, once. Never asked again after a choice.
      if (s.stage === "sync" && useAiSettings.getState().status?.plan === null) {
        set({ stage: "ai", aiChoice: 0 });
        return;
      }
      useStore.getState().setAccountFilter(s.connected.id);
      set(fresh());
    },

    moveAiChoice: (step) => set((s) => ({ aiChoice: Math.min(aiChoices.length - 1, Math.max(0, s.aiChoice + step)) })),

    chooseAi: async (plan) => {
      await useAiSettings.getState().choosePlan(plan ?? aiChoices[get().aiChoice]);
      get().finish();
    },

    addAnother: () => {
      keep(get());
      set(fresh());
    },
  };
});

/** Back to a fresh flow on the provider list. */
export function resetAddAccount() {
  attempt++;
  useAddAccount.setState(fresh());
}
