import {
  AlarmClock,
  ArrowRightLeft,
  AtSign,
  Ban,
  BellOff,
  CalendarX,
  Info,
  Reply,
  Briefcase,
  CheckCheck,
  FilePen,
  Inbox,
  Keyboard,
  Leaf,
  Mail,
  MailOpen,
  Moon,
  Monitor,
  PenLine,
  Plug,
  Plus,
  RotateCcw,
  Search,
  Settings,
  Signature,
  Sprout,
  Star,
  Sun,
  Trash2,
  FolderLock,
  UserRoundX,
  Undo2,
  UserRoundCheck,
  Clock3,
  CircleArrowUp,
  RotateCw,
} from "lucide-react";
import type { PaletteCommand, PaletteGroup } from "../design-system/organisms";
import type { Thread } from "../data/types";
import { domainOf, isPublicDomain } from "../lib/senders";
import { useAskPanel } from "./askPanel";
import { useDrafts, visibleDrafts } from "./drafts";
import { openScheduled, openSendLater, useScheduled, visibleScheduled } from "./scheduled";
import { useSearch } from "./search";
import { accountFor, findThread, useStore } from "./store";
import { inTrash, openTrash, restoreToInbox, trashCursor } from "./trash";
import { addressCounts, addressOf, sendingAddresses } from "../lib/addresses";
import { SEP } from "../lib/text";
import { dayBefore, dayBeforeDate, remindTime, weekdayOf } from "../lib/time";
import { splits } from "../data/demo";
import { whyText } from "../lib/why";
import { useUpdates } from "./updates";
import { useAppIdentity } from "../services/appIdentity";

/** The email the palette acts on: the open one, or the one under the cursor. */
export function contextThread() {
  const s = useStore.getState();
  // Deleted mail offers only Back to inbox (`trashCommands`).
  if (s.view === "zero" || s.view === "trash" || (s.view === "thread" && inTrash(s.openThreadId))) return undefined;
  const search = useSearch.getState();
  const id =
    s.view === "thread" || s.view === "compose"
      ? s.openThreadId
      : s.view === "blocked"
        ? s.blockedCursorId
        : s.view === "done"
          ? s.doneCursorId
          : s.view === "search"
            ? search.hits.find((h) => h.doc.id === search.cursorId)?.doc.threadId
            : s.cursorId;
  return findThread(s, id);
}

function words(q: string) {
  return q
    .toLowerCase()
    .split(/\W+/)
    .filter((w) => w.length > 2);
}

/** In Trash, or reading an email there: put it back in the inbox. */
function trashCommands(): PaletteCommand[] {
  const view = useStore.getState().view;
  if ((view !== "trash" && view !== "thread") || !inTrash(trashCursor())) return [];
  return [{ id: "restore", label: "Back to inbox", icon: Inbox, shortcut: "E", run: () => restoreToInbox() }];
}

/** Keep commands that share a word with the query. With no query, or `fallback` and no match, show everything. */
function filter(items: PaletteCommand[], q: string, fallback = true) {
  const ws = words(q);
  if (!ws.length) return items;
  const hits = items.filter((c) => ws.some((w) => c.label.toLowerCase().includes(w)));
  return hits.length || !fallback ? hits : items;
}

/** Opens the Ask panel and asks it `question`, opening `thread` first so the question is about it. */
function askMoss(question: string, thread?: Thread) {
  if (thread) useStore.getState().open(thread.id);
  useAskPanel.getState().show();
  void useAskPanel.getState().ask(question);
}

/**
 * Ask AI: what you typed, asked in the Ask panel (↵), about the email you're reading or any
 * other. From a list, ⇧↵ opens the email under the cursor and asks about it. Then any suggestion
 * the words call for.
 */
function aiSuggestions(q: string): PaletteCommand[] {
  const t = contextThread();
  const s = useStore.getState();
  const question = q.trim();
  const out: PaletteCommand[] = [
    { id: "ask-moss", label: `Ask Moss: “${question}”`, sub: "About the email you’re reading, or any other", icon: Leaf, shortcut: "↵", ai: true, run: () => askMoss(question) },
    ...(t && s.view !== "thread"
      ? [{ id: "ask-email", label: `Ask about ${t.subject}`, sub: "Opens it, then asks Moss", icon: Mail, shortcut: "⇧↵", run: () => askMoss(question, t) }]
      : []),
  ];
  const now = Date.now();
  // The real day Moss found, or the weekday a demo deadline names.
  const deadline = t?.dueDate ? dayBeforeDate(t.dueDate, now) : t?.askDue ? dayBefore(t.askDue, now) : undefined;
  const before = /remind|deadline|before/i.test(q) ? deadline : undefined;
  if (t && before) {
    out.push({
      id: "ai-remind",
      label: `Remind me ${remindTime(before, now)}`,
      sub: `One day before ${t.from.short ?? t.from.name}’s ${weekdayOf(before + 86_400_000)} deadline`,
      icon: AlarmClock,
      ai: true,
      run: () => s.remindLater(t.id, before),
    });
  }
  return out;
}

