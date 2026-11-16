# Apple Intelligence as a writing model

Examples and quoted outputs in this document are anonymized. Historical measurements used the original inputs, some of which included private correspondence. The replacement synthetic fixtures have not been rerun with real models.
7 November 2026. Can Moss write with Apple's own on-device model, so a Mac that has it needs no 3 to 5 GB Gemma download? Yes, on macOS 27.

## What Apple offers

- **The Foundation Models framework** (Swift only) runs Apple's on-device model. On this Mac (M-series, macOS 27.0.1) it reports `available` and a context of **8,192 tokens** (Gemma 4 runs with 32,768 in Moss).
- **macOS 27 adds what an agent needs**: a transcript Moss can build from its own chat history (tool calls and their results included), tools defined at run time from a JSON schema (`GenerationSchema` reads JSON schema with `title`, `x-order` and `required` on every object), and `GenerationOptions.toolCallingMode = .required` for the forced calls Moss uses to pick, review and find.
- **`fm serve`** (in macOS 27) serves the model over an OpenAI-style endpoint, but it doesn't pass tools on: the model never called one, and `tool_choice: "required"` failed with "An unsupported generation guide was used". So Moss doesn't use it.
- Through Swift, with tools: it searched first and then marked the right email Done 4 of 4 times, in 1.5 to 2 seconds each.

## How Moss uses it

`moss-apple` (in `apple/`, Swift) is a small server that takes llama-server's flags and serves Apple's model on the same `/v1/chat/completions` endpoint, streamed, behind the run's key. Moss starts it the way it starts llama-server (`manager::Ai::writer`), so the Rust side talks to either the same way.

- Each request becomes a transcript: system messages are the instructions (with the tool definitions), earlier turns are prompts, responses, tool calls and tool outputs, and the last user message is the prompt. A chat that ends in a tool's result goes on with an empty prompt, which works.
- A tool the model calls is written down and the answer stops there: Moss runs the tools, not the helper. Calls made together (Done on two emails) come back together.
- `moss-apple --available` says whether the model can run; Moss asks once at start (`apple::detect`) and lists Apple Intelligence first in Settings, AI only when it can. It's built by `src-tauri/build.rs` and shipped beside Moss as Tauri's `externalBin` (`tauri.macos.conf.json`).
- On a Mac with it, it's the recommended writer for a new setup, and the recommended download is sorting and search only (about 1.4 GB). A Mac already set to a Gemma model keeps it.

## The smaller context

Every budget set for the Gemma writer's 32k is scaled to the model's context with `chat::room` (a quarter for Apple's 8k): the open email in Ask Moss, a thread or file Ask Moss reads, a tool's result, and the conversation Quick reply reads (`reply::Context::within`).

Ask Moss's tools alone are about 13,000 characters: the full set would take half of Apple's context. When the tools would take more than a quarter of it, the model first picks the tools a request needs (`agent::offered`), beside the ones that find and read mail, which are always on offer. When a review says something is left, it picks again for that. A Gemma model's context has room for all of them, so it never picks.

## Results

`src-tauri/tests/real_actions.rs`, the 10 checks on the made-up mailbox (`MOSS_WRITER=apple MOSS_APPLE=apple/.build/release/moss-apple`):

| Run | Passed | Misses |
|---|---|---|
| First, as built | 7 of 10 | the three newsletters Done, a reminder (it set and then stopped it), a reply set to go later |
| Forwards kept as written, `list_inbox` always on offer, the writer told to add nothing of its own | 8 of 10, twice | the newsletters (it finds two of three), the reply to Dana (it wrote a new email to her, not a reply) |
| write_email says what kind of email it wrote, the review checks it, no "Moss adds" in the sign-off rule | 7 of 10 | the newsletters, the reply to Dana (the review sent it back, and it wrote a new email again), a "[Your Signature]" placeholder it carried into the email |

| write_email answers the model with what it said (which kind of email, not to write it out), not with the email Quick reply's writer made of its note | 9 of 11 (two new checks: the answer doesn't write the email out again) | the newsletters, the reply to Dana (set to go, but the writer signed it "[Your Signature] Alex") |

