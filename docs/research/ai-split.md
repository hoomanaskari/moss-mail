# AI in Moss: which model does which job

Decision record · 2 November 2026 · Status: **built**.

Builds on [decision-models.md](decision-models.md) (agreed 2 November 2026):
`d3-lite` sorts and decides, Gemma 4 writes, no cloud and no fallback by default.

## The problem

Settings → AI was a mock-up. The Mac, the model list, the sizes, the download and the
cloud options were all made up. There was no AI code in Rust at all.

## Two models, two jobs

- **Decision model, `d3-lite`:** picks from a fixed list of answers and says how sure it is.
  Fast (about 40 ms per question on an Apple GPU), small (1.1 GB), always the same one.
- **Writing model, Gemma 4:** writes text. Slower, bigger. You pick the size in Settings.
- **Rules:** anything a header or your own mail settles for certain. Free and instant.

Neither model stands in for the other. Before a model is downloaded, its jobs don't run
(the rules still sort mail).

## Every AI job, and who does it

✅ built now · 🔜 next · later = after the first set works on real mail.

| Job | Who | When |
|---|---|---|
| **Needs your reply** (Reply or Info) | decision model | ✅ new mail in Important |
| Split (Important, Other, Newsletters…) | rules as mail arrives; decision model moves Important and Receipts mail when sure, decides where list mail goes, and files sales pitches under Newsletters | ✅ |
| Has a deadline (the amber due chip) | decision model, then a date parser; rules ("by Friday") without it | ✅ |
| Expects a reply → back in 3 days if no one answers | decision model; rules (a question, "let me know") without it | ✅ |
| Suggest done (FYI mail, nothing to do) | decision model, suggestion only | later |
| Typed or spoken words → command | decision model | later |
| Which signature | decision model | later |
| New sender: expected or unsolicited? (block hint) | decision model; the code decides who is new | later |
| Show remote images? | rules (see [remote-images.md](remote-images.md)) | done, no model |
| Spam, phishing | the mail provider | no model |
| Mentions an attachment, none attached | keyword rule | later, no model |
| **"The ask":** one line on what you need to do | writing model | ✅ when you open mail that needs your reply |
| Thread summary, the side-panel insight | writing model | 🔜 |
| Reply options (instant reply) | writing model, only for mail that needs a reply | 🔜 |
| Drafts and rewrites in the composer | writing model | later |
| Questions in search, the agent and its tools | writing model | later |

The decision model also decides **when Gemma runs**: Gemma only writes "the ask" for mail
that needs your reply, so most mail never wakes the big model.

## The runtime: llama.cpp's `llama-server`

- **Pinned build:** `b11960` (llama.cpp 0.8.0), the official macOS arm64 build,
  16.5 MB, checked by SHA-256. The same version the decision-model research used.
- **One process per model**, on `127.0.0.1` only, a free port, and a random key per run,
  so other apps and web pages on the Mac can't use it.
- Started the first time a job needs it. Stopped when Moss quits.

Why this one:

| Option | Runs `d3-lite`? | Runs Gemma 4? | Apple GPU | Notes |
|---|---|---|---|---|
| **`llama-server`** | ✅ `/v1/systemone` | ✅ | ✅ Metal | MIT. New decision models work with each update. |
| `llama-cpp-2` (Rust bindings) | ❌ the decision readout is server code | ✅ | ✅ | We'd port and own 840 lines of C++. |
| MLX | ❌ | ✅ | ✅ | Mac only; no decision models. |
| Ollama | ✅ | ✅ | ✅ | You'd have to install it first. Kept as a "use another AI" option. |
| candle | ❌ | partly | ✅ | No decision models. |

**Downloaded, not bundled, for now.** The runtime comes with the models when you press
Download, so `pnpm tauri dev` works with no extra build step.
Before a signed release: bundle it inside the app as a signed sidecar instead.

## The models Moss downloads

Pinned to an exact Hugging Face commit and SHA-256. All Apache-2.0.