/**
 * Block, Accept or Unblock the sender of the email in context. A new sender's come first, with
 * the banner's keys; anyone else you know can be blocked too, once you type "block".
 */
function senderCommands(q: string): PaletteCommand[] {
  const s = useStore.getState();
  const t = contextThread();
  if (!t) return [];
  const name = t.from.name;
  if (s.blockedThreads.some((b) => b.id === t.id))
    return [{ id: "unblock", label: `Unblock ${name}`, icon: Undo2, shortcut: "U", run: () => s.unblockSender(t.id) }];
  const isNew = t.sender === "new";
  if (!isNew && !words(q).length) return [];
  const domain = domainOf(t.from.email);
  const block: PaletteCommand[] = [
    { id: "block", label: `Block ${name}`, icon: Ban, shortcut: isNew ? "B" : undefined, run: () => s.blockSender(false, t.id) },
    ...(domain && !isPublicDomain(domain)
      ? [{ id: "block-domain", label: `Block everyone at ${domain}`, icon: Ban, shortcut: isNew ? "⇧B" : undefined, run: () => s.blockSender(true, t.id) }]
      : []),
  ];
  if (!isNew) return filter(block, q, false);
  return [
    ...block,
    { id: "accept", label: `Accept ${name}`, icon: UserRoundCheck, shortcut: "A", run: () => s.acceptSender(false, t.id) },
    ...(domain && !isPublicDomain(domain)
      ? [{ id: "accept-domain", label: `Accept everyone at ${domain}`, icon: UserRoundCheck, shortcut: "⇧A", run: () => s.acceptSender(true, t.id) }]
      : []),
  ];
}

/**
 * The address the email in context came to, when it isn't one you send from: send from it or
 * block its mail, once you type. Mail blocked for its address offers unblocking it.
 */
function addressCommands(q: string): PaletteCommand[] {
  const s = useStore.getState();
  const t = contextThread();
  const to = t?.toAddress;
  if (!t || !to) return [];
  const account = accountFor(t.accountId);
  if (addressOf(account, to)?.blocked)
    return [{ id: "address-unblock", label: `Unblock mail to ${to}`, icon: Undo2, run: () => s.blockAddress(account.id, to, false) }];
  if (!words(q).length || sendingAddresses(account).some((a) => a.email === to)) return [];
  return filter(
    [
      { id: "address-add", label: `Send from ${to}`, icon: AtSign, run: () => s.addAddress(account.id, to) },
      { id: "address-block", label: `Block mail to ${to}`, icon: Ban, run: () => s.blockAddress(account.id, to, true) },
    ],
    q,
    false,
  );
}

/**
 * Corrections to what Moss decided about the email in context. The ones that undo a decision
 * Moss made (the AI's needs-reply call, its split, a deadline, a reminder) come without typing;
 * the other moves once you type for them.
 */
function correctionCommands(q: string): PaletteCommand[] {
  const t = contextThread();
  if (!t) return [];
  const s = useStore.getState();
  const typed = words(q).length > 0;
  const out: PaletteCommand[] = [];
  if (t.split === "important" && t.section !== "later" && (typed || t.aiSure !== undefined)) {
    const [to, label, icon] = t.section === "reply" ? (["info", "For your information", Info] as const) : (["reply", "Needs your reply", Reply] as const);
    const now = t.section === "reply" ? "Needs your reply" : "For your information";
    out.push({
      id: `move-${to}`,
      label: `Move to ${label}`,
      ...(t.aiSure !== undefined && { sub: `${now} · AI, ${t.aiSure}% sure` }),
      icon,
      run: () => s.moveToSection(to, t.id),
    });
  }
  // Undoing the AI's move to a tab is Move to Important, or Move out of Important (see moveCommands).
  if (t.dueDate) {
    const sub = t.dueWhy ? `${t.due} · ${whyText(t.dueWhy)}` : t.due;
    out.push({ id: "no-deadline", label: "No deadline", sub, icon: CalendarX, run: () => s.clearDeadline(t.id) });
  }
  if (t.waiting) out.push({ id: "no-remind", label: "Don’t remind me", icon: BellOff, run: () => s.remindIfNoReply(t.id, false) });
  return typed ? filter(out, q, false) : out;
}

