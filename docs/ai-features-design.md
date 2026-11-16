# AI features: design spec

Frames live in `design.pen` (rows starting at y = 20400). Every frame has a dark version (`AI · …`) and a light one (`AI Light · …`, same x, 1200 lower). Open them through the Pencil tools only.

This is the design spec for summaries, AI replies and Ask Moss. It describes the designed behaviour, not a promise that every detail is built. Where it and the frames disagree, the frames win.

## One AI language

Everything AI looks and behaves the same way, so you learn it once.

| Thing | Rule |
| --- | --- |
| Colour | Teal is only for AI. AI-made content sits on `$x-ai-bg` with `$x-ai-ink` text; AI labels use `$x-ai`. Dates that are due use the amber `Due` pill (`$x-due-bg` / `$x-due`). |
| Marker | **No sparkles anywhere** (the user asked for none). Moss's mark is the lucide `leaf` icon (13–16 px, `$x-ai`) + a short label: "Short summary", "Moss". Action buttons use an icon for the action instead: Summarise `scroll-text`, Quick reply `zap`, Write with Moss / Change it `pen-line`, Ask `message-circle`. |
| Why it's there | Every AI block says why, in plain words, next to its label: "You asked just now", "Always for Dana Whitfield", "Because you replied in this conversation". |
| Editable text | AI text you can change sits in a `$x-surface` card with a `$x-ai-line` border (Quick reply, draft cards). It is ordinary, editable text. |
| Sending | Moss never sends. Every draft ends in a Send button the user presses (`⌘↵`). Sending never marks mail Done. |
| States | Loading, model not downloaded, and failed all appear in the same place as the result. No pop-ups, no toasts. |
| Keys | Every action shows its key. Lists you type into use ↑ ↓ + ↵ (number keys would type digits). Lists with no text box use 1–5. |
| Privacy line | Menus and panels end with a lock icon and "Runs on this Mac". |

Name: the AI is called **Moss** in the UI ("Ask Moss", "Write with Moss", "Moss wrote this").

## Keys

Checked against `src/state/useShortcuts.ts` and `src/state/commands.ts`. All of these are free today.

| Key | Where | Does |
| --- | --- | --- |
| `S` | Reading view | Opens the summary menu. With a summary showing, opens it again to change the kind. |
| `1`–`5` | Summary menu | Short, Detailed, Action points, Key facts, Custom. |
| `↵` / `⇧↵` / `Esc` | After a summary appears (no rule yet) | Always for this sender / Always for everyone at this domain / Not now. |
| `Q` | Reading view | Quick reply: Moss writes a reply in the docked card. |
| `Tab` | Quick reply card | Next reply option (same as `InstantReply` today). |
| `⌘J` | Everywhere | Ask Moss. The email on screen is its context, and Moss works out whether a question is about it. In the composer it opens "Write with Moss". |
| `⌘R` | Ask panel | Draft a reply from this chat. |
| `⌘E` / `⌘↵` | Draft card (Ask panel or Quick reply) | Edit in the full composer / Send. |
| `⌘O` | Draft card in Ask Moss | Open the email being replied to. |
| `↑ ↓` `↵` | Ask panel, Write with Moss menu | Pick a suggestion, email card or action; ↵ runs it. |
| `⌘Z` | Composer | Undo what Moss wrote. |
| `Esc` | Any AI state | Stop writing / close the menu or panel. |
| `⇧↵` | `⌘K` palette | From a list: open the email under the cursor and ask about it. |

Write with Moss menu: with text in the box, `↵` writes from your note. With an empty box, `↵` runs the highlighted action.

## 1. Summaries in the reading view

The top bar gets two AI buttons left of Done / Remind / Delete: **Summarise `S`** and **Ask `⌘J`** (teal icons, normal labels). The summary card sits under the subject, above the earlier messages, full column width.

### Frames