| Model | File | Size | Offered | Recommended |
|---|---|---|---|---|
| `d3-lite` (decides) | `ggml-org/vllm-d3-lite-GGUF` Q8_0 | 1.08 GB | always | always |
| Gemma 4 E2B | `unsloth/gemma-4-E2B-it-GGUF` Q4_K_M | 3.11 GB | always | under 16 GB of memory |
| Gemma 4 E4B | `unsloth/gemma-4-E4B-it-GGUF` Q4_K_M | 4.98 GB | 16 GB or more | 16–47 GB |
| Gemma 4 26B | `unsloth/gemma-4-26B-A4B-it-GGUF` UD-Q4_K_M | 16.9 GB | 32 GB or more | 48 GB or more |

- **Moss reads the real Mac:** the chip name and memory, from the system.
- **Download only when you ask:** the Download button, or the first time you use an AI feature.
  Never by itself in the background.
- **Resumes** after a dropped connection or a restart (HTTP range requests).
- **Checked:** each file's SHA-256 must match, or Moss deletes it and says so.
- **"Ready" means ready:** every file is in and checked.
- Files live in `~/Library/Application Support/app.moss.mail/ai/`.

## "Use another AI"

Moss writes with its own models only. Writing with Claude Code, Codex, an API key, Ollama or
OpenRouter was built on 5 Nov 2026 and taken out on 6 Nov 2026: it went against keeping the AI
private and on this Mac. Your own agents can reach Moss through its MCP server instead.

## Tried on a real Mac (M4 Max, 36 GB)

- Settings read the Mac as "M4 Max, 36 GB memory" and recommended Gemma 4 E4B.
- The real download: `d3-lite` (1.08 GB) and E4B (4.98 GB) from Hugging Face; both checksums matched.
- Two made-up emails: `d3-lite` said 96% "needs a reply" for a request, 18% for a status report.
- A copy of a real mailbox: 4 open Important emails from the last 30 days. `d3-lite` was 51–69%
  sure on each, so it moved none (the 80% rule). Gemma wrote a sensible ask in 6 s, model load included.
- So the writing model is useful now. The decision model needs the labelled set and fitted
  thresholds below before it moves much real mail.

## How decisions are kept (built 6 November 2026)

- One loop asks every question, newest mail first, 20 at a time: split, needs reply,
  deadline, expects a reply (`crates/moss-engine/src/sort.rs`).
- Each answer says who gave it: the model, a rule, or you (`ai_decisions.source`, schema V22).
  The UI shows why: "AI, 88% sure", "says “by Friday”", "you asked".
- Without the model, rules answer deadline and expects-a-reply (`crates/moss-engine/src/rules.rs`);
  split and needs reply keep the rules' answer from when the mail arrived. Once the model can
  run, it looks again at what only a rule answered, once.
- Unsure (under 80% either way): the rule's answer stays, shown as the rule's.
- List mail (15 November 2026): mailing-list headers come on newsletters and on notices about
  your own work alike (a review on your pull request, a failed build, a sign-in alert). They only
  hold mail in Newsletters until the decision model has said what it is, with its own question
  (`prompt::list_split`). Its pick decides, sure or not, because no rule may say what is a
  newsletter; Important only when it is at least 80% sure, otherwise Other. Without the model,
  list mail stays in Newsletters. A thread you moved is never asked. Checked on made-up mail in
  `crates/moss-ai/tests/real_split.rs`.
