# Decision models for Moss

Decision record. Research done 1–2 November 2026.

Status: **agreed** 2 November 2026: `d3-lite` sorts and decides, Gemma 4 writes,
no cloud decision provider and no fallback between the two models.
Nothing in the app has changed yet.

---

## The answer

- **Default local decision model: vLLM-SR `d3-lite`.**
  0.85B parameters, a 1.1 GB file, Apache-2.0.
  It runs in normal llama.cpp (`/v1/systemone`).
- **Yes, add it to Moss's first AI version, but keep it narrow.**
  Seven duties: split, needs reply, deadline, expects a reply,
  typed or spoken words → command, signature choice,
  and "expected or unsolicited?" for new senders.
  See [What the decision model does in Moss](#what-the-decision-model-does-in-moss).
- **Two models, two jobs. No fallback between them.**
  - `d3-lite` **sorts and decides**: it picks from a fixed list of answers.
  - Gemma 4 **writes**: summaries, drafts, and the agent loop.
    The agent picks its own tools (Slack, GitHub…) as part of writing.
  - Gemma does not stand in for `d3-lite`, and `d3-lite` does not stand in for Gemma.
- **No cloud decision provider.** `d3-lite` runs locally, privately and fast enough
  on every laptop Moss targets. Cloud services are listed below for reference only.
- **Before `d3-lite` is downloaded** (or with AI turned off), Moss sorts with
  plain rules: the provider's own categories and the email's headers.
- **Some checks need no model at all.** Leave spam to the mail provider.
  Check "mentions an attachment but none attached" with a keyword rule.

Why `d3-lite` and not just Gemma:
in my test they were **equally accurate** (within the noise of 40 emails),
but `d3-lite` was **2.6× faster on Apple GPU, about 4× faster on CPU,
and used about 2.5× less memory.**
On a laptop without a GPU, Gemma took **1.7 s per question**.
That is too slow for a keyboard-first palette, and for every incoming email.

Before release: re-run the test on a bigger labelled set (300+ emails).
The test scripts are ready (see [How I tested](#how-i-tested)).

---

## What a decision model is

- You give it **some text** (the "state") and **typed questions**.
- Question types: **choice** (pick one), **noul** (yes/no), **score** (a scale).
- It returns a **probability for every option**. It writes no text.
- It cannot invent an answer outside your options.
- TypeSafe started the category with **Jev** on 15 September 2026.
  Its request shape (`POST /v1/systemone`) is now the common standard.
  llama.cpp, Ollama, OpenRouter and most open models use it.

Plain words: it is a classifier you set up with words, not training.

---

## What the decision model does in Moss

Worked out on 2 November 2026 from three things:
what `d3-lite` can do (its model card, the llama.cpp server docs, TypeSafe's docs),
how ten other mail apps sort mail (links under "How others do it" in [Sources](#sources)),
and what Moss already has (all sorting goes through `classify()` in `crates/moss-store`).

### Ground rules

1. **Rules first, model second.** Headers settle easy cases for certain:
   a calendar invite, a `List-Unsubscribe` header, a `no-reply@` sender.
   The model answers only what the rules can't, and Moss asks only the questions an email needs.
2. **Your corrections win.** Moving one email fixes that email.
   Moving a sender's mail again fixes the sender.
   (Apple Mail only lets you fix whole senders, and users complain about it.)
3. **Every decision says why:** "Rule: mailing list" or "AI, 82% sure".
   Users praise Gmail and Shortwave for showing a reason.
4. **Unsure means don't act.** Below the threshold, Moss keeps the rule's answer and shows it as a guess.
   Set thresholds on labelled mail: `d3-lite`'s probabilities are "not guaranteed to be calibrated" (llama.cpp docs).
5. **It sorts, flags and suggests. It never archives, deletes or sends.**
6. **It decides when Gemma runs.** Gemma drafts reply options only for mail that needs a reply,
   so most mail never wakes the big model. That saves memory and battery.

### Its duties in the first AI version

| # | Duty | Question | Asked about | What it changes in Moss | Who else does it, and how |
|---|---|---|---|---|---|
| 1 | **Split** | pick one of 5, each described in words | new mail the rules didn't settle | improves `classify()` | Gmail, Apple, Outlook: ML; Superhuman, Fyxer: cloud LLM |
| 2 | **Needs your reply** | yes/no | new mail in Important | replaces the "sent to me directly" rule | Superhuman, Fyxer, Gmail AI Inbox: cloud LLM |
| 3 | **Has a deadline** | yes/no, then a date parser finds the date | Important and Receipts | the amber due chip | Gmail AI Inbox to-dos: cloud LLM |
| 4 | **Expects a reply** | yes/no | mail you send | offers "Remind me if X doesn't reply" by itself | Gmail nudges, Superhuman, SaneBox: thread state + a check |
| 5 | **What command?** | pick one of the commands, or "none" | palette text and voice, only when fuzzy match or the voice grammar find nothing | typed or spoken words → command | nobody does this locally |
| 6 | **Signature** | yes/no "does this need a signature?", then pick one of yours | the composer, when the account's rules leave a choice | "Let Moss choose" signatures | none found |
| 7 | **Expected or unsolicited?** | yes/no "did the user ask for this mail?" | only new senders (code finds them) | hides the "Block this sender?" banner on mail you asked for, like a receipt or sign-in code, but only when the model is very sure | Spark Gatekeeper and HEY Screener ask about every new address; Superhuman auto-archives "cold pitches" with a cloud LLM |

Cost: duties 1–3 are at most 3 questions per email, about 42 ms each on an Apple GPU
and 0.4 s each on CPU in my test. Duties 4–6 run only when you write or type.
Duty 7 runs only for new senders. It is untested: add new-sender mail to the 300+ email set
before trusting it to hide any banner.

### Next wave (once the first set works on real mail)

- **Your own splits in plain words**, e.g. "Anything from investors about the round".
  Each one is a yes/no question. Superhuman (10 at most, Business plan only), Shortwave and
  Notion Mail do this with cloud LLMs. Moss can do it locally, with no limit.
- **Notify me now?** An urgency score (1–4) decides whether a new email interrupts you,
  and orders the Reply section. I found no other mail app that decides this.
- **Search routing:** is a query keywords, a person, a date range, or a question for Gemma?

### Not its job

| Job | Who does it instead | Why |
|---|---|---|
| Spam, phishing | the mail provider | `d3-lite` flagged 11 normal emails as spam in my test |
| Mentions an attachment, none attached | keyword rule + attachment list | models were unreliable in my test |
| Newsletters, calendar invites, robot senders | rules on headers (already in `classify()`) | certain and free |
| "No reply yet" | code: who wrote last, and how long ago | thread state, not a judgement |
| "Is this a new sender?" | code: you've never written to them, they're not in a thread you started, and they're not at your organisation's domain | a fact from your own mail, instant and certain; the model can't see your history |
| Blocking a sender | you, from the banner; unblock from the Blocked list | the model never blocks anyone |
| Summaries, the "ask" line, reply drafts, the agent and its tool choice | Gemma 4 | they need written text |
| Auto-archive | nobody in the first version | an unseen wrong archive is the worst mistake |

### Limits to design around

- **One email per request, and every question is its own pass** through the model.
  Cost grows with the number of questions, so skip any the rules already answered.
- **The whole prompt must fit in one batch** (d3 reads both ways).
  Send the headers plus the new text without quoted replies (moss-core already strips them),
  cut to about 1,500 tokens, and run with `-ub 2048`.
- **Question types:** pick one (up to 255 options), yes/no, and score (2–10 levels).
  No multi-label, ranking or extraction. For several labels, ask one yes/no per label.
- **Describe every option in words.** Bare labels route worse (llama.cpp blog, TypeSafe docs).
  TypeSafe warns that "none of the above" can work poorly.
  In the voice research, `d3-lite` still got 7 of 8 non-commands right.
- **Languages aren't stated** by the vendor. It sorted all 6 non-English emails right in my test.
  Check again on the bigger set.
- **First sync:** sort only recent mail (say, the last 30 days plus anything unread),
  newest first, in the background. Older mail keeps the rules' answer.
  2,000 emails × 2 questions ≈ 27 minutes on CPU, about 3 minutes on an Apple GPU.

---

## The candidates

✅ good for Moss · ⚠️ a problem · ❌ rules it out.
"Vendor" marks a number the model's maker published. I could not check it.

### Open-weight models that fit an 8–16 GB laptop

| Model | Maker | Size (file) | Licence | Runs in llama.cpp? | Decision Index ¹ | Notes |
|---|---|---|---|---|---|---|
| **d3-lite** | vLLM Semantic Router | 0.85B (1.1 GB Q8) | ✅ Apache-2.0 | ✅ yes, mainline | 26.2 | Best small model in my test. Text, image, video. |
| d3-nano | vLLM Semantic Router | 2.2B (2.6 GB Q8) | ✅ Apache-2.0 | ✅ yes | 34.3 | Not better than d3-lite in my test. |
| d3-mini | vLLM Semantic Router | 4.5B | ✅ Apache-2.0 | ✅ yes | 50.0 | Not tested (time). Next one to try for a "large" tier. |
| Kev-4B | Jared Palmer | 4B (3.0 GB Q4) | ✅ Apache-2.0 | ✅ yes | 34.9 | Accurate and well calibrated in my test. English only. Slow on CPU. |
| Kev-0.8B | Jared Palmer | 0.8B (0.8 GB Q8) | ✅ Apache-2.0 | ✅ yes | 16.3 | Weak on splits. |
| lev | Interfaze | 4B (3.0 GB Q4) | ✅ Apache-2.0 | ✅ yes | 35.5 | Most accurate in my test, but 0.5 s per question even on GPU. |
| Laya | ConvAI Innovations | 0.42B (0.45 GB Q8) | ✅ Apache-2.0 | ✅ yes | 9.2 | Weak. Fails outside English (independent paper ²). |
| Julia-1 | Supersonic Labs | 0.14B (0.17 GB Q8) | ✅ Apache-2.0 | ✅ yes | 4.8 | Tiny and fast, but near chance in my test. |
| d1-3B, d1-omni-600M | Liquid AI | 3.1B / 0.6B | ❌ LFM licence: free only under $10M yearly revenue | ✅ yes | 31.1 / 11.8 | Good in my test, but the licence doesn't fit an MIT app. |
| Perplexity Decider Flash v1.2 | Perplexity | 4.5B (5.2 GB Q8) | ✅ Apache-2.0 | ⚠️ needs custom code (its "protocol v3") | 54.3 | Best small model from a big maker. Stock llama.cpp can't run it yet. |
| Strands Decider 2B | AWS Strands Labs | 1.9B | ✅ Apache-2.0 | ❌ no GGUF; PyTorch or Ollama MLX (Mac only) | 20.7 | Doesn't fit Moss's runtime. |
| Nimble 9B v3 | Bespoke Labs | 9B | ❌ CC BY-NC 4.0 (older v1 was Apache) | ✅ yes | 51.1 | Non-commercial licence; too big. |
| StartLux-Decision 4B | StartLux | 4.5B | ❌ CC BY-NC 4.0 | ✅ GGUF | 54.8 | Non-commercial licence. |
| KnowLine 4B Gen5.5 | PelaAI | 4.5B | ✅ Apache-2.0 | community GGUF | 56.0 | Top small entry, but a tiny maker (≈560 downloads). Watch it. |

### Too big for the default (but open)

| Model | Size | Licence | Decision Index ¹ | Note |
|---|---|---|---|---|
| Cloudflare Clef-Flash | 9B (6.7 GB Q4) | Apache-2.0 | 44.4 | Runs in llama.cpp. Below the 4B models above. |
| Cloudflare Clef | 27B | Apache-2.0 | 54.9 | Server-class. |
| OpenJev | 27B | CC BY-NC 4.0 | None | Non-commercial. |

### Cloud only (reference; not planned for Moss)

| Service | Price (vendor) | Data | Decision Index ¹ | Note |
|---|---|---|---|---|
| **TypeSafe Jev 1.14** | $0.042 per million input tokens; output free | US hosting; no training on your data; zero retention only for enterprise | 60.2 | Most used (≈1 billion requests a week on OpenRouter). Defines the API. |
| OpenAI Decisions API (gpt-6-luna) | $0.10 per million input tokens (press reports) | OpenAI terms | not listed | Different API shape (`/v1/decisions`). I found no primary OpenAI page. |
| Perplexity Decider v1.2 (27B) | via OpenRouter | provider terms | 68.0 | Weights are also open (Apache-2.0), but 27B. |
| Microsoft-Decision-1 | Azure Foundry | Azure terms | not listed | Weights not released. No published benchmarks. |
| OpenRouter decision models | varies | goes through OpenRouter too | None | OpenRouter lists 21–30 decision models from 13 makers (counts differ by date). |

¹ **Decision Index 0.4.1** (text board, 29 Oct 2026), "Full" score.
0 = random guessing, 100 = perfect.
It is community-run by a Hugging Face staff member and is
"not affiliated with TypeSafe AI". The maintainers re-run submissions.
Its tests are general (knowledge, language, tools), not email.
[Board](https://huggingface.co/spaces/multimodalart/jev-decision-index) ·
[data](https://huggingface.co/datasets/multimodalart/decision-index-leaderboard) ·
[kit](https://github.com/apolinario/decision-index).

² [arXiv:2610.02267](https://arxiv.org/abs/2610.02267).

---

## My hands-on test

40 emails I wrote and labelled myself (no real user data).
16 Important, 7 Other (3 of them spam), 6 Newsletters, 6 Receipts, 5 Calendar.
6 are not in English (German, Persian, Portuguese, French, Japanese, Spanish).
Plus 20 command-palette phrases for 9 Moss commands.

Machine: MacBook with Apple M4 Max, 36 GB. llama.cpp 0.8.0 (Homebrew).
"GPU" = Metal. "CPU" = 4 threads, no GPU: closer to a Windows or Linux laptop,
but still faster than most of those.

### Accuracy (%)

Always guessing the most common answer gives:
split 40, needs reply 70, deadline 70.

| Model | Split | Needs reply | Deadline | Palette | Split, non-English (6) |
|---|---|---|---|---|---|
| **d3-lite** (0.85B) | **88** | 85 | 92 | 95 | 6/6 |
| Gemma 4 E2B, letter scoring (baseline) | **88** | 78 | 92 | 95 | 6/6 |
| lev (4B) | **88** | **90** | **98** | 95 | 6/6 |
| Kev-4B | 82 | **90** | 95 | **100** | 5/6 |
| d1-3B ⚠️ licence | **88** | 85 | 95 | **100** | 6/6 |
| d3-nano (2.2B) | 78 | 85 | 95 | 95 | 5/6 |
| d1-omni-600M ⚠️ licence | 80 | 75 | 90 | 95 | 5/6 |
| Laya | 68 | 57 | 78 | 75 | 3/6 |
| Kev-0.8B | 62 | 78 | 85 | 90 | 3/6 |
| Julia-1 | 48 | 50 | 60 | 55 | 4/6 |

With only 40 emails, **one email = 2.5 points**.
Gaps under about 10 points could be luck.

### Speed and memory

Median time. "Per question" = one split question for one email.
"Per email" = all 6 questions I asked about one email.

| Model | GPU per question | GPU per email | CPU per question | CPU per email | Memory used (GPU / CPU) |
|---|---|---|---|---|---|
| **d3-lite** | **42 ms** | 191 ms | **0.40 s** | 2.2 s | **1.4 / 2.0 GB** |
| Gemma 4 E2B (Q4_K_M) | 110 ms | 526 ms | 1.72 s | 7.2 s | 3.6 / 5.0 GB |
| Kev-4B (Q4_K_M) | 166 ms | 530 ms | 2.61 s | 5.8 s | 3.7 / 6.6 GB |
| lev (Q4_K_M) | 521 ms | 1,009 ms | not run | not run | 3.6 GB |
| Julia-1 | 7 ms | 20 ms | 0.04 s | 0.17 s | 0.4 GB |

Load time from disk was 0.5–4 s for every model.

### Calibration (are the probabilities honest?)

- **Gemma is overconfident.** When it was wrong about the split,
  it still gave the wrong answer **98%**. So you can't use its
  probability to decide "ask the user" without fixing it first
  (one temperature fitted on labelled mail would fix most of this).
- **Decision models were more honest.** When wrong, `d3-lite` gave the
  wrong split 66% on average, Kev-4B 62%.
- Pooled calibration error (lower is better, ~260 answers per model):
  lev 0.06, d1-3B 0.07, Kev-4B 0.08, d1-omni 0.12, Gemma 0.15, d3-lite 0.16.
- Independent papers agree that calibration is shaky:
  Jev's "95%" holds in only 8 of 51 languages
  ([arXiv:2610.36311](https://arxiv.org/abs/2610.36311)),
  and a plain Qwen2.5-3B said "100% sure" on every answer, right or wrong
  ([arXiv:2604.02367](https://arxiv.org/abs/2604.02367)).

### Where models failed

- **Spam:** too few spam emails (3) to judge.
  `d3-lite` flagged 11 normal emails as spam (mostly newsletters).
  Gemma flagged 1, Kev-4B and lev flagged none.
  → Don't use `d3-lite` for spam.
- **Missing attachment:** asked as one question, `d3-lite` said "yes" to
  almost every email. lev and d1-3B got it right.
  Split into "does it mention an attachment?" + code checks the list,
  most models still missed both cases.
  → Use a keyword rule ("attached", "enclosed", "anbei", "ci-joint"…).
- **Hard split cases every model missed:** an email asking to resend an
  invoice (said "Receipts"), an HR "confirm your benefits" email
  (said "Other"), a scam letter from a person (said "Important").
  These need context Moss has (contacts, past replies), not a bigger model.

---

## Runtime fit

Moss plans to embed llama.cpp through Rust bindings and download Gemma 4 on demand.

- **Good news:** mainline llama.cpp supports most open decision models
  since 2 October 2026 (`/v1/systemone`, [PR #29818](https://github.com/ggml-org/llama.cpp/pull/29818)).
  v0.7.0 added Liquid d1, vLLM d3 and Perplexity Decider (v1 protocol).
  ggml-org publishes ready GGUF files
  ([collection](https://huggingface.co/collections/ggml-org/decision-models)).
- **Catch:** the decision logic lives in **llama-server**
  (`tools/server/server-decision.cpp`, about 840 lines), not in the
  `libllama` C API. The Rust crate `llama-cpp-2` has no decision support.
- So Moss has two options:
  1. **Run `llama-server` as a Tauri sidecar** on localhost. ✅ Recommended.
     New decision models work with each llama.cpp update, for free.
     Cost: about 7 MB more binary (the server part), plus the shared libraries Moss
     already ships for Gemma. Whole Homebrew llama.cpp install = 21 MB.
  2. Port the readout for the one model we pick into Rust. Less binary,
     but we own the code and lose new models for free.
- Decision models need `--cache-ram` to reuse the email between questions.
  It is off by default for them. On CPU it cut `d3-lite` from 2.2 s to 1.8 s per email.

---

## Why not let Gemma sort mail too?

Gemma could answer the same questions (I tested it that way as the baseline),
but its job in Moss is writing. Sorting is a separate job with a separate model.

The honest case for Gemma doing both:

- Same accuracy in my test. No extra download. No new runtime code.
- The Decision Index shows that **prompting a plain model well can beat
  trained decision models**: "inference technique" entries on stock
  Gemma 4 31B score 61–68, above Jev 1.14 (60.2).
  So "decision model" is partly a way of asking, not magic weights.

The case for a separate `d3-lite`:

- **Speed on CPU.** 0.4 s vs 1.7 s per question. Windows and Linux
  laptops often have no usable GPU. A keyboard-first palette can't wait 1.7 s.
- **Volume.** First sync of 2,000 emails × 3 questions:
  about 40 minutes with `d3-lite` on CPU, about 2–3 hours with Gemma.
- **Memory.** 1.4–2 GB vs 3.6–5 GB, resident all day. That matters on 8 GB.
- **Earlier start.** A 1.1 GB download can sort mail before the
  3 GB Gemma download is done.
- **More honest probabilities** out of the box.
- **Clear jobs.** Sorting never waits behind a long Gemma draft,
  and Gemma doesn't need to stay loaded just to sort mail.

---

## Risks

- **The category is 7 weeks old.** Models change every week
  (vLLM-SR went from Decision 2.0 to 3.0 in under a month).
  → Build the interface (state + typed questions → probabilities),
  make the model a swappable download, and keep the test harness.
- **Most numbers are vendor claims.** Independent papers so far:
  - Jev beat Laya on 9 of 11 agent decisions, but neither beat chance
    on model routing ([arXiv:2610.02267](https://arxiv.org/abs/2610.02267)).
  - A fine-tuned DeBERTa classifier beat Jev on most labelled tasks;
    Gemma 4 31B matched Jev ([arXiv:2610.00346](https://arxiv.org/abs/2610.00346)).
  - Jev tied a schema-constrained LLM on clinical tasks, and was faster
    ([arXiv:2610.35721](https://arxiv.org/abs/2610.35721)).
  - The "front-door routing" benchmark tested plain small LLMs, not decision models.
    No 1–4B model reached 85% accuracy
    ([arXiv:2604.02367](https://arxiv.org/abs/2604.02367)).
- **My test is small and I wrote the labels.** It is a sanity check, not proof.
- **d3-lite's own claims** (vendor): 13.6 ms per text request on an AMD MI325X GPU,
  "first among entries of similar size" on Decision Index 0.3.2.
  On the newer 0.4.1 board, other ≈1B models score a bit higher (e.g. OneJev 0.8B, 29.6).
- **Licences move.** Nimble went from Apache to non-commercial between versions.
  Pin the exact file hash Moss downloads.

---

## Next steps (if agreed)

1. Define a Rust `Decide` trait: state + typed questions → probabilities.
   One back end: llama-server `/v1/systemone`. The trait keeps the model swappable.
2. Ship `llama-server` as a sidecar; download `d3-lite` Q8_0 on first AI setup.
   Until then, sort with rules (provider categories, `List-Unsubscribe`, calendar parts).
3. Grow the labelled set to 300+ emails (synthetic, plus team mail with consent).
   Re-run all models. Fit one temperature per question type.
4. Try `d3-mini` (4B) as an optional "more accurate" tier for 16 GB Macs.
5. Keyword rule for missing attachments. Use the provider's spam verdict.

---

## How I tested

- Scripts and the 40 emails are in the session notes folder
  (`decision-model-research/bench/` beside this group's other notes).
  They are not in the repo because this task was research only.
- Decision models: one `/v1/systemone` request per email with all questions.
- Gemma 4 E2B (`unsloth/gemma-4-E2B-it-GGUF`, Q4_K_M): one chat request per question,
  options shown as letters, `max_tokens: 1`, thinking off.
  Probabilities = softmax over the option letters in the top 20 log-probabilities.
- Files used: `ggml-org/*-GGUF` for d3-lite (Q8_0), d3-nano (Q8_0), Kev-4B (Q4_K_M),
  Kev-0.8B (Q8_0), lev (Q4_K_M), Laya (Q8_0), Julia-1 (Q8_0);
  `LiquidAI/d1-3B-GGUF` (Q4_K_M) and `LiquidAI/d1-omni-600M-GGUF` (Q8_0).
- Server flags: `-ub 2048 -b 2048 -c 8192`; CPU runs add `-ngl 0 -dev none -t 4 -tb 4`.

---

## Sources

Primary sources unless marked.

**Category and cloud**
- TypeSafe: [docs](https://docs.typesafe.ai/introduction), [models](https://docs.typesafe.ai/models.md), [launch blog](https://typesafe.ai/blog/introducing-system-one-models-and-jev), [Jev 1.14](https://typesafe.ai/blog/jev-1-14), [privacy policy](https://typesafe.ai/legal/privacy-policy), [own evals](https://evals.typesafe.ai/) (vendor)
- OpenRouter: [Jev guide](https://openrouter.ai/docs/guides/community/jev), [decision rankings](https://openrouter.ai/rankings/decisions)
- OpenAI Decisions API (press, no primary page found): [Unite.AI](https://unite.ai/openai-releases-decisions-api-in-public-beta-powered-by-gpt-6-luna), [ProPakistani](https://propakistani.pk/2026/10/07/openai-launches-decisions-api-with-up-to-10x-faster-responses/)
- Microsoft-Decision-1 (secondary): [OrcaRouter](https://www.orcarouter.ai/fr/blog/microsoft-decision-1-ga-foundry-no-published-benchmarks), [LLM Reference](https://www.llmreference.com/model/microsoft-decision-1)

**Models**
- vLLM-SR Decision 3.0: [d3-lite](https://huggingface.co/vllm-sr/d3-lite), [d3-nano](https://huggingface.co/vllm-sr/d3-nano), [d3-mini](https://huggingface.co/vllm-sr/d3-mini)
- [Kev-4B](https://huggingface.co/jaredpalmer/kev-4b), [lev](https://huggingface.co/interfaze-ai/lev), [Laya](https://huggingface.co/convaiinnovations/laya), [Julia-1](https://huggingface.co/SupersonicLabs/Julia-1)
- Liquid AI: [d1-3B](https://huggingface.co/LiquidAI/d1-3B), [LFM licence](https://www.liquid.ai/lfm-license)
- Perplexity: [Decider Flash v1.2](https://huggingface.co/perplexity-ai/pplx-decider-flash-v1.2), [GGUF and runtime note](https://huggingface.co/perplexity-ai/pplx-decider-flash-gguf-v1.2)
- Cloudflare: [launch blog](https://blog.cloudflare.com/clef-decision-models), [Clef-Flash](https://huggingface.co/Cloudflare/clef-flash)
- Strands: [GitHub](https://github.com/strands-labs/strands-decider), [Ollama](https://ollama.com/library/strands-decider)
- Bespoke Nimble: [9B](https://huggingface.co/bespokelabs/Bespoke-Nimble-9B), [9B v3 (CC BY-NC)](https://huggingface.co/bespokelabs/Bespoke-Nimble-9B-v3)
- Gemma 4 E2B GGUF: [unsloth](https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF)

**What d3 can do**
- [llama.cpp server README, `/v1/systemone`](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md), [PR #30383 (d3 labels)](https://github.com/ggml-org/llama.cpp/pull/30383)
- TypeSafe question types: [choice](https://docs.typesafe.ai/primitives/choice.md), [noul](https://docs.typesafe.ai/primitives/noul.md), [score](https://docs.typesafe.ai/primitives/score.md), [advanced](https://docs.typesafe.ai/primitives/advanced.md)
- [vLLM recipe for d3-lite](https://recipes.vllm.ai/vllm-sr/d3-lite)

**How others do it**
- Gmail: [importance markers](https://support.google.com/mail/answer/186543), [nudges](https://support.google.com/mail/answer/6585), [Purchases](https://blog.google/products/gmail/one-stop-purchase-tracking-in-gmail/), [AI Inbox](https://www.androidcentral.com/apps-software/gmail/google-announces-major-gmail-gemini-powered-upgrades) (press)
- Apple Mail: [Six Colors review](https://sixcolors.com/post/2024/12/ios-18-2-mail-is-a-misfire/), [heise](https://heise.de/-10227127), [Apple forum: Priority](https://discussions.apple.com/thread/256210627)
- Outlook: [Focused Inbox](https://support.microsoft.com/help/f714d94d-9e63-4217-9ccb-6cb2986aa1b2), [Copilot "Prioritize" retirement, MC1519861](https://mc.merill.net/message/MC1519861)
- Superhuman: [AI launch](https://blog.superhuman.com/the-next-superhuman-ai/), [Auto Labels help](https://help.superhuman.com/hc/en-us/articles/40127432866323)
- [HEY features](https://www.hey.com/features/) · Spark: [inbox help](https://sparkmailapp.com/help/manage-your-inbox/customize-your-inbox), [AI press release](https://readdle.com/press/release/4647)
- Shortwave: [labels](https://shortwave.com/docs/guides/labels), [custom bundles](https://www.shortwave.com/blog/magic-labels-custom-bundles) · [Notion Mail](https://techcrunch.com/2025/04/15/notion-releases-its-ai-driven-email-inbox) (press)
- Fyxer: [categories](https://support.fyxer.com/en/articles/13107658-fyxer-email-categorization-handbook), [G2 reviews](https://www.g2.com/products/fyxer/reviews) · [SaneBox](https://sanebox.com/help/155-how-does-sanebox-work)
- Local filters: [Thunderbird junk filter](https://support.mozilla.org/en-US/kb/thunderbird-and-junk-spam-messages), [Fastmail spam filter](https://www.fastmail.help/hc/en-us/articles/360060591413-Spam-filtering)

**Runtime**
- llama.cpp: [PR #29818 (/v1/systemone)](https://github.com/ggml-org/llama.cpp/pull/29818), [PR #29831 (Clef)](https://github.com/ggml-org/llama.cpp/pull/29831), [v0.7.0 notes](https://newreleases.io/project/github/ggml-org/llama.cpp/release/v0.7.0) (mirror), [HF blog](https://huggingface.co/blog/ggml-org/decision-models-in-llamacpp), [GGUF collection](https://huggingface.co/collections/ggml-org/decision-models)
- Walk-through of the server code (secondary): [thesatyajit](https://ai.thesatyajit.com/articles/llamacpp-decision-models)
- [Ollama decision-model support](https://ollama.com/blog/ollama-now-supports-decision-models)

**Independent evaluations**
- [arXiv:2610.02267](https://arxiv.org/abs/2610.02267): Fast Models, Slow Evidence (Laya vs Jev, agent harnesses)
- [arXiv:2610.00346](https://arxiv.org/abs/2610.00346): System One models vs trained classifiers (Texas State University)
- [arXiv:2610.36311](https://arxiv.org/abs/2610.36311): Confidence Is Not a Promise (calibration across 51 locales)
- [arXiv:2610.35721](https://arxiv.org/abs/2610.35721): Jev for clinical trial criteria
- [arXiv:2604.02367](https://arxiv.org/abs/2604.02367): Small LMs for front-door routing (author's company built the system tested)
- [Decision Index](https://huggingface.co/spaces/multimodalart/jev-decision-index): community leaderboard