/** Marks the email in context unread, or read again. Only once you type for it, so the list stays short. */
function readCommand(q: string): PaletteCommand[] {
  const t = contextThread();
  if (!t || !words(q).length) return [];
  const s = useStore.getState();
  const command = t.unread
    ? { id: "read", label: "Mark as read", icon: MailOpen, shortcut: "U", run: () => s.toggleRead(t.id) }
    : { id: "read", label: "Mark as unread", icon: Mail, shortcut: "U", run: () => s.toggleRead(t.id) };
  return filter([command], q, false);
}

/**
 * Another tab for the email in context, for good. To Important (or out of it, with M's menu) is
 * always there; each other tab, and Moss's sorting back, once you type.
 */
function moveCommands(q: string): { always: PaletteCommand[]; typed: PaletteCommand[] } {
  const t = contextThread();
  if (!t) return { always: [], typed: [] };
  const s = useStore.getState();
  // The AI moved it: say so, so this is the way back.
  const why = t.splitWhy && { sub: `Moved to ${splits.find((x) => x.id === t.split)!.label} · ${whyText(t.splitWhy)}` };
  const always: PaletteCommand[] =
    t.split === "important"
      ? [{ id: "move-out", label: "Move out of Important…", ...why, icon: ArrowRightLeft, shortcut: "M", run: () => s.openMove(t.id) }]
      : [{ id: "move-important", label: "Move to Important", ...why, icon: Star, shortcut: "M", run: () => s.moveTo("important", t.id) }];
  const others: PaletteCommand[] = [
    ...splits
      .filter((x) => x.id !== "important" && x.id !== t.split)
      .map((x) => ({ id: `move-${x.id}`, label: `Move to ${x.label}`, icon: ArrowRightLeft, run: () => s.moveTo(x.id, t.id) })),
    ...(t.movedByYou ? [{ id: "move-moss", label: "Move: let Moss sort it", icon: Sprout, run: () => s.moveTo(null, t.id) }] : []),
  ];
  return { always, typed: words(q).length ? filter(others, q, false) : [] };
}

/** Moves the email in context to Trash. Only once you ask for it, so it's never picked by accident. */
/** "Check for updates" (in the update window), the update the top bar offers, or the restart into it. None in Moss Dev. */
function updateCommand(): PaletteCommand[] {
  if (!useAppIdentity.getState().updates) return [];
  const u = useUpdates.getState();
  const { update } = u;
  if (update.state === "ready") return [{ id: "updates", label: `Restart to update to Moss ${update.version}`, icon: RotateCw, run: () => void u.restart() }];
  if (update.state === "available" || (update.state === "failed" && update.during === "download"))
    return [{ id: "updates", label: `Update to Moss ${update.version}`, icon: CircleArrowUp, run: () => void u.install() }];
  return [{ id: "updates", label: "Check for updates", icon: CircleArrowUp, run: () => void u.check({ asked: true, open: true }) }];
}

function deleteCommand(q: string): PaletteCommand[] {
  const t = contextThread();
  // Search deletes its own way (⌘⌫), so the result leaves the results too.
  if (!t || useStore.getState().view === "search" || !/delet|trash|\bbin\b|remove/i.test(q)) return [];
  const s = useStore.getState();
  return [{ id: "delete", label: "Delete (move to Trash)", icon: Trash2, shortcut: "⌫", run: () => s.deleteThread(t.id) }];
}

