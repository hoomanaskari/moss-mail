# Ask Moss: chatting with your mail

Decision record · 4 November 2026 · Status: **built**: the backend,
and the Ask Moss panel from the approved design (sections 3 and 4 of [ai-features-design.md](../ai-features-design.md)).

Builds on [ai-split.md](ai-split.md): Gemma 4 writes, in llama.cpp's `llama-server`, on this Mac.

## 7 November: one chat that acts

Two changes, asked for together: the chat can do with mail everything an agent can over MCP (see
[mcp-server.md](mcp-server.md)), for people who want AI to act on their mail and keep it on the
Mac; and the two chats became one.

- **One chat, no tabs.** The email on screen is the context when there is one. Moss works out
  from the question whether it's about that email or any other. No This email | All mail switch,
  no context line, no `Tab`.
- **The MCP tools, without the server.** `moss_mcp::MossTools::chat_specs` / `chat_run` give the
  chat the same tools, through `moss_ai::tools::Actions` (`src-tauri/src/ask.rs`,
  `EngineActions`). The chat keeps its own search, read, files and people tools (they cite what
  they found), so the MCP ones that do the same are left out (`CHAT_LEAVES_OUT`).
- **One tool writes every email: `write_email`.** It is MCP's `send` with its own words, and it
  never sends: the email waits under the answer for you to press Send, or Send later at the time
  you asked for (`send_at`). The card says "Not sent yet" whatever the answer says.
- **What it changed is listed** under the answer ("Marked done: Studio Notes, issue 42"), each
  with a check.

### Tried on the real model (Gemma 4 E2B, the one on this Mac)

`src-tauri/tests/real_actions.rs`: ten requests over a made-up mailbox, through the app's real
wiring. The first try, with the prompt and tools as they first were, got **0 of 10**: E2B ignored
the open email ("What email are you referring to?") and picked the wrong tools. What fixed it,
in order:

| Change | Why |
|---|---|
| A note beside the question: *(I have the email "…" from Dana Lee open. Here "this", "it", "she"… mean that email.)* | a small model heeds what's next to the question, not what's far up in the prompt |
| Say how to find emails: `list_inbox` (by split) or `search_mail`; never ask which email | it asked the user instead of looking |
| Ids written as citations (`t:5`, `[t:5]`, `5`) reach the tools plain | it passed `t:5` as a thread id |
| `write_email` instead of `create_draft` + `send`; its text required | with two writing tools it never set `send_at`; with the text optional it left it out |
| A search with no mail for every word tries each word | it searched "contract email" |
| The user's time zone in the prompt, and "fix what a tool says and try again, or say it didn't work" | it wrote times in UTC, and claimed success after a refused call |
| Reminders and moves listed in plain words | an agent reads "back at 2026-11-08T08:00:00Z" |

Then **10 of 10, three runs in a row**: Done on three newsletters (found, then three calls in one
step), a reminder on the open email, a reply to Priya readied and not sent, "By when does she need
it?" about the open email, moving it, a reply set to go tomorrow at 8, a forward to a new address,
deleting the open email, blocking a sender. Each answer took 0.4 to 4 s.

Then in the app itself, on test emails, three more misses, now in `real_actions.rs` too:

| Asked | It did | Fix |
|---|---|---|
| "Stop reminding me about the Moss chat test A email" | called `remind_if_no_reply`, with the subject as the thread id | the two reminder tools described so they can't be confused; a subject given as an id is looked up (one match: used; several: listed for it to choose) |
| the same, again | called `remind_me` with no thread at all, then asked the user | an action that names no email gets the one whose subject shares the most words with the question ("test **A**" counts), or a short list when it's a tie |
| "Mark all the Moss chat test emails as done" | looked only in the Other split, found nothing | "search by name or subject; give list_inbox a split only when the user names one" |

In real mail an email you sent yourself has copies (the one you received, your sent copy, the
reply), so several match equally. Then Moss prefers the one the action fits: the copy with a
reminder to stop, one not Done to mark Done; a list of the rest says how each stands ("(Done)",
"(coming back later)"). Tried again in the app: the reminder stopped on the right copy, all the
test emails went Done, "Mark this as done" and "Delete my draft reply about lunch" worked.

### The harness: the model judges, Moss gives it the means (7 November, later)

A first try at making E2B act used keyword rules: lists of filler words, phrases such as "I've
drafted" or "just say", subjects scored by shared words. Each one fixed one case and would miss the
next, so they were taken out. What's left gives the judgment to the model at three points, and
keeps only structural checks:

