import { listenAppKeys } from "./appKeyboard";
import { useEffect } from "react";
import { closeWindow } from "../lib/window";
import { addressCounts } from "../lib/addresses";
import { keyDigit } from "../lib/keyDigit";
import { mossKeys } from "./mossKeys";
import { useAskPanel } from "./askPanel";
import { threadDraft, useDrafts } from "./drafts";
import { useQuickReply } from "./quickReply";
import { openSendLater, useScheduled } from "./scheduled";
import { accountFor, findThread, openThread, useStore, type View } from "./store";

/** Screens that handle their own keys (Esc, ⌘1–⌘6, ↑/↓…). Only ⌘K and ⌘J reach past them. */
const ownKeys: View[] = ["add-account", "settings", "search", "done", "blocked", "drafts", "senders", "scheduled"];

/** Screens without the Ask panel. In the composer ⌘J is Write with Moss. */
const noAsk: View[] = ["compose", "settings", "add-account"];

function isTyping(target: EventTarget | null) {
  const el = target as HTMLElement | null;
  return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable);
}

/**
 * Global keyboard shortcuts. Single keys work only when you are not typing;
 * ⌘/Ctrl combinations and Esc work everywhere.
 */
export function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const s = useStore.getState();
      // The palette, the Remind and Move menus and the Send later picker handle their own keys.
      if (s.paletteOpen || s.remindingId || s.movingId || useScheduled.getState().picking) return;

      const mod = e.metaKey || e.ctrlKey;
      const key = e.key.toLowerCase();
      // From the number row or the numpad, whatever ⌘ or ⌥ does to e.key.
      const digit = keyDigit(e);

      // Everywhere
      if (mod && key === "k") {
        e.preventDefault();
        s.setPaletteOpen(true);
        return;
      }
      if (mod && key === "j" && !noAsk.includes(s.view)) {
        // A Quick reply you opened takes ⌘J for Write with Moss first.
        if (!ownKeys.includes(s.view) && mossKeys(e, s)) return;
        e.preventDefault();
        useAskPanel.getState().toggle();
        return;
      }
      if (ownKeys.includes(s.view)) return;
      if (mossKeys(e, s)) return;
      if (mod && key === ",") {
        e.preventDefault();
        s.openSettings();
        return;
      }
      if (mod && key === "n") {
        e.preventDefault();
        s.openAddAccount();
        return;
      }
      if (mod && digit !== undefined) {
        if (digit === 0 || s.accounts[digit - 1]) {
          e.preventDefault();
          s.setAccountFilter(digit === 0 ? "all" : s.accounts[digit - 1].id);
        }
        return;
      }
      if (mod && key === "z" && !e.shiftKey && !isTyping(e.target)) {
        e.preventDefault();
        s.undo();
        return;
      }
      if (mod && key === "w" && s.view === "zero") {
        e.preventDefault();
        closeWindow();
        return;
      }
      if (e.key === "Escape") {
        if (useAskPanel.getState().open) useAskPanel.getState().hide();
        else if (s.view === "thread" && s.recipientsOpen) s.toggleRecipients();
        else if (s.view !== "inbox") s.back();
        return;
      }
      // Blocked mail, tucked away: ⌘⇧B from anywhere but a new email, where it adds Bcc.
      if (mod && e.shiftKey && key === "b" && s.view !== "compose") {
        e.preventDefault();
        s.openBlocked();
        return;
      }
      if (s.view === "compose") {
        if (mod && e.shiftKey && e.key === "Enter") {
          e.preventDefault();
          openSendLater();
        } else if (mod && e.key === "Enter") {
          e.preventDefault();
          s.send();
        } else if (mod && e.shiftKey && key === "e") {
          e.preventDefault();
          s.updateDraft({ fieldsOpen: !s.draft?.fieldsOpen });
        } else if (mod && e.shiftKey && key === "c") {
          e.preventDefault();
          s.updateDraft({ showCc: true, fieldsOpen: true });
        } else if (mod && e.shiftKey && key === "b") {
          e.preventDefault();
          s.updateDraft({ showBcc: true, fieldsOpen: true });
        } else if (mod && e.shiftKey && key === "a") {
          e.preventDefault();
          s.attach();
        } else if (e.altKey && e.code === "KeyS") {
          // ⌥ changes e.key on macOS, so use the physical key.
          e.preventDefault();
          s.toggleSignaturePicker();
        } else if (e.altKey && e.code === "KeyF") {
          e.preventDefault();
          s.toggleFromPicker();
        }
        return;
      }
      if (s.view === "thread" && mod) {
        // Your draft here, shown at the end of the conversation, unless you asked for Quick reply.
        const quickHere = useQuickReply.getState().threadId === s.openThreadId;
        const draft = quickHere ? undefined : threadDraft(useDrafts.getState().drafts, s.openThreadId);
        if (e.key === "Enter") {
          e.preventDefault();
          if (draft) s.sendDraft(draft);
          else s.sendInstantReply();
        } else if (key === "e") {
          e.preventDefault();
          if (draft) s.openDraft(draft.id);
          else s.editInstantReply();
        }
        return;
      }

      if (mod || isTyping(e.target)) return;

      // ⌥I: the open email's images, just this once. Uses e.code because ⌥ changes e.key on macOS.
      if (e.altKey) {
        // ⌥0–⌥9: all of the account's addresses, or one of them, as the address row shows them.
        if (digit !== undefined && s.view === "inbox" && s.accountFilter !== "all") {
          const row = addressCounts(s.threads, accountFor(s.accountFilter));
          if (digit === 0 || row[digit - 1]) {
            e.preventDefault();
            s.setAddressFilter(digit === 0 ? undefined : row[digit - 1].email);
          }
        }
        if (e.code === "KeyI" && s.view === "thread") {
          e.preventDefault();
          const id = openThread(s)?.messages.at(-1)?.id;
          if (id) s.showImages(id);
        }
        return;
      }

      // Single keys

      const handled = () => e.preventDefault();
      switch (key) {
        case "arrowdown":
          handled();
          s.moveCursor(1);
          break;
        case "arrowup":
          handled();
          s.moveCursor(-1);
          break;
        case "enter":
        case "o":
          if (s.view === "inbox") {
            handled();
            s.open();
          }
          break;
        case "e":
          handled();
          s.markDone();
          break;
        // ⇧H: your own email comes back if no one replies.
        case "h":
          handled();
          // ⇧H: back if no one replies to your latest email.
          if (e.shiftKey) s.remindIfNoReply();
          else s.openRemind();
          break;
        // M: to Important, or out of it, for good.
        case "m":
          if (s.view === "inbox" || s.view === "thread") {
            handled();
            s.openMove();
          }
          break;
        // ⌫ is "delete" on a Mac keyboard; Delete is the forward-delete key elsewhere.
        case "backspace":
        case "delete":
          handled();
          s.deleteThread();
          break;
        // A new sender's email, open or under the cursor, like the banner: B blocks, A accepts,
        // and with ⇧ everyone at their domain. U unblocks blocked mail you are reading.
        case "b":
        case "a": {
          const t = s.view === "thread" ? openThread(s) : s.view === "inbox" ? findThread(s, s.cursorId) : undefined;
          if (t?.sender !== "new") break;
          handled();
          if (key === "a") s.acceptSender(e.shiftKey, t.id);
          else s.blockSender(e.shiftKey, t.id);
          break;
        }
        case "u":
          if (s.view === "thread" && s.blockedThreads.some((t) => t.id === s.openThreadId)) {
            handled();
            s.unblockSender();
          } else if (s.view === "inbox" || s.view === "thread") {
            handled();
            s.toggleRead();
          }
          break;
        case "r":
          handled();
          s.compose(e.shiftKey ? "reply-all" : "reply");
          break;
        case "f":
          handled();
          s.compose("forward");
          break;
        case "c":
          handled();
          s.compose("new");
          break;
        case "i":
          if (s.view === "thread") {
            handled();
            if (!e.shiftKey) s.toggleRecipients();
            else {
              // ⇧I: images from the open email's sender, from now on.
              const id = openThread(s)?.messages.at(-1)?.id;
              if (id) s.allowImages(id);
            }
          }
          break;
        case "tab":
          if (s.view === "inbox") {
            handled();
            s.cycleSplit(e.shiftKey ? -1 : 1);
          } else if (s.view === "thread") {
            const n = openThread(s)?.replyOptions?.length ?? 0;
            if (n) {
              handled();
              s.setReplyChoice((s.replyChoice + (e.shiftKey ? n - 1 : 1)) % n);
            }
          }
          break;
        case "/":
          handled();
          s.openSearch();
          break;
        case "d":
          handled();
          s.openDone();
          break;
        case "z":
          handled();
          s.showInboxZero();
          break;
        case "?":
          handled();
          s.openSettings("keyboard");
          break;
      }
    };
    const stopKeys = listenAppKeys(onKey);
    return () => stopKeys();
  }, []);
}