| State | Dark | Light |
| --- | --- | --- |
| Reading view with AI buttons (no summary) | `IYxJW` | `MHolO` |
| Summary menu open (`S`) | `I8Mbw` | `mvnt2` |
| Short + "always" offer | `mhsKH` | `k9ajlR` |
| Action points, rule for Dana | `L3QJWN` | `VlLYx` |
| Key facts, rule for a whole domain | `O9RCa` | `eWi3s` |
| Detailed (timeline) | `KHANW` | `vUsXV` |
| Custom (your instruction) | `vOHVC` | `YKV0Q` |
| Auto one-liner, labelled, with "why" tooltip | `Fym3L` | `sbAuq` |
| States: loading, model not downloaded, failed | `D70ql` | `pM1TD` |
| Settings → AI → Summaries | `hM4AM` | `GnPhw` |

### Kinds and words

| Kind | Menu line | Card label | What it shows |
| --- | --- | --- | --- |
| Short | "One or two lines" | Short summary | 1–2 sentences. |
| Detailed | "Every point, in order" | Detailed summary | One row per step: date (teal, bold) + what happened. Last row is "Today". |
| Action points | "What you need to do, with dates" | Action points | Checklist. Each row: checkbox, the task, amber due pill. Ticking is local only. |
| Key facts | "People, dates, amounts, links" | Key facts | Four labelled rows: PEOPLE, DATES, AMOUNTS, LINKS. Links are teal and open. |
| Custom… | "Say how you want it" | Custom summary | An instruction box at the top (`↵ Redo`), the result under it. |

Card header: `leaf` + kind label + why + **Change kind ⌄ `S`** on the right. A rule adds **Stop rule** (bell-off icon).

### The "always" offer

Shown only after you pick a kind for a sender with no rule. It sits at the bottom of the card, under a thin `$x-ai-line` divider:

> Always summarise mail from dana@brightlane.com like this?
> [Always for Dana Whitfield `↵`] [Everyone at brightlane.com `⇧↵`] [Not now `Esc`]

- Use the sender's address in the question so it's clear which one.
- Hide the domain button for public domains (gmail.com, icloud.com…). Reuse `isPublicDomain` from `src/lib/senders.ts`.
- Custom says "…this way?" and saves the instruction with the rule.
- "Not now" hides the offer for this email only.

### With a rule

The summary is already there when the email opens. Why-text: "Always for Dana Whitfield" or "Always for everyone at brightlane.com", with a `repeat` icon. **Stop rule** removes it straight away and the card stays until you leave. Undo with `⌘Z`, like other actions.

A sender rule beats a domain rule.

### The automatic one-liner

Moss writes one line by itself for threads with 2 or more messages that are in Important or that you've replied in (`crates/moss-store/src/summaries.rs`, `to_summarize`). The user couldn't tell why it appeared, so it now has a label:

- Label: **Auto summary**. Why: "Because you replied in this conversation" or "Because it's in Important".
- An `info` icon opens a tooltip: "Why is there a summary? Moss writes one line by itself for conversations of 2 or more emails that are in Important, or that you've replied in. It runs on this Mac. Turn it off in Settings → AI."
- It can be switched off in Settings (below).

The "ask" line ("Daniel needs you to approve…", `ThreadHeader`) is unchanged. If a thread has both, the summary card goes first and the ask line follows.

### States (frame `D70ql`)

| State | Header | Body | Actions |
| --- | --- | --- | --- |
| Loading | "Summarising 6 emails…" | Three skeleton bars in `$x-ai-line` | `Esc` Stop |
| Model not downloaded | "Summaries need the writing model" (download icon) | "Gemma 4 E4B runs on this Mac. It's about 5 GB and downloads in the background." | Download `↵`, AI settings `⌘,` |
| Failed | "Couldn't summarise this one" (circle-alert) | "The writing model stopped before it finished. Your email is fine." | Try again `↵`, Change kind `S` |

### Settings → AI → Summaries (frame `hM4AM`)

A new section **SUMMARIES** after WRITING MODEL. (The frame shows the page scrolled, so the top sections are out of view.)