- **It looks first, with words the model picks.** Before the chat model answers, a short call
  asks it (with the chat so far) what to search the mail for: "just say I'll text him" becomes
  "Alex Morgan". Moss's search (words, then meaning) finds the emails, listed beside the question.
- **It picks the email an action means.** When an action names no email, or something that isn't
  an email's id (a subject, a name), Moss searches for it and the model picks one from what was
  found, seeing how each stands ("Done", "coming back later"); its answer can only be one of those
  ids or "none". When it can't tell, the choices go back to the chat model.
- **It reviews its own answer.** After each answer the model looks at the question, its tool calls
  with what they said, and the answer, and says whether the job is done; if not, what to do now
  (up to twice). This catches asking the user what it could look up, saying it did what no tool did,
  acting on only some of the emails, saying an email was sent, and writing an email out as text.
- **Writing to someone new to the chat.** `find_people` is always on offer (a small model that
  picks its tools left it out, then made up an address), and says how often and when last the
  user and each person wrote, so it takes the address in use over an old one. `to` can be just a
  name, which Moss looks up. A new email goes from the account and address the user last wrote
  that person from (`Store::writes_from`). `from` is only ever one of the user's addresses: an
  address of theirs picks its account, and in the chat one that isn't theirs is left out (a small
  model put the recipient there too). A new email without a subject is sent back for one.
- **Emails are written as Quick reply writes them.** What the model puts in `write_email` (or,
  with nothing, what the user asked) is the note; Quick reply's writer (`reply::draft`) turns it
  into the email, with a greeting, the user's voice from what they wrote that person before, and a
  sign-off that fits the draft's signature, and the draft takes it. The model still hears what
  `write_email` said (which kind of email, and that Moss shows it under the answer), never the
  written email: given it, a small model pastes it into its answer above the card.
- **Structural checks only:** a call repeated with the same arguments isn't run again (OpenCode
  does the same, its "doom loop"); a tool call the model writes as text is cut from the answer.

These short calls answer through a tool the model has to call (`chat::decided`): asked for JSON
instead, Gemma thinks aloud and runs out of room before it answers.

Tool results leave out null, false and empty fields (an empty list at the top stays: it says none
were found), which helps any agent on MCP too.

Tried on the real models: the made-up mailbox (19 checks) with no misses on E4B and one on E2B (2
of 3 newsletters marked); a historical multi-turn reply scenario containing copied private correspondence, right draft in the
right thread 3 of 3 on E4B. The original private fixture has since been replaced with a synthetic scenario; this result is historical, not a rerun of that replacement.

### Same model, another harness