- **The email twice**: in the app the answer wrote the whole draft out above the card that already shows it. The cause was the tool's answer, not the model: once Quick reply's writer had turned the model's note into the email, the model heard update_draft's result (the whole email) in place of write_email's note (what kind of email, and that Moss shows it under the answer). Telling it not to, in the prompt, the note and the review, changed nothing until it stopped getting the email back. Now it says in a sentence what it wrote, sometimes quoting its gist.
- **Speed**: each model call takes 0.4 to 4 seconds; an answer that acts takes 5 to 15 seconds, one that writes an email 12 to 20. Gemma 4 E4B takes 25 to 30 seconds to write a reply.
- **Writing**: Quick reply's writer, on Apple's model, padded short notes with sentences of its own ("I've confirmed the meeting time with the legal team"). Its prompt now says, beside the note, to add no news, plans or promises the note and the conversation don't give; after that it still adds a closing line or two, but no made-up facts.
- **Where it's weaker than E4B**: going through every email of a kind (it acts on the ones the first search found), and choosing a reply over a new email. write_email now says which it wrote, and the review checks for it.

## Writing from a short note (8 November)


The app asked "Can you draft an email to [recipient] telling him that I will text him soon?" and the card read "Hi [recipient] / I will text you soon. / I will send the message as requested. / I will keep you updated when it's ready." Ask Moss's own words were fine ("I'll text you soon."); Quick reply's writer, which turns them into the email, padded them. Quick reply itself did the same. Run it with `cargo test -p moss-ai --test real_short_replies -- --ignored --nocapture` (`MOSS_WRITER=apple MOSS_APPLE=...`).

What caused it, in the writer's prompt:

- It asked for "a complete email" that answers "anything else in their message the note doesn't settle", a closing sentence, and "what happens next", and to "say you'll confirm" a missing detail. Apple's model filled each with a line of its own.
- It told the model it drafts for the user, with "you" meaning the user. The model mixed up who said what and wrote the recipient's own words back to him ("I can visit next week to review the project").
- A new email had no conversation, so nothing said who it was for: one began "Hi Maya", to the user.

What fixed it: the model is the user, writing to someone named ("You are replying to [recipient], who wrote the latest message" or "You are writing a new email to [recipient]"); the note is what the email says, as long as it needs and keeping its own facts; a rule against repeating them or adding lines about following up or looking forward; no closing sentence. A new email gets the person it's to and what the user wrote them before, as a reply does. Quick reply's choices say, in the first person, what you say back.

Before and after, on Apple:

| | Before | After |
|---|---|---|
| Ask Moss, the [recipient] email | 5 lines of filler, greeting "Hi Maya" | "Hi [recipient], / I'll text you soon." |
| Reply to [recipient] from "I'll text you soon" | 35 words, repeats his offer, leaves the note out | the note, 7 words; in some runs his offer again after it |
| Quick replies | 31 to 49 words, the recipient's words as yours, closing filler | 9 to 20 words, one or two sentences |

What Apple's model is sensitive to: moving today's date out of the top of the message, or the `---` lines between messages, brought back the repeated offer in every run, so both stay. A second model call to check the email against the note (pick lines to drop, or label each line) either dropped the wrong lines or kept everything, and cost 10 seconds, so there's none. Left over, now and then: a `<p>` tag, a `---` line above the name, the date written in as a line, "by Friday" where the note said "tomorrow".

## Not done yet

- Apple Intelligence turned on after Moss started shows up the next time Moss starts.
- Sorting (d3-lite) and search by meaning (EmbeddingGemma) still download: Apple's model gives no probabilities for sorting, and macOS's sentence embeddings would need their own work.