- Sales pitches (15 November 2026): see [Sales pitches](#sales-pitches-15-november-2026) below.
- Your corrections (⌘K: move, No deadline, Don't remind me; ⇧H) are saved as yours and never
  asked again or overwritten.
- Your email that expects an answer waits in Later and comes back to Needs your reply after
  3 days with no answer ("No reply" chip). The model's own reminders never hide other mail;
  ⇧H snoozes the whole thread, like Remind me. Moss never sends a nudge.

## Sales pitches (15 November 2026)

Cold sales, recruiting, funding and partnership pitches from strangers, and their "bumping
this" follow-ups, are marketing, so they belong in Newsletters. They kept landing elsewhere.

### Why, on a copy of a real mailbox

Read-only, on a copy, counts only:

- **Most pitches were in Important, not Other.** Of the last 30 days, 51 pitches sat in
  Important and 1 (a partnership pitch sent through a helpdesk address) in Other.
- **Important:** a pitch has no list headers and comes from a person's address, so the rules
  put it in Important. The split question has no answer for a pitch: of the 4 the model was
  asked about, it said Newsletters 75% and 79% sure (under 80%, so they stayed) and Important
  85% and 87%. The other 47 were never asked, because the owner had blocked them or marked
  them done first.
- **Other:** the split question was never asked about Other mail at all (0 of 43 had an
  answer). Everything there had been placed by the rules from its automatic sender.
- List mail (the `outreach` answer, and a weak "important" turned into Other) put no pitch in
  Other.

### What Moss does now

- A second question, `prompt::pitch`: "Is the sender a vendor, agency, staffing firm or
  investor pitching their services, unasked, or following up on that pitch? Not an answer to
  the user's own request, order or account."
- Asked with it, in the same request, `prompt::wants`: "Is the sender asking the user for
  something they need from the user: help, an answer, a quote, or the user's product?" A
  would-be customer reads like a pitch to the first question; this one tells them apart.
- **Who is asked:** mail in Important from a sender you don't know (you never wrote to them,
  accepted them, or had them in a conversation you started: the same facts as the new-sender
  banner, `Undecided::known`), and all mail in Other. Never list mail (its own question has the
  `outreach` answer), receipts, or a thread you moved. Mail in Other is asked only this.
- **What it does:** sure it is a pitch (80% or more) and not sure the sender needs something
  of yours (under 80%): Newsletters, "AI, n% sure". Otherwise the split answer decides as
  before. Mail from someone you know is never taken for a pitch.
- **Already decided mail:** schema V50 drops the split answers the model left in Important, so
  the last 30 days of that mail is asked again once. Moves the model made, list mail and your
  own moves stay.
- No rule, word list or sender list says what a pitch is: the decision model's answers do.

### How it was checked

Made-up mail in `crates/moss-ai/tests/real_split.rs`, direct and list-sent, with people and
notices beside the pitches. Wording was chosen on `Tune` only. `Held` was scored once, showed a
would-be customer taken for a pitch, and the second question was added for that, so `Late`,
written after both wordings were fixed, is the clean check. Some made-up mail was then
rewritten so that none of it resembles a real email; the numbers below are from the rewritten
set, with the wording unchanged.

Pitches in Newsletters, before and now:

| Half | Before | Now |
|---|---|---|
| Tune | 4 of 14 | 13 of 14 |
| Held | 3 of 13 | 10 of 13 |
| Fresh (list mail only) | 2 of 3 | 2 of 3 |
| Late | 2 of 9 | 9 of 9 |

Guard rails, now: no one you write to goes to Newsletters (0 of 8). First-time people: 0 of 15
in `Tune` and `Held`, 1 of 6 in `Late` (an offer of paid work). Notices: 1 newly in Newsletters,
a trial-ending notice from a no-reply address (`Late`). Promotions sent from no-reply
addresses now reach Newsletters too (2 of 2). List mail sorts as before.

On a copy of the real mailbox (374 emails, counts only, nothing kept):

| Mail | To Newsletters before | Now |
|---|---|---|
| Pitches in Important, last 30 days | 7 of 51 | 37 of 51 |
| Older mail in Important from senders the owner blocked, 150 at random (40 such emails read by hand were all pitches) | 13 of 150 | 99 of 150 |
| The pitch in Other | 0 of 1 | 0 of 1 |
| Vendor webinar reminders in Other | 0 of 5 | 2 of 5 |
| Promotions in Other | 0 of 6 | 0 of 6 |
| People in Important | 1 of 21 | 1 of 21 (the same forwarded newsletter) |
| Notices in Important | 0 of 45 | 0 of 45 |
| Notices in Other | 0 of 52 | 0 of 52 |
| List mail | unchanged | unchanged |

What it can't do: a quarter to a third of real pitches still read to the model like a letter
from a person and stay in Important, and a stranger offering you work, or a notice with a sales
angle, can still be taken for a pitch now and then.

## Not now

- Fitting one temperature per question on 300+ labelled emails (decision-models.md, step 3).
  Until then Moss acts only when the model is at least 80% sure.
- `d3-mini` as a "more accurate" tier.
- Moving Gemma out of memory when it has been idle.