export function paletteGroups(q: string): PaletteGroup[] {
  const s = useStore.getState();
  const t = contextThread();

  const who = t?.from.short ?? t?.from.name;
  const moves = moveCommands(q);
  // Actions on the email in context stay visible whatever you type.
  const onEmail: PaletteCommand[] = [
    { id: "remind", label: "Remind me…", icon: AlarmClock, shortcut: "H", run: () => t && s.openRemind(t.id) },
    // Real mail knows who wrote last; demo mail offers it as designed.
    ...(who && !t?.waiting && t?.youWroteLast !== false
      ? [{ id: "remind-no-reply", label: `Remind me if ${who} doesn’t reply`, icon: RotateCcw, shortcut: "⇧H", run: () => s.remindIfNoReply(t!.id) }]
      : []),
    ...moves.always,
  ];

  const general: PaletteCommand[] = [
    ...(s.view === "compose" && s.draft ? [{ id: "send-later", label: "Send later…", icon: Clock3, shortcut: "⌘⇧↵", run: openSendLater }] : []),
    { id: "new", label: "New email", icon: PenLine, shortcut: "C", run: () => s.compose("new") },
    ...(t ? [{ id: "reply", label: "Reply", icon: Mail, shortcut: "R", run: () => s.compose("reply", t.id) }] : []),
    { id: "search", label: "Search all mail", icon: Search, shortcut: "/", run: s.openSearch },
    { id: "done", label: "Show done emails", icon: CheckCheck, shortcut: "D", run: s.openDone },
    { id: "zero", label: "Show inbox zero", icon: Sprout, shortcut: "Z", run: s.showInboxZero },
    { id: "add-account", label: "Add an email account", icon: Plus, shortcut: "⌘N", run: s.openAddAccount },
    { id: "settings", label: "Settings", icon: Settings, shortcut: "⌘,", run: () => s.openSettings() },
    { id: "signatures", label: "Edit signatures", icon: Signature, run: () => s.openSettings("signatures") },
    { id: "addresses", label: "Your addresses (send from another address)", icon: AtSign, run: () => s.openSettings("accounts") },
    { id: "agents", label: "Connect your AI agents (MCP)", icon: Plug, run: () => s.openSettings("ai") },
    { id: "shortcuts", label: "Keyboard shortcuts", icon: Keyboard, shortcut: "?", run: () => s.openSettings("keyboard") },
    ...updateCommand(),
    {
      id: "theme",
      label: s.theme === "dark" ? "Switch to light theme" : "Switch to dark theme",
      icon: s.theme === "dark" ? Sun : Moon,
      run: () => s.setTheme(s.theme === "dark" ? "light" : "dark"),
    },
    ...(s.themeChoice !== "system"
      ? [{ id: "theme-system", label: "Match the system theme", icon: Monitor, run: () => s.setTheme("system") }]
      : []),
  ];

  const drafts = visibleDrafts(useDrafts.getState().drafts, s.accountFilter).length;
  const scheduled = visibleScheduled(useScheduled.getState().emails, s.accountFilter).length;
  const goTo: PaletteCommand[] = [
    { id: "go-all", label: "All accounts", icon: Inbox, shortcut: "⌘0", run: () => s.setAccountFilter("all") },
    ...s.accounts.map((a, i) => ({
      id: `go-${a.id}`,
      label: `${a.name}${SEP}${a.address}`,
      icon: i === 0 ? Briefcase : Inbox,
      shortcut: i < 9 ? `⌘${i + 1}` : undefined,
      run: () => s.setAccountFilter(a.id),
    })),
    // Inside an account with several addresses, each one's mail on its own (⌥1–⌥9).
    ...(s.accountFilter === "all"
      ? []
      : addressCounts(s.threads, accountFor(s.accountFilter)).flatMap((a, i, row) => [
          ...(i === 0
            ? [{ id: "go-address-all", label: `All of ${accountFor(s.accountFilter).name}’s addresses`, icon: AtSign, shortcut: "⌥0", run: () => s.setAddressFilter(undefined) }]
            : []),
          { id: `go-address-${a.email}`, label: `Only ${a.email}`, icon: AtSign, shortcut: i < 9 ? `⌥${i + 1}` : undefined, run: () => s.setAddressFilter(row[i].email) },
        ])),
    { id: "go-drafts", label: drafts ? `Drafts${SEP}${drafts}` : "Drafts", icon: FilePen, run: s.openDrafts },
    { id: "go-scheduled", label: scheduled ? `Scheduled${SEP}${scheduled}` : "Scheduled", icon: Clock3, run: openScheduled },
    { id: "go-trash", label: "Trash (deleted emails)", icon: Trash2, run: openTrash },
    { id: "go-blocked", label: "Blocked mail", icon: FolderLock, shortcut: "⌘⇧B", run: s.openBlocked },
    { id: "go-senders", label: "Blocked and accepted senders", icon: UserRoundX, run: s.openSenders },
  ];

  return [
    { title: "Ask AI", items: q.trim() ? aiSuggestions(q) : [] },
    { title: "Commands", items: [...trashCommands(), ...correctionCommands(q), ...senderCommands(q), ...addressCommands(q), ...(t ? onEmail : []), ...readCommand(q), ...moves.typed, ...deleteCommand(q), ...filter(general, q, false)] },
    { title: "Go to", items: filter(goTo, q) },
  ];
}