- First row: **Auto summaries** + the plain rule as its subtitle + an on/off switch.
- Then one row per rule: icon (`user` for a person, `at-sign` for a domain), name, address (and the custom instruction in quotes), a teal **kind pill with ⌄** to change it, and remove.
- The row under the cursor shows `⌫ Remove`; the others show a quiet `×`.
- Note under the list: "Add a rule from any email: press S, pick a kind, then choose Always."
- Shortcut bar: `J K` Move · `↵` Change kind · `⌫` Remove rule · `⌘1–⌘6` Sections · `Esc` Close.

## 2. AI reply

### Quick reply in the reading view (frame `tlx2N` / `HGJva`)

This reworks `InstantReply`. The reply bar gains **`Q` ⚡ Quick reply** (`zap` icon) (teal) between Reply and Reply all. Pressing `Q` swaps the reply bar for the docked Quick reply card:

- Header: `zap` **Quick reply** · "Moss wrote this. Nothing sends until you press Send." · `Tab` next option.
- Option chips (from `replyOptions`): the selected one has a `$x-ai-select` border.
- The draft is **editable in place**, with the cursor at the end. There's no separate "Edit" step.
- Footer: From picker · **Change it `⌘J`** (opens the Write with Moss menu) · **Full editor `⌘E`** · **Send `⌘↵`**.

The current InstantReply behaviour (`⌘↵` sends, `⌘E` opens the editor, `Tab` cycles) stays the same.

### Composer: Write with Moss (frame `QDtWW` / `y2rChc`)

- Toolbar gets **Write with Moss `⌘J`** (`pen-line` icon) as its first button.
- `⌘J` opens the menu where the cursor is. The top is a text box: "Tell Moss what to say and how, e.g. "yes to the 17th, keep it short"" · `↵` Write.
- **REPLY FOR ME**: Quick reply ("Moss picks what to say"), Say yes ("Accept what Dana asks"), Say no, kindly ("Decline and keep it friendly"), Ask for more time ("Suggests a new date").
- **CHANGE WHAT'S WRITTEN**: Shorter ("Same point, fewer words"), Change tone ("Friendlier, more formal, more direct"), Fix spelling and grammar ("Changes only mistakes"), Translate ("Into Dutch, German, French, Persian…").
- If text is selected, the second group acts on the selection only.
- Footer: "Lands as text you can edit. Nothing sends by itself." · `Esc` Close.

### After Moss writes (frame `VqGGl` / `w2Eif`)

A teal strip above the text: `leaf` **Moss wrote this from your note** / the note in quotes · **Change it `⌘J`** · **Undo `⌘Z`**. The strip goes away once you type in the text. For a menu action the first line names it, e.g. "Moss made it shorter".

### Writing states (frame `RwwrU` / `R5hzL`)

The same strip is used in the composer and in Quick reply:

- **Writing:** "Moss is writing…" / "Your words stay as they are until it's done." · Stop `Esc`. The text streams in where it will land, with a teal cursor.
- **Model not downloaded:** "Writing needs the writing model" · Download `↵` · AI settings `⌘,`.
- **Failed:** "Couldn't write this one" / "The writing model stopped. Your draft is unchanged." · Try again `↵` · Undo `⌘Z`.

## 3 and 4. Ask Moss: one panel, one chat

**One chat:** no scope switch and no context line. The email on screen is the context; Moss works out from each question whether it's about that email or any other, and can act on mail as the window does (Done, remind, move, delete, block, write). The frames below still show the switch.

Ask Moss is a **right-hand panel**, 440 px wide, `$x-surface`, with a left border.

- `⌘J` opens it anywhere. With an email open, its suggested questions come first.
- In the reading view the email column moves left (padding 130) so the column stays 740 wide. In the inbox the list narrows; the row under the cursor stops showing its hover keys while the panel has focus.
- Header: `leaf` **Ask Moss** · `Esc` Close.
- Bottom: text box with a teal border ("Ask about this email, or any other…" / "Ask about any email…" / "Ask a follow-up…") + `↵`, and a line under it: "Runs on this Mac. Answers come only from your mail."
- After the first answer, chat actions sit above the box: **Draft a reply from this chat `⌘R`** · New chat.