To check whether the model or Moss's harness was the problem, the same tests ran in OpenCode (the
latest, 2.2.6) against Moss's MCP server over the evaluation mailbox, which then included copied private correspondence. E2B: 0 of 3 (it asked for
thread ids). E4B: 0 of 3, though smarter (it found the selected contact's email, then asked what to say). Again
after the tool results were trimmed: E4B wrote the reply out as text and said no tool lists
newsletters. Pi has no MCP of its own, so it wasn't tried. A general coding harness doesn't make a
small model act on mail; looking first, picking and reviewing do.

The historical run of `the_real_writing_model_drafts_a_reply_from_a_name` reported 3 of 3 passes before the fixture replacement. Its current synthetic scenario has not been rerun with the real model.

Also: the open email's reminder is in the prompt ("To stop it, call remind_me without until"), and
the open email is no longer shown as a card under answers about it.

Limits that changed with it: 10 model calls per question (was 6), several tool calls per step
(`parallel_tool_calls`), and the open email capped at 48,000 characters (was 72,000) to leave
room for the 21 tool definitions (about 4,000 tokens).

## Before: two ways to ask

- **Ask about this email:** a chat beside the open thread. The whole thread is the context:
  every message, who sent it, when, and the attachment names. Follow-up questions remember the
  earlier ones, per thread, until Moss quits (the last 20 threads asked about; older chats are
  forgotten). It can also draft a reply to this thread. New chat forgets a chat everywhere.
- **Ask Moss:** a general chat. Moss looks things up with tools, then answers and shows the
  emails it used as cards.

## The question: does Gemma 4 call tools reliably?

Tried for real, 4 November 2026, on an M4 Max (36 GB), with the model Moss downloads:
Gemma 4 **E4B** Q4_K_M in `llama-server` **b11960**, started like Moss starts the writer
(`-np 1 -c 8192`, Jinja chat templates on, which is the default in this build).
Five tools (search mail, read a thread, list attachments, find people, draft a reply), canned
results, ten questions, run several times. The probe script is not in the repo; its cases are
the ones in the table.

| | Native tool calls (`tools` in the request) | JSON plan (`response_format` schema, no tools) |
|---|---|---|
| Picked the right tool | **37 of 40** (all 3 misses: the same question, below) | right tool, **wrong argument name** (`name` for `query`) |
| Broken calls (bad JSON, unknown tool) | **0 of 40** | the schema can't check each tool's arguments |
| Several steps (search → read → answer) | yes, on its own | Moss would have to drive every step |
| Cited the thread it used (`[t:41]`) | 29 of 34 answers that used mail | n/a |
| Time per question, thinking off | **0.4–3 s** | similar |
| Time per question, thinking on | 2–15 s, no more accurate | |
| Streams | yes: text and tool calls both stream (OpenAI format) | |

The one miss, every time: *"Send Dana a message saying thanks."* There is no tool to write a
new email, so it asked which thread to reply to. Fine for now (it never sends), and a
`draft_email` tool is the obvious next one.

Other things it did:

- It once answered with **nothing** after a tool call. Moss asks once more ("answer now").
- It **cites loosely:** `[t:41]`, `[t:41, t:57]`, and once `[t:1]` for a person (no such
  thread). Moss only keeps ids a tool actually returned.
- The five tool definitions cost about **390 tokens** of the 8,192.

Then the built code end to end, same Mac and model (`crates/moss-ai/tests/real_ask.rs`):

- *"Did Sam approve the Q4 budget, and how much is it?"*: searched, read a thread, answered
  with both emails cited. **4.9 s**, model load included.
- *"Draft a reply to Dana confirming it."*: searched, saved a draft on the right thread.
  **2.5 s**. The first run signed it "Best, Moss"; the tool now says to end with a sign-off
  and no name, and Moss drops a "Moss" sign-off line if one still comes.
- Ask about this email, *"When is it?"* on a lunch invite: "Thursday at noon". **0.8 s**.

## Choice

- **Native tool calling**, thinking **off**. No JSON-plan fallback: it was worse at the one
  thing a fallback is for, and the native path didn't break once. A model behind another
  provider (cloud, Ollama) gets the same OpenAI-style `tools`, which all of them speak.
- **The model sits behind a trait** (`moss_ai::chat::ChatModel`). Today `Ai` implements it with
  the local writer. A cloud provider or Ollama is another implementation; the loop and tools
  don't change.
- **The loop is Moss's own** (`moss_ai::agent`), about 200 lines. Pi and OpenCode were design
  references only.

## Hard rules, enforced by what the tools can do

Sending stays yours: no tool sends. `write_email` saves an ordinary draft and hands it to the UI,
where you press Send. Everything else it can do (Done, read, remind, move, delete to Trash, block,
accept) is a step you can take back. (Before 7 November it had no tool for any of them.)

## Limits: enough to read whole emails

The writer runs with a 32,768-token context (raised from 8,192 for summaries, 4 November). Email
is short next to that, so Ask reads it whole: no email is cut to its first few lines.

| Limit | Value | Why |
|---|---|---|
| Model calls per question | **10** | enough to find emails, act on each, and answer. The last call gets no tools, so it must answer. |
| Answer length per call | 1,024 tokens | a full answer, or a long draft reply |
| A thread the agent reads | 40,000 characters (~10,000 tokens) | a long email and its replies, whole; two still fit |
| A list (search hits, files, people) | 4,000 characters | it only points at emails; the agent reads the ones it needs |
| The open email | 48,000 characters (~16,000 tokens) | a long conversation whole, with room for the tool definitions; only a longer one loses its oldest messages |
| Whole request | 32,768 − 1,024 − tool definitions | oldest turns of the chat are dropped first, then old tool results are shortened |

Tokens are counted roughly (characters ÷ 3, which overcounts English: safe). Tests check the
limits fit the context together.

**Why this changed (4 November).** The first limits were sized for the 8k context: a thread the
agent read was cut at 2,400 characters (about one email) and the open thread at 14,000. On the
real E4B, a fact at the end of a 23,000-character email was lost: asked for it, Gemma said
"I do not see any information about an entry code … in this thread." With the limits above it
answers from the whole thread, in Ask about this email (13 s, model load included) and in All mail
(it searched, read the thread, and cited it; 14 s). `tests/real_ask.rs` checks both.

## What the UI uses

Tauri commands (`src-tauri/src/ask.rs`):

| Command | Args | Returns |
|---|---|---|
| `ask_moss` | `chat` (any id the UI picks), `question`, `open` (the thread on screen, or null) | `AskAnswer` |
| `ask_stop` | `chat` | `bool`: was one running |
| `ask_forget` | `chat` | nothing; the chat starts over |
| `ask_mail_count` | | emails on this Mac, for "2,431 emails on this Mac" |

Event `moss://ask`, while an answer is written, tagged by `type`:

| `type` | Fields | Means |
|---|---|---|
| `step` | `chat`, `step` | the model is thinking (step 1, 2…). Text after this replaces earlier text. |
| `tool` | `chat`, `tool`, `label` | it's looking something up, e.g. "Searching mail for “Q4 budget”" |
| `text` | `chat`, `delta` | more of the answer, citation marks already taken out |
| `done` | `chat`, `answer` | the finished `AskAnswer` |
| `failed` | `chat`, `error` | `{kind, message}`; `kind: "notReady"` means the model isn't downloaded |

`AskAnswer` is `{ text, sources: ThreadSummary[], messages: string[], files, drafts: Draft[], sendAt, did, stopped }`:
`sources` are the emails to show as cards, in the order the answer cites them; `messages` are the
messages of the open email it cites (`[m:ID]`), for its source chips; `drafts` are emails waiting
for you to check and send; `sendAt` maps a draft's id to when you asked for it to go; `did` lists
what it changed.

TS: `src/services/ask.ts` and `src/services/tauriAsk.ts`:

- `ask()` is the backend (`askMoss(chat, question, open?)`, `stop`, `forget`, `onEvent`).
- `startTurn(question)` and `applyEvent(turn, event)` give a `ChatTurn` per question:
  `state` is `thinking` (with `activity`, the tool label), `writing`, `done`, `stopped` or
  `failed`. `modelMissing(turn)` says to show "download the model" instead of an error.
- Answers arrive in the UI's own shapes: `sources` are `Thread`s (list rows), `drafts` are
  `SavedDraft`s ready for the composer. A draft has `signatureId: null`: the composer should
  add the account's default signature when it opens one.
- The demo: `demoAsk({ answer, threads, delayMs })` plays a script (`cited`, `draft`, `thread`,
  `acted`, `nothing`, `thinking`, `missing`, `failed`); `demoTurn(kind, question, threads, open?)`
  is a turn as a scene shows it, without waiting.

## The panel

One right-hand panel, 440 px, beside every mail screen (not the composer, Settings or adding an
account): `AppFrame` has a side slot the app fills, so screens don't each add it.

- `src/state/askPanel.ts`: open, the chat's turns, the box, ↑ ↓ picks. ⌘J opens it; with an email
  open its suggested questions come first. Esc stops, then closes.
- `src/design-system/organisms/AskPanel`: the panel as drawn (frames `xMcPb`, `LC7J6`, `dqC1r`,
  `a9w3A`, `AkCIm`), with its keys: ↵ ask or open the picked card, ⌘R draft from this chat,
  ⌘E / ⌘↵ / ⌘O on the latest draft card.
- `src/screens/AskSide.tsx`: joins them, and words the suggestions and states.
- A drafted reply goes from the address the email came to (`replyFrom`, only an added address),
  with that address's signature, and is kept in Drafts. Send sends it with Undo and stays on the
  screen you're on (`store.sendDraft`). Sending never marks the email Done.
- ⌘K: "Ask Moss: “…”" (↵), and from a list "Ask about <subject>" (⇧↵), which opens that email
  first.
- Scenes: `ask-email-start`, `ask-email-draft`, `ask-all-cards`, `ask-all-draft`, `ask-acted`,
  `ask-send-later`, `palette-ask`, `ask-thinking`, `ask-no-model`, `ask-nothing-found`,
  `ask-failed` (`src/dev/askScenes.ts`).
  Screenshots against the frames differ by 1–8 % of the panel's pixels: text wrapping, and the
  app's thread top bar, which is 10 px shorter than the frame's.

Where it differs from the frames, on purpose:

- **Nothing found** offers Search instead (/), and no "Open <closest> email" button: the agent
  doesn't mark which email is the closest. When it shows one, it's an email card.
- **All mail keeps "Draft a reply from this chat" and "New chat"** above the box after an
  answer, as the spec says; the All mail frames leave them out, but then New chat can't be reached.
- Draft cards are read-only; Edit (⌘E) opens the full composer.
- An email card shows the email's time as the list does ("9:12 AM", not "Today, 9:12 AM"), and no
  "older draft" note.

## Not now

- Undo (⌘Z) for what the chat changed: each change can be taken back, but by hand.
- Calendar, contacts and other connectors, through an MCP client.
- Remembering chats after Moss quits.
- Cloud models and Ollama as `ChatModel`s.
