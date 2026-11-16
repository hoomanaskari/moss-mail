import { listenAppKeys } from "../state/appKeyboard";
import { useEffect, useRef } from "react";
import { ChevronDown, CircleAlert, Download, Repeat, ScrollText } from "lucide-react";
import type { Person } from "../data/types";
import { Icon, Kbd } from "../design-system/atoms";
import {
  kindChoices,
  SummaryAction,
  SummaryBody,
  SummaryCard,
  SummaryInstruction,
  SummaryMenu,
  SummaryNote,
  SummaryOffer,
  SummarySkeleton,
} from "../design-system/organisms";
import { domainOf, isPublicDomain } from "../lib/senders";
import { keyDigit } from "../lib/keyDigit";
import { listTime } from "../lib/time";
import { summaries, type SummaryKind, type SummaryRule, type SummaryView } from "../services/summaries";
import { useAiSettings } from "../state/aiSettings";
import { useStore } from "../state/store";
import { useSummary } from "../state/summary";
import styles from "./SummaryPanel.module.css";

const labels: Record<SummaryKind, string> = {
  short: "Short summary",
  detailed: "Detailed summary",
  actions: "Action points",
  facts: "Key facts",
  custom: "Custom summary",
};

const autoInfo = {
  title: "Why is there a summary?",
  text: "Moss writes one line by itself for conversations of 2 or more emails that are in Important, or that you’ve replied in. It runs on this Mac.",
  hint: "Turn it off in Settings → AI.",
};

/** A sender by the whole name they go by: its first word is no name for a newsletter or a company ("The Tinker Weekly"). */
export const senderName = (p: Person) => p.name.trim() || p.email;

/** The sender's domain, unless it's a public mail service: a rule for everyone there makes no sense. */
const ruleDomain = (email: string) => {
  const domain = domainOf(email);
  return domain && !isPublicDomain(domain) ? domain : undefined;
};

/** Why a summary is there, in plain words. */
export function whyText(v: SummaryView, rule: SummaryRule | undefined, from: Person, emails: number, now: number) {
  let why: string;
  if (v.why === "important") why = "Because it’s in Important";
  else if (v.why === "replied") why = "Because you replied in this conversation";
  else if (v.why === "rule")
    why = !rule
      ? "Written by a rule"
      : rule.sender.includes("@")
        ? `Always for ${rule.sender === from.email.toLowerCase() ? senderName(from) : (rule.name ?? rule.sender)}`
        : `Always for everyone at ${rule.sender}`;
  else {
    const at = listTime(v.madeAt, now);
    why = `You asked ${now - v.madeAt < 60_000 ? "just now" : /\d:\d/.test(at) ? `at ${at}` : `on ${at}`}`;
  }
  if (v.kind === "detailed" && emails > 1) why += ` · ${emails} emails`;
  if (v.stale) why += " · before the newest email";
  return why;
}

/** Summarise S in the top bar: the kind you chose last, or its menu of kinds. The arrow beside
 * it (or ⇧S) opens the menu first, to pick the kind before anything is written. */
export function SummariseButton() {
  const { menuOpen, setMenu, pick, shown, phase, summariseOrChoose } = useSummary();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: PointerEvent) => {
      if (!ref.current!.contains(e.target as Node)) setMenu(false);
    };
    document.addEventListener("pointerdown", onDown);
    return () => document.removeEventListener("pointerdown", onDown);
  }, [menuOpen, setMenu]);
  return (
    <div ref={ref} className={styles.anchor}>
      <div className={styles.split}>
        <button type="button" className={styles.button} aria-expanded={menuOpen} onClick={() => (menuOpen ? setMenu(false) : summariseOrChoose())}>
          <Icon icon={ScrollText} size={16} tone="ai" />
          Summarise
          <Kbd>S</Kbd>
        </button>
        <button
          type="button"
          className={styles.arrow}
          aria-label="Choose a summary kind"
          title="Choose a summary kind (⇧S)"
          aria-expanded={menuOpen}
          onClick={() => setMenu(!menuOpen)}
        >
          <Icon icon={ChevronDown} size={12} tone="ink-3" />
        </button>
      </div>
      {menuOpen && (
        <div className={styles.menu}>
          <SummaryMenu current={phase === "custom" ? "custom" : shown?.kind} onPick={pick} onClose={() => setMenu(false)} />
        </div>
      )}
    </div>
  );
}

function isTyping(target: EventTarget | null) {
  const el = target as HTMLElement | null;
  return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
}

/**
 * The summary's keys, caught before the app's: S writes the kind you chose last (or opens the
 * menu, as it does under a summary), ⇧S opens the menu first, 1–5 pick a kind in it; under a summary, X discards it and
 * ↵ / ⇧↵ / Esc answer the "always" offer; Esc stops one being written; ↵ tries again or
 * downloads the model; ⌘Z brings back a rule you just stopped.
 */