### Frames

| State | Dark | Light |
| --- | --- | --- |
| This email, start: suggested questions | `xMcPb` | `OXhU0` |
| This email, answer with sources + draft reply | `LC7J6` | `N1Vgc` |
| All mail, answer cites emails as cards | `dqC1r` | `G8t3O` |
| All mail, draft a reply without opening the email | `a9w3A` | `P7YkTJ` |
| `⌘K` palette with the Ask Moss entry | `oXgeP` | `S4VBL` |
| States: thinking, model not downloaded, nothing found, failed | `AkCIm` | `gbhWe` |

### Chat

- Your messages: right-aligned `$x-raise` bubbles, at most about 300 px wide.
- Moss's turns: no bubble. `leaf` **Moss** label, then the text.
- **Sources** (this email): small chips under the answer ("Dana · Oct 28"). Clicking one scrolls to that message.
- **Email cards** (all mail): account badge, sender, when, subject, attachment. The first card is selected when the answer lands; `↑ ↓` moves, `↵` opens. Typing goes back to the box.
- Suggested questions (this email): "What does Dana need from me?", "What changed since my last email?", "When is everything due?", "Draft a reply that says yes". Use the sender's first name.

### Draft cards (both scopes)

A `$x-bg` card with a `$x-ai-line` border:

- In All mail it starts with a small "replying to" box: badge, sender, time, subject and snippet, `⌘O` Open.
- "To Priya Nair · from hello@rossstudio.art" (the From address is picked the same way as in the composer).
- The draft text, then **Edit `⌘E`** · Not now · **Send `⌘↵`**.
- Under it: "Sending doesn't mark Priya's email Done. You decide that."

### `⌘K` palette

When you type a question, the ASK AI group starts with **Ask Moss: "…"** ("Answers from all your mail, in the Ask panel", `↵`). If an email is in context, the next line is **Ask about <subject> only** ("Just the email you're on", `⇧↵`). The footer gains `⌘J` Ask Moss. This replaces today's placeholder "Summarize everything about…" item in `aiSuggestions`.

### States (frame `AkCIm`)

- **Thinking:** "Looking through 2,431 emails…" + skeleton · `Esc` Stop.
- **Model not downloaded:** "Ask needs the writing model. Gemma 4 E4B runs on this Mac, about 5 GB." · Download `↵` · AI settings `⌘,`.
- **Nothing found:** say so plainly and offer the closest thing: "I couldn't find a PDF from a bank. Closest: a statement from your bank on Thu, with no attachment." · Open it `↵` · Search instead `/`.
- **Failed:** "Something went wrong before I could answer. Nothing was changed." · Try again `↵`.

## Older frames changed too

The sparkles icon is gone from the whole file: Settings nav "AI", the "Needs your reply · AI, 95% sure" line (Email · Sorted by AI) and the search "meaning" hint now use `leaf`. The code still imports `Sparkles` in `ThreadHeader.tsx`, `SettingsScreen.tsx` and `SearchScreen.tsx`; swap it for `Leaf` when you touch those.

## Scene names (suggested)

For `src/dev/scenes.ts`: `summary-menu`, `summary-short-offer`, `summary-action-points-rule`, `summary-key-facts-domain`, `summary-detailed`, `summary-custom`, `summary-auto`, `summary-loading`, `summary-no-model`, `summary-failed`, `settings-ai-summaries`, `quick-reply`, `compose-moss-menu`, `compose-moss-wrote`, `compose-moss-writing`, `ask-email-start`, `ask-email-draft`, `ask-all-cards`, `ask-all-draft`, `palette-ask`, `ask-thinking`, `ask-no-model`, `ask-nothing-found`, `ask-failed`.