function useSummaryKeys(email: string) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (useStore.getState().paletteOpen) return;
      const u = useSummary.getState();
      const key = e.key.toLowerCase();
      const take = () => {
        e.preventDefault();
        e.stopPropagation();
      };
      if (isTyping(e.target)) return;
      if (e.metaKey || e.ctrlKey) {
        if (key === "z" && !e.shiftKey && u.stopped) {
          take();
          void u.undoStop();
        }
        return;
      }
      if (e.altKey) return;
      if (u.menuOpen) {
        const digit = keyDigit(e);
        if (digit && digit <= kindChoices.length) {
          take();
          u.pick(kindChoices[digit - 1].kind);
        } else if (key === "escape" || key === "s") {
          take();
          u.setMenu(false);
        }
        return;
      }
      if (key === "s") {
        take();
        if (e.shiftKey) u.setMenu(true);
        else u.summariseOrChoose();
      } else if (key === "x" && u.phase === "shown") {
        take();
        void u.discard();
      } else if (key === "escape" && u.phase === "loading") {
        take();
        u.stop();
      } else if (key === "enter" && u.phase === "failed") {
        take();
        void u.retry();
      } else if (key === "enter" && u.phase === "notReady") {
        take();
        void useAiSettings.getState().download();
      } else if (u.offer && u.phase === "shown" && (key === "enter" || key === "escape")) {
        const domain = ruleDomain(email);
        if (key === "escape") return take(), u.notNow();
        if (e.shiftKey && !domain) return;
        take();
        void u.always(e.shiftKey ? domain! : email.toLowerCase());
      }
    };
    const stopKeys = listenAppKeys(onKey, true);
    return () => stopKeys();
  }, [email]);
}

/** "Gemma 4 E4B runs on this Mac. It's about 5 GB…", once the AI's status is known. */
function modelText() {
  const status = useAiSettings.getState().status;
  const writer = status?.writers.find((w) => w.id === status.writer);
  return writer
    ? `${writer.name} runs on this Mac. It’s ${writer.size.replace(/^About/, "about")} and downloads in the background.`
    : "The writing model runs on this Mac and downloads in the background.";
}

/** The summary card under the subject: what's kept as the email opens, or what you ask for with S. */
export function SummaryPanel({ threadId, from, emails }: { threadId: string; from: Person; emails: number }) {
  const u = useSummary();
  const openSettings = useStore((s) => s.openSettings);
  useAiSettings((s) => s.status);
  useSummaryKeys(from.email);

  useEffect(() => {
    void useSummary.getState().load(threadId);
  }, [threadId]);
  useEffect(() => summaries().onWritten((id) => void useSummary.getState().written(id)), []);
  useEffect(() => {
    if (u.phase === "notReady") void useAiSettings.getState().loadStatus();
  }, [u.phase]);

  const changeKind = () => u.setMenu(true);
  switch (u.phase) {
    case "none":
      return null;
    case "loading":
      return (
        <SummaryCard label={emails > 1 ? `Summarising ${emails} emails…` : "Summarising this email…"} onStop={u.stop}>
          {u.writing ? <SummaryBody content={u.writing} /> : <SummarySkeleton />}
        </SummaryCard>
      );
    case "notReady":
      return (
        <SummaryCard icon={Download} label="Summaries need the writing model">
          <SummaryNote text={modelText()}>
            <SummaryAction keys="↵" strong onClick={() => void useAiSettings.getState().download()}>
              Download
            </SummaryAction>
            <SummaryAction keys="⌘," onClick={() => openSettings("ai")}>
              AI settings
            </SummaryAction>
          </SummaryNote>
        </SummaryCard>
      );
    case "failed":
      return (
        <SummaryCard icon={CircleAlert} label="Couldn’t summarise this one">
          <SummaryNote text="The writing model stopped before it finished. Your email is fine.">
            <SummaryAction keys="↵" strong onClick={() => void u.retry()}>
              Try again
            </SummaryAction>
            <SummaryAction keys="S" onClick={changeKind}>
              Change kind
            </SummaryAction>
          </SummaryNote>
        </SummaryCard>
      );
    case "custom":
      return (
        <SummaryCard label={labels.custom} why="Say how you want it" onChangeKind={changeKind}>
          <SummaryInstruction
            value={u.instruction}
            focus
            onChange={u.setInstruction}
            onRedo={() => void u.summarize({ kind: "custom", instruction: u.instruction })}
          />
        </SummaryCard>
      );
  }
  const shown = u.shown!;
  const auto = shown.why === "important" || shown.why === "replied";
  const domain = ruleDomain(from.email);
  return (
    <SummaryCard
      label={auto ? "Auto summary" : labels[shown.kind]}
      why={u.stopped ? "Rule stopped · ⌘Z brings it back" : whyText(shown, u.rule, from, emails, Date.now())}
      whyIcon={shown.why === "rule" && !u.stopped ? Repeat : undefined}
      info={auto ? autoInfo : undefined}
      onChangeKind={changeKind}
      onStopRule={u.rule && shown.why === "rule" ? () => void u.stopRule() : undefined}
      onDiscard={() => void u.discard()}
      footer={
        u.offer ? (
          <SummaryOffer
            email={from.email}
            name={senderName(from)}
            domain={domain}
            custom={shown.kind === "custom"}
            problem={u.problem}
            onAlways={() => void u.always(from.email.toLowerCase())}
            onDomain={() => void u.always(domain!)}
            onNotNow={u.notNow}
          />
        ) : undefined
      }
    >
      {shown.kind === "custom" && (
        <SummaryInstruction
          value={u.instruction}
          onChange={u.setInstruction}
          onRedo={() => void u.summarize({ kind: "custom", instruction: u.instruction }, true)}
        />
      )}
      <SummaryBody content={shown.content} />
    </SummaryCard>
  );
}
