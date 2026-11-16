//! AI replies, written by the writing model in your voice: quick replies to pick from, a reply
//! that says what you tell it, and rewrites of what's in the composer.
//!
//! The AI only ever writes text for the composer, where you change it as you like. It never
//! sends and never marks mail Done. It leaves out the signature: the composer adds yours.

use moss_core::{Person, Thread};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::chat::{ChatModel, ChatRequest};
use crate::{Ai, AiError};

/// What the writing model reads to reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    pub subject: String,
    /// Oldest first: who wrote, when (milliseconds since the epoch), and their new text.
    pub messages: Vec<(Person, i64, String)>,
    /// You, as the account replying: the name you send as and the address.
    pub me: Person,
    /// What you wrote to the same person before, newest first, so it sounds like you.
    pub past: Vec<String>,
    /// Now, in milliseconds since the epoch, so the model knows what day it is.
    pub now: i64,
    /// The lines of the signature added below the reply; none when it gets none.
    pub signature: Vec<String>,
    /// Who a new email is for, when it isn't a reply: with no conversation, the model has no
    /// other way to know whom it writes to.
    pub to: Option<Person>,
}

/// How the email ends, which depends on the signature added below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// No signature: a sign-off, then your first name.
    SignOffAndName,
    /// A signature that starts with your name: a sign-off, then nothing.
    SignOff,
    /// A signature with its own sign-off ("Best" then your name): the email ends with its body.
    Nothing,
}

impl Ending {
    /// The ending for `signature`, sent as `me`. Its first line is a sign-off when it is short,
    /// isn't your name and isn't contact details.
    pub fn for_signature(signature: &[String], me: &Person) -> Ending {
        let Some(first) = signature.iter().map(|l| l.trim()).find(|l| !l.is_empty()) else {
            return Ending::SignOffAndName;
        };
        let line = first.trim_end_matches([',', '!', '.']).to_lowercase();
        let name = me.name.trim().to_lowercase();
        let yours = !name.is_empty() && (line == name || Some(line.as_str()) == name.split_whitespace().next());
        let contact = line.contains(['@', ',', '.', '/', ':']) || line.chars().any(|c| c.is_ascii_digit());
        if yours || contact || line.split_whitespace().count() > 3 {
            Ending::SignOff
        } else {
            Ending::Nothing
        }
    }
}

/// Characters of conversation the model reads, newest messages first. The writing model runs
/// with a 32,768-token window (see `manager`), and this is about 20,000 tokens of it at most,
/// leaving room for the instructions and the reply. Nearly every conversation fits whole.
const CONVERSATION_CHARS: usize = 70_000;
/// Characters of one message: only a very long one is cut.
const MESSAGE_CHARS: usize = 30_000;
/// Earlier emails of yours it learns your greeting, sign-off and length from, and characters of each.
const PAST: usize = 3;
const PAST_CHARS: usize = 4_000;

impl Context {
    /// A thread as the model reads it: its newest messages that fit, oldest first. `past` is
    /// what you wrote to the other person before, newest first; `now` is today.
    pub fn from_thread(thread: &Thread, me: Person, past: Vec<String>, now: i64) -> Context {
        let mut messages = Vec::new();
        let mut room = CONVERSATION_CHARS;
        for m in thread.messages.iter().rev() {
            let text: String = m.paragraphs.join("\n\n").trim().chars().take(MESSAGE_CHARS).collect();
            if text.is_empty() {
                continue;
            }
            let length = text.chars().count();
            if length > room {
                break;
            }
            room -= length;
            messages.push((m.from.clone(), m.sent_at, text));
        }
        messages.reverse();
        let past = past
            .iter()
            .map(|p| p.trim().chars().take(PAST_CHARS).collect::<String>())
            .filter(|p| !p.is_empty())
            .take(PAST)
            .collect();
        Context { subject: thread.summary.subject.clone(), messages, me, past, now, signature: vec![], to: None }
    }

    /// The context with at most `chars` of conversation: its newest messages that fit, each
    /// message and earlier email cut in the same measure. For a model with a smaller context
    /// than the Gemma writer's: see [`room`](crate::chat::room).
    pub fn within(&self, chars: usize) -> Context {
        let scaled = |most: usize| most * chars / CONVERSATION_CHARS;
        let mut room = chars;
        let mut messages = vec![];
        for (who, at, text) in self.messages.iter().rev() {
            let text: String = text.chars().take(scaled(MESSAGE_CHARS)).collect();
            let length = text.chars().count();
            if length > room {
                break;
            }
            room -= length;
            messages.push((who.clone(), *at, text));
        }
        messages.reverse();
        let past = self.past.iter().map(|p| p.chars().take(scaled(PAST_CHARS)).collect()).collect();
        Context { messages, past, ..self.clone() }
    }
}

/// What to write. Every job but [`Job::Instructions`] and the three answers rewrites your
/// draft; those write a reply when the draft is empty and rework it when it isn't.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Job {
    /// Your own words on what to say and how.
    Instructions { instructions: String },
    Friendlier,
    Formal,
    Shorter,
    Longer,
    /// Fix spelling and grammar, nothing else.
    Fix,
    /// Into another language, e.g. "French".
    Translate { language: String },
    /// Say yes to what they asked.
    Accept,
    Decline,
    /// Ask for more time.
    MoreTime,
}

/// A quick reply to pick: a short label ("Yes, Friday works") and the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyOption {
    pub label: String,
    pub draft: String,
}

/// The system instruction and user message for `job`, or `None` when it rewrites a draft
/// that is empty.
pub fn prompt(job: &Job, draft: &str, context: &Context) -> Option<(String, String)> {
    let has_draft = !draft.trim().is_empty();
    let reply = |what: &str| {
        if has_draft {
            format!("Rework your draft into a complete reply that {what}. Keep what your draft already says unless it contradicts that.")
        } else {
            format!("Write your reply. It {what}.")
        }
    };
    let task = match job {
        Job::Instructions { instructions } if has_draft => format!(
            "Rework your draft as your note says. Your note, which comes first over everything else: {}",
            instructions.trim()
        ),
        Job::Instructions { instructions } => format!(
            "Write your {}. Your note on what it should say, which comes first over everything else: {}\nSay what your note says and nothing more. Keep its words and facts where they read well in an email (a day it names stays that day), changing only what you must to say it to them yourself. The email is as long as the note needs: a note of a line or two makes an email of a sentence or two. Add no news, plans or promises of your own that the note and the conversation don't give.",
            if context.messages.is_empty() { "email" } else { "reply" },
            instructions.trim()
        ),
        Job::Accept => reply("says yes to what they asked, clearly, and answers any other question they asked"),
        Job::Decline => reply("says no to what they asked, kindly and warmly, showing you appreciate being asked, with no reason you haven't been given; if they asked other things, it still answers those"),
        Job::MoreTime => reply("asks for a little more time on what they asked, says briefly that you're on it, and promises no date or deadline you haven't been given"),
        _ if !has_draft => return None,
        // Rewrites keep your draft's language; the conversation's may differ.
        Job::Friendlier => "Rewrite your draft so it sounds warmer and friendlier. Keep everything it says, in the language it's written in.".into(),
        Job::Formal => "Rewrite your draft so it sounds more formal and professional. Keep everything it says, in the language it's written in.".into(),
        Job::Shorter => "Rewrite your draft shorter, in the language it's written in: keep every point that matters, the greeting and the sign-off, and cut the rest.".into(),
        Job::Longer => "Rewrite your draft a little fuller and warmer, in the language it's written in, without padding and without adding facts.".into(),
        Job::Fix => "Fix the spelling, grammar and punctuation of your draft. Change nothing else: keep its words, tone, language, greeting and sign-off.".into(),
        Job::Translate { language } => format!(
            "Translate your draft into {}. Keep its meaning, tone, greeting, sign-off and paragraphs. This one is in {0}, not in the language of the conversation.",
            language.trim()
        ),
    };
    // Said last, where a small model heeds it most.
    let language = match job {
        Job::Translate { .. } => String::new(),
        Job::Friendlier | Job::Formal | Job::Shorter | Job::Longer | Job::Fix => "\nKeep the language your draft is written in.".into(),
        Job::Instructions { .. } if context.messages.is_empty() => "\nWrite it in the language of your note.".into(),
        _ => {
            let whose = them(context).map(|p| format!("{}'s", first_name(p))).unwrap_or_else(|| "their".into());
            format!("\nWrite it in the language of {whose} latest message.")
        }
    };
    // A small model heeds what comes last: the note's own facts, kept.
    let keep = match job {
        Job::Instructions { .. } if !has_draft => "\nKeep every fact your note gives as it gives it: a day, time or amount in your note stays as it is.",
        _ => "",
    };
    let task = format!("{task}{language}{keep}");
    // Fixing and translating only need the draft.
    let conversation = !matches!(job, Job::Fix | Job::Translate { .. });
    Some((system(context), user(context, draft, &task, conversation)))
}

/// The system instruction and user message that choose 2 or 3 quick replies: what each says,
/// not the email itself, which is written on its own as plain text (models write prose worse
/// inside JSON).
pub fn options_prompt(context: &Context) -> (String, String) {
    let them = them(context).map(first_name).unwrap_or_default();
    let task = format!(
        "Choose 2 or 3 genuinely different replies you might send to {them}'s latest message, as the person replying would see the choice. \
If they ask for something, the choices are usually to agree, to decline or push back, and to ask for what's missing before deciding; \
if their message needs no answer, offer short acknowledgements instead. Leave out any choice that doesn't fit this conversation.\n\
For each, give:\n\
- \"label\": a button label of 2 to 4 words, written in the language of {them}'s latest message.\n\
- \"says\": what you say back to {them} in that reply, in one sentence in the first person, specific to this conversation, with names, days and amounts as the conversation gives them: your answer, not a retelling of what {them} wrote.\n\
Answer in JSON only."
    );
    (system(context), user(context, "", &task, true))
}

/// The other person: the latest sender who isn't you.
fn them(context: &Context) -> Option<&Person> {
    context.messages.iter().rev().map(|(p, _, _)| p).find(|p| !p.email.eq_ignore_ascii_case(&context.me.email))
}

/// One short paragraph, the way you'd answer a text.
fn note(text: &str) -> bool {
    let text = text.trim();
    !text.contains("\n\n") && text.split_whitespace().count() < 60
}

/// Whether to answer with a quick note rather than a whole email: only when their latest
/// message is a note and you write notes to them too, going by your latest message in the
/// conversation, or else your latest email to them. With nothing of yours to go by, it's an
/// email.
fn casual(context: &Context) -> bool {
    let mine = |p: &Person| p.email.eq_ignore_ascii_case(&context.me.email);
    let theirs = context.messages.iter().rev().find(|(p, _, _)| !mine(p));
    let yours = context.messages.iter().rev().find(|(p, _, _)| mine(p)).map(|(_, _, t)| t).or(context.past.first());
    theirs.is_some_and(|(_, _, t)| note(t)) && yours.is_some_and(|t| note(t))
}

/// "Sat 31 Oct 2026", in this Mac's time zone.
fn day(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms).single().unwrap_or_default().format("%a %-d %b %Y").to_string()
}

/// "Dana" from "Dana Wu"; the address's name when there's no name.
fn first_name(p: &Person) -> &str {
    p.name.split_whitespace().next().unwrap_or_else(|| p.email.split('@').next().unwrap_or(""))
}

/// How Moss writes as you, whatever the job: the shape of an email, described, never shown,
/// because small models copy whatever sample they're given. The model is you, writing to them,
/// so there is one "you" for it to keep track of: told it drafts for you, a small model
/// mixed up who said what and wrote their words back to them as yours.
fn system(context: &Context) -> String {
    let me = crate::prompt::sender(&context.me);
    let end = match Ending::for_signature(&context.signature, &context.me) {
        Ending::SignOffAndName => "3. A sign-off on its own line that fits the tone, then your first name on the line below it. The email ends with your name: no title, company or contact details.",
        Ending::SignOff => "3. A sign-off on its own line that fits the tone. The email ends there: nothing after the sign-off, not your name, title, company or contact details, because your saved signature is added below it automatically.",
        Ending::Nothing => "3. No sign-off and no name: the email ends with the body, because your saved signature, which has its own sign-off and your name, is added below it automatically.",
    };
    let them = them(context);
    let to = them.or(context.to.as_ref());
    let form = if casual(context) {
        "Their latest message is a quick, casual note, so your reply is one too: one to three relaxed sentences, the way you'd answer a text from someone you know. No greeting line and no sign-off.\n\n".to_string()
    } else {
        let name = to.map(first_name).filter(|n| !n.is_empty()).unwrap_or("the other person's first name");
        format!(
            "The email you write has these parts, in order:\n\
1. A greeting line: a greeting word and {name}, as formal or casual as the conversation, then a blank line.\n\
2. The body: what you're asked to say, in as few sentences as it takes, often one or two. Short paragraphs of one to three sentences, with a blank line between them.\n\
{end}\n\n"
        )
    };
    let whom = match (them, &context.to) {
        (Some(p), _) => format!(" You are replying to {}, who wrote the latest message.", crate::prompt::sender(p)),
        (None, Some(p)) => format!(" You are writing a new email to {}.", crate::prompt::sender(p)),
        (None, None) => String::new(),
    };
    format!(
        "You are {me}, writing an email yourself, in the first person.{whom}\n\
Write the way a real person writes to someone: plain, warm and to the point. Say what you're asked to say and stop: a short email that sounds like you beats a complete one.\n\n\
{form}Rules:\n\
- Every sentence says something new. Don't repeat or sum up what they wrote, don't say the same thing twice in other words, and don't add lines of your own about following up, confirming, keeping them posted or looking forward to things.\n\
- Answer what they asked only with what you're told or what the conversation says. When you don't have an answer, leave it out.\n\
- Language: write the whole email, greeting and sign-off included, in the language of their latest message, even when these instructions or your note are in another language, unless you're asked for another language.\n\
- Tone and length: match the conversation and how you usually write to them.\n\
- Start with substance after the greeting: no stock pleasantries or generic thanks unless something deserves thanks.\n\
- Time: the conversation gives today's date and when each message was sent. Anything planned for a day before today has already happened: write about it as done, never as still to come.\n\
- Never make up facts, dates, numbers, prices, places, reasons or promises that the conversation and your note don't give.\n\
- Write only the email: no subject line, no notes about it, no placeholders in brackets, no Markdown."
    )
}

/// What your earlier emails to them show about how you write to them: the greeting and
/// sign-off you use (the sign-off only when `sign_off`, as your signature may bring its own),
/// how long, and whether in lowercase. Never the emails themselves, which a small model would
/// copy.
pub fn voice(past: &[String], me: &Person, sign_off: bool) -> Option<String> {
    let name = me.name.trim().to_lowercase();
    let first = name.split_whitespace().next().unwrap_or("").to_string();
    let mut greetings = Vec::new();
    let mut signoffs = Vec::new();
    let mut words = 0;
    let mut lowercase = 0;
    for email in past {
        let paragraphs: Vec<&str> = email.split("\n\n").map(str::trim).filter(|p| !p.is_empty()).collect();
        let lines: Vec<&str> = email.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let mut body = lines.clone();
        // Your name at the end isn't part of how you write: the signature adds it.
        if body.last().is_some_and(|l| !name.is_empty() && (l.to_lowercase() == name || l.to_lowercase() == first)) {
            body.pop();
        }
        if paragraphs.len() > 1 {
            if let Some(g) = body.first().filter(|l| l.split_whitespace().count() <= 4) {
                greetings.push(g.to_string());
            }
            if let Some(s) = body.last().filter(|l| l.split_whitespace().count() <= 4 && body.len() > 2) {
                signoffs.push(s.to_string());
            }
        }
        words += body.iter().map(|l| l.split_whitespace().count()).sum::<usize>();
        if email.chars().any(char::is_alphabetic) && !email.chars().any(char::is_uppercase) {
            lowercase += 1;
        }
    }
    if past.is_empty() {
        return None;
    }
    let mut said = Vec::new();
    if let Some(g) = greetings.first() {
        said.push(format!("you open with \u{201c}{g}\u{201d}"));
    } else {
        said.push("you usually skip the greeting".to_string());
    }
    match signoffs.first() {
        _ if !sign_off => {}
        Some(s) => said.push(format!("you sign off with \u{201c}{s}\u{201d}")),
        None => said.push("you usually skip the sign-off".to_string()),
    }
    let average = words / past.len();
    said.push(format!("your emails are about {average} words long"));
    if lowercase * 2 > past.len() {
        said.push("you write in lowercase".to_string());
    }
    Some(format!("How you usually write to them: {}.", said.join("; ")))
}

/// Today's date, the conversation (unless `conversation` is false), your voice, your draft,
/// then the job.
fn user(context: &Context, draft: &str, task: &str, conversation: bool) -> String {
    let mut out = format!("Today is {}.\n\n", day(context.now));
    if conversation && !context.messages.is_empty() {
        out.push_str(&format!("The conversation, oldest first.\nSubject: {}\n", context.subject));
        for (from, at, text) in &context.messages {
            let who = if from.email.eq_ignore_ascii_case(&context.me.email) {
                "You".to_string()
            } else {
                crate::prompt::sender(from)
            };
            out.push_str(&format!("\n--- From: {who}, {}\n{text}\n", day(*at)));
        }
        out.push_str("--- End of the conversation.\n\n");
    }
    if conversation {
        let sign_off = Ending::for_signature(&context.signature, &context.me) != Ending::Nothing;
        if let Some(voice) = voice(&context.past, &context.me, sign_off) {
            out.push_str(&voice);
            out.push_str("\n\n");
        }
    }
    if !draft.trim().is_empty() {
        out.push_str(&format!("Your draft:\n{}\n\n", draft.trim()));
    }
    out.push_str(task);
    out
}

/// Tidies what the model wrote into the text of an email: no subject line, preamble, code
/// fences or placeholders, and an end that fits `ending`: your name stays only when there's no
/// signature to add it, and a sign-off goes when your signature has its own.
pub fn clean(text: &str, me: &Person, ending: Ending) -> String {
    let text = line_breaks(text);
    let mut lines: Vec<&str> = text.trim().lines().map(str::trim_end).collect();
    if lines.first().is_some_and(|l| l.trim_start().starts_with("```")) {
        lines.remove(0);
    }
    if lines.last().is_some_and(|l| l.trim() == "```") {
        lines.pop();
    }
    // A subject line, or a line about what follows ("Here's a friendlier version:").
    let preamble = |line: &str| {
        let l = line.trim().to_lowercase();
        l.ends_with(':')
            && ["here", "sure", "certainly", "okay", "of course"].iter().any(|w| l.starts_with(w))
            && ["version", "reply", "email", "draft", "rewrite", "translation", "response"].iter().any(|w| l.contains(w))
    };
    while lines.first().is_some_and(|l| {
        l.trim().is_empty() || l.trim().to_lowercase().starts_with("subject:") || preamble(l)
    }) {
        lines.remove(0);
    }
    // At the end: placeholders and your name.
    let full = me.name.trim().to_lowercase();
    let first = full.split_whitespace().next().unwrap_or("").to_string();
    let name = |line: &str| {
        let l = line.trim().trim_start_matches(['-', '–', '—']).trim_end_matches('.').trim().to_lowercase();
        !full.is_empty() && (l == full || l == first)
    };
    let placeholder = |line: &str| line.trim().starts_with('[') && line.trim().ends_with(']');
    let blank = |line: &str| line.trim().is_empty();
    let stray = |line: &str| line.trim().chars().all(|c| c.is_ascii_punctuation() || "–—…".contains(c));
    // A signature the model made up anyway starts with your name near the end: cut from there,
    // keeping your name when nothing else will add it.
    let tail = lines.len().saturating_sub(5);
    if let Some(at) = lines[tail..].iter().position(|l| name(l)) {
        lines.truncate(tail + at + usize::from(ending == Ending::SignOffAndName));
    }
    // So does one after a placeholder ("[Your Signature]", then a name it made up).
    let tail = lines.len().saturating_sub(5);
    if let Some(at) = lines[tail..].iter().position(|l| placeholder(l)) {
        lines.truncate(tail + at);
    }
    while lines.last().is_some_and(|l| stray(l)) {
        lines.pop();
    }
    // A sign-off on its own after the body, when your signature brings one: a short last line
    // that doesn't end a sentence.
    let sign_off = |line: &str| {
        let l = line.trim();
        l.split_whitespace().count() <= 3 && !l.ends_with(['.', '!', '?', ':'])
    };
    if ending == Ending::Nothing && lines.len() > 2 && lines.last().is_some_and(|l| sign_off(l)) && blank(lines[lines.len() - 2]) {
        lines.pop();
        while lines.last().is_some_and(|l| blank(l)) {
            lines.pop();
        }
    }
    // At most one blank line between paragraphs.
    let mut out: Vec<&str> = Vec::new();
    for line in lines {
        if !(blank(line) && out.last().is_some_and(|l| blank(l))) {
            out.push(line);
        }
    }
    out.join("\n").trim().to_string()
}

/// `text` with the HTML line breaks a model writes ("<br>", "<BR/>", "<br />") as the line
/// breaks they mean: an email's text is plain.
fn line_breaks(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    // Lowercasing ASCII keeps every byte where it was.
    while let Some(at) = rest.to_ascii_lowercase().find("<br") {
        let after = &rest[at + 3..];
        match after.trim_start_matches(' ').trim_start_matches('/').strip_prefix('>') {
            Some(next) => {
                out.push_str(&rest[..at]);
                out.push('\n');
                rest = next;
            }
            None => {
                out.push_str(&rest[..at + 3]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Two or three quick replies to pick from: first what each says, then each written as a
/// whole email, streaming: `on_options` hears the replies so far each time one grows, the one
/// being written last. Return `false` to stop, and get what's written so far.
pub async fn options(
    model: &dyn ChatModel,
    context: &Context,
    on_options: &mut (dyn FnMut(&[ReplyOption]) -> bool + Send),
) -> Result<Vec<ReplyOption>, AiError> {
    let context = &context.within(crate::chat::room(CONVERSATION_CHARS, model));
    let (system, user) = options_prompt(context);
    // A writer that can hold the model to this shape does, so the answer always reads.
    let request = ChatRequest::ask(&system, &user, 400).temperature(0.4).format(json!({
        "type": "object",
        "properties": {"options": {"type": "array", "minItems": 2, "maxItems": 3, "items": {
            "type": "object",
            "properties": {"label": {"type": "string"}, "says": {"type": "string"}},
            "required": ["label", "says"]
        }}},
        "required": ["options"]
    }));
    let reply = model.chat(&request, &mut |_| true).await?;
    let unreadable = || AiError::Model("no replies came back".into());
    let read: Value = json_in(&reply.text).ok_or_else(unreadable)?;
    let mut chosen: Vec<(String, String)> = Vec::new();
    for option in read["options"].as_array().into_iter().flatten() {
        let label = option["label"].as_str().unwrap_or("").trim().to_string();
        let says = option["says"].as_str().unwrap_or("").trim().to_string();
        let seen = chosen.iter().any(|(l, _)| l.eq_ignore_ascii_case(&label));
        if !label.is_empty() && !says.is_empty() && !seen && chosen.len() < 3 {
            chosen.push((label, says));
        }
    }
    let mut found: Vec<ReplyOption> = Vec::new();
    let mut stopped = false;
    for (label, says) in chosen {
        let job = Job::Instructions { instructions: says };
        let text = draft(model, &job, "", context, &mut |text| {
            let mut so_far = found.clone();
            so_far.push(ReplyOption { label: label.clone(), draft: text.to_string() });
            stopped = !on_options(&so_far);
            !stopped
        })
        .await?;
        if !text.is_empty() {
            found.push(ReplyOption { label, draft: text });
        }
        if stopped {
            break;
        }
    }
    if found.is_empty() {
        return Err(unreadable());
    }
    Ok(found)
}

/// Writes `job`, streaming: `on_text` hears the whole reply so far, tidied, each time it
/// grows; return `false` to stop. Returns the reply, tidied.
pub async fn draft(
    model: &dyn ChatModel,
    job: &Job,
    draft: &str,
    context: &Context,
    on_text: &mut (dyn FnMut(&str) -> bool + Send),
) -> Result<String, AiError> {
    let context = &context.within(crate::chat::room(CONVERSATION_CHARS, model));
    let Some((system, user)) = prompt(job, draft, context) else {
        return Ok(String::new());
    };
    let exact = matches!(job, Job::Fix | Job::Translate { .. });
    let request = ChatRequest::ask(&system, &user, 1_000).temperature(if exact { 0.1 } else { 0.3 });
    let ending = Ending::for_signature(&context.signature, &context.me);
    let mut so_far = String::new();
    let mut shown = String::new();
    let written = model.chat(&request, &mut |piece| {
        so_far.push_str(piece);
        let now = clean(&so_far, &context.me, ending);
        if now == shown {
            return true;
        }
        shown = now;
        on_text(&shown)
    })
    .await?;
    Ok(clean(&written.text, &context.me, ending))
}

/// The JSON object in what a model wrote: all of it, or the part between the first `{` and the
/// last `}` when it wrapped it in words or a code fence.
fn json_in(text: &str) -> Option<Value> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(text.get(start..=end)?).ok()
}

/// Whether a failed run is worth one more: a failure stops the built-in model, and the first
/// run after a while can fail where a fresh start works. A model that isn't downloaded is
/// final.
fn again<T>(run: &Result<T, AiError>) -> bool {
    matches!(run, Err(e) if !matches!(e, AiError::NotReady))
}

impl Ai {
    /// Two or three quick replies to a conversation, from the writing model, streaming (see
    /// [`options`]), tried once more on a fresh start if they fail.
    pub async fn reply_options(
        &self,
        context: &Context,
        on_options: &mut (dyn FnMut(&[ReplyOption]) -> bool + Send),
    ) -> Result<Vec<ReplyOption>, AiError> {
        let run = options(self, context, on_options).await;
        if !again(&run) {
            return run;
        }
        options(self, context, on_options).await
    }

    /// Writes `job` with the writing model, streaming (see [`draft`]), tried once more on a
    /// fresh start if it fails. Rewriting an empty draft writes nothing and doesn't start the
    /// model.
    pub async fn reply(
        &self,
        job: &Job,
        text: &str,
        context: &Context,
        on_text: &mut (dyn FnMut(&str) -> bool + Send),
    ) -> Result<String, AiError> {
        if prompt(job, text, context).is_none() {
            return Ok(String::new());
        }
        let run = draft(self, job, text, context, on_text).await;
        if !again(&run) {
            return run;
        }
        draft(self, job, text, context, on_text).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::Endpoint;
    use moss_core::{ImageTrust, Message};
    use wiremock::matchers::{body_partial_json, body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn person(name: &str, email: &str) -> Person {
        Person { name: name.into(), email: email.into() }
    }

    fn maya() -> Person {
        person("Maya Chen", "maya@moss.app")
    }

    fn dana() -> Person {
        person("Dana Wu", "dana@acme.co")
    }

    #[test]
    fn a_first_name_comes_from_the_address_when_there_is_no_name() {
        assert_eq!(first_name(&dana()), "Dana");
        assert_eq!(first_name(&person(" ", "sam.lee@acme.co")), "sam.lee");
    }

    /// Sat 31 Oct 2026 and Thu 5 Nov 2026, at noon UTC: the same day in nearly every time zone.
    const SAT: i64 = 1_793_448_000_000;
    const TODAY: i64 = 1_793_880_000_000;

    fn context() -> Context {
        Context {
            subject: "Q4 deck".into(),
            messages: vec![
                (dana(), SAT, "Can you send the deck by Friday?".into()),
                (maya(), SAT, "Working on it.".into()),
                (dana(), SAT, "Great — and can we meet Thursday at 3?".into()),
            ],
            me: maya(),
            past: vec!["Hey Dana! Sure thing.".into()],
            now: TODAY,
            signature: vec!["Maya Chen".into(), "Moss Studio".into()],
            to: None,
        }
    }

    #[test]
    fn a_smaller_room_keeps_the_newest_messages_and_cuts_each_in_measure() {
        let long = Context {
            messages: vec![(dana(), SAT, "a".repeat(9_000)), (maya(), SAT, "b".repeat(9_000)), (dana(), SAT, "c".repeat(20_000))],
            past: vec!["p".repeat(4_000)],
            ..context()
        };
        assert_eq!(long.within(CONVERSATION_CHARS), long, "the Gemma writer's room: all of it");
        // A quarter of the room: messages of at most 7,500 characters, 17,500 in all.
        let small = long.within(CONVERSATION_CHARS / 4);
        let lengths: Vec<usize> = small.messages.iter().map(|(_, _, t)| t.chars().count()).collect();
        assert_eq!(lengths, [7_500, 7_500], "the oldest left out");
        assert_eq!(small.messages[1].2, "c".repeat(7_500));
        assert_eq!(small.past, ["p".repeat(1_000)]);
        assert_eq!((small.subject.as_str(), small.signature.clone()), ("Q4 deck", context().signature));
    }

    #[tokio::test]
    async fn a_model_with_a_smaller_context_reads_less_of_the_conversation() {
        let model = crate::test_mail::ScriptedModel::new(vec![crate::test_mail::Step::Say(vec!["Sure."])]).with_context(8_192);
        let long = Context { messages: vec![(dana(), SAT, "x".repeat(40_000))], ..context() };
        draft(&model, &Job::Accept, "", &long, &mut |_| true).await.unwrap();
        let user = model.requests()[0].messages[1].chars();
        assert!(user < 12_000, "{user}");
    }

    fn message(id: &str, from: Person, paragraphs: &[&str]) -> Message {
        Message {
            id: id.into(),
            thread_id: "t".into(),
            from,
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "Q4 deck".into(),
            sent_at: id.parse().unwrap(),
            paragraphs: paragraphs.iter().map(|p| p.to_string()).collect(),
            body_text: String::new(),
            body_html: None,
            attachments: vec![],
            read: true,
            image_trust: ImageTrust::default(),
            sender_unverified: false,
            headers_only: false,
            images_missing: false,
            forwards: vec![],
            forwarded: false,
        }
    }

    fn thread(messages: Vec<Message>) -> Thread {
        Thread {
            summary: moss_core::ThreadSummary {
                id: "t".into(),
                account_id: "work".into(),
                split: moss_core::SplitId::Important,
                section: moss_core::Section::Reply,
                from: dana(),
                subject: "Q4 deck".into(),
                summary: String::new(),
                last_message_at: 0,
                snoozed_until: None,
                unread: false,
                done: false,
                done_at: None,
                labels: vec![],
                message_count: messages.len() as u32,
                has_attachments: false,
                ai_sure: None,
                ai_summary: None,
                ai_summary_why: None,
                sender: Default::default(),
                due: None,
                split_why: None,
                waiting: None,
                moved_by_you: false,
                you_wrote_last: false,
                to_address: None,
            },
            messages,
        }
    }

    #[test]
    fn a_thread_is_read_whole_unless_it_is_huge() {
        // Fills the conversation to just under the limit with the two huge ones and the latest.
        let long = "w".repeat(CONVERSATION_CHARS - 2 * MESSAGE_CHARS - 30);
        let huge = "é".repeat(MESSAGE_CHARS + 10);
        let t = thread(vec![
            message("1", dana(), &["Oldest, and doesn't fit."]),
            message("2", dana(), &[&huge]),
            message("3", dana(), &[&huge]),
            message("4", dana(), &[&long]),
            message("5", maya(), &[]),
            message("6", dana(), &["Can we meet", "Thursday at 3?"]),
        ]);
        let c = Context::from_thread(&t, maya(), vec!["a".repeat(5_000), "b".into(), "c".into(), "d".into()], TODAY);
        assert_eq!(c.subject, "Q4 deck");
        assert_eq!(c.me, maya());
        assert_eq!(c.now, TODAY);
        // Each message keeps when it was sent.
        assert_eq!(c.messages[3].1, 6);
        let texts: Vec<&str> = c.messages.iter().map(|(_, _, t)| t.as_str()).collect();
        // A long email is read whole; only a huge one is cut; the oldest doesn't fit; an
        // empty message is left out.
        assert_eq!(texts.len(), 4);
        assert_eq!(texts[3], "Can we meet\n\nThursday at 3?");
        assert_eq!(texts[2], long.trim());
        assert!(texts[..2].iter().all(|t| t.chars().count() == MESSAGE_CHARS));
        assert_eq!(c.past.len(), PAST);
        assert_eq!(c.past[0].chars().count(), PAST_CHARS);
    }

    /// Dana's latest is a whole email, with a greeting and paragraphs.
    fn email_context() -> Context {
        let mut c = context();
        c.messages.push((dana(), SAT, "Hi Maya,\n\nThanks for the update. Can we also go through the numbers on Thursday?\n\nDana".into()));
        c
    }

    #[test]
    fn a_reply_is_written_as_you_from_the_conversation() {
        let job = Job::Instructions { instructions: "say yes to Thursday, Friday is too soon".into() };
        let (system, user) = prompt(&job, "", &email_context()).unwrap();
        // The model is you, writing to them: one "you" for a small model to keep track of.
        assert!(system.starts_with("You are Maya Chen <maya@moss.app>, writing an email yourself, in the first person."), "{system}");
        assert!(system.contains("You are replying to Dana Wu <dana@acme.co>, who wrote the latest message."), "{system}");
        // The shape of an email, described: a greeting with their first name, the body, a
        // sign-off and nothing after it.
        for part in ["1. A greeting line", "Dana", "2. The body", "3. A sign-off", "nothing after the sign-off", "Language:", "already happened"] {
            assert!(system.contains(part), "{part}: {system}");
        }
        // Short beats complete: it says what it's asked to and stops, with nothing to fill in.
        assert!(system.contains("Say what you're asked to say and stop"), "{system}");
        assert!(system.contains("Don't repeat or sum up what they wrote"), "{system}");
        assert!(system.contains("leave it out"), "{system}");
        for filler in ["closing sentence", "what happens next", "say you'll confirm it", "Answer every question"] {
            assert!(!system.contains(filler), "{filler}: {system}");
        }
        // The conversation with your own messages marked, how you write to them (never your
        // emails themselves, which a small model would copy), then the job, then the language.
        let conversation = user.find("Subject: Q4 deck").unwrap();
        let voice = user.find("How you usually write to them:").unwrap();
        let task = user.find("say yes to Thursday, Friday is too soon").unwrap();
        assert!(conversation < voice && voice < task, "{user}");
        assert!(!user.contains("Sure thing"), "{user}");
        // Beside the note, where a small model heeds it: the note is what the email says, as
        // long as it needs, and nothing made up of its own.
        let own = user.find("Add no news, plans or promises").unwrap();
        assert!(task < own, "{user}");
        assert!(user.contains("Say what your note says and nothing more. Keep its words and facts"), "{user}");
        assert!(!user.contains("complete email"), "{user}");
        assert!(user.contains("--- From: Dana Wu <dana@acme.co>, Sat 31 Oct 2026\nCan you send the deck by Friday?"), "{user}");
        assert!(user.contains("--- From: You, Sat 31 Oct 2026\nWorking on it."), "{user}");
        // Today's date comes first, so "Monday" and "next week" mean something.
        assert!(user.starts_with("Today is Thu 5 Nov 2026.\n\nThe conversation, oldest first."), "{user}");
        // Then, last, the note's own facts, kept as it gives them.
        assert!(user.ends_with("\nWrite it in the language of Dana's latest message.\nKeep every fact your note gives as it gives it: a day, time or amount in your note stays as it is."), "{user}");
        assert!(!user.contains("Your draft"), "{user}");
        // With a draft, it reworks the draft.
        let (_, user) = prompt(&job, "Thursday is fine.", &email_context()).unwrap();
        assert!(user.contains("Your draft:\nThursday is fine."), "{user}");
    }

    #[test]
    fn a_new_email_is_written_to_the_person_it_is_for() {
        // Ask Moss writes to Alex from a note, with no conversation: only `to` says who he is.
        let alex = person("Alex Morgan Studio Build", "alex@studio-build.example.test");
        let new = Context { subject: "Texting".into(), messages: vec![], past: vec![], to: Some(alex), ..context() };
        let job = Job::Instructions { instructions: "I'll text you soon.".into() };
        let (system, user) = prompt(&job, "", &new).unwrap();
        assert!(system.contains("You are writing a new email to Alex Morgan Studio Build <alex@studio-build.example.test>."), "{system}");
        assert!(system.contains("1. A greeting line: a greeting word and Alex,"), "{system}");
        assert!(!system.contains("replying"), "{system}");
        assert!(user.contains("Write your email. "), "{user}");
        assert!(!user.contains("Write your reply"), "{user}");
        assert!(!user.contains("Subject:"), "no conversation: {user}");
        assert!(user.contains("\nWrite it in the language of your note.\nKeep every fact"), "{user}");
    }

    fn email_form(c: &Context) -> bool {
        let system = system(c);
        let note = system.contains("quick, casual note");
        assert_ne!(note, system.contains("1. A greeting line"), "{system}");
        !note
    }

    #[test]
    fn a_quick_note_gets_one_back_only_when_you_both_write_notes() {
        // Dana's latest is one short line, and so was yours to her.
        assert!(!email_form(&context()));
        // A long one-paragraph message is still an email.
        let mut c = context();
        c.messages.push((dana(), SAT, "word ".repeat(80)));
        assert!(email_form(&c));
        // A two-line note in a thread where you write whole emails gets an email back.
        let mut c = context();
        c.messages[1].2 = "Hi Dana,\n\nIt's on its way by Friday.\n\nBest\nMaya".into();
        c.messages.push((dana(), SAT, "The sample is ready.\nPlease message me tomorrow.\nAlex".into()));
        assert!(email_form(&c));
        // With nothing of yours in the thread, your last email to them decides; with nothing
        // at all, it's an email.
        let mut c = context();
        c.messages.remove(1);
        assert!(!email_form(&c));
        c.past = vec!["Hi Dana,\n\nSure thing.\n\nBest,".into()];
        assert!(email_form(&c));
        c.past.clear();
        assert!(email_form(&c));
    }

    #[test]
    fn how_you_write_to_them_comes_from_your_earlier_emails_without_their_words() {
        let me = maya();
        let formal = "Hi Dana,\n\nSure thing, see you then.\n\nBest,\nMaya Chen".to_string();
        assert_eq!(
            voice(&[formal.clone()], &me, true).as_deref(),
            Some("How you usually write to them: you open with \u{201c}Hi Dana,\u{201d}; you sign off with \u{201c}Best,\u{201d}; your emails are about 8 words long.")
        );
        let chatty = "haha yes!! see you then x".to_string();
        assert_eq!(
            voice(&[chatty.clone()], &me, true).as_deref(),
            Some("How you usually write to them: you usually skip the greeting; you usually skip the sign-off; your emails are about 6 words long; you write in lowercase.")
        );
        // Lowercase only when most are; a long first line isn't a greeting; an emoji isn't lowercase.
        let long_start = "Thanks so much for this, Dana!\n\nSee you.".to_string();
        let said = voice(&[chatty, long_start, "👍".into()], &me, true).unwrap();
        assert!(said.contains("skip the greeting") && !said.contains("lowercase"), "{said}");
        assert_eq!(voice(&[], &me, true), None);
        // Without a name to drop, the last line stays part of the email.
        assert!(voice(&[formal], &person("", "m@x.co"), true).unwrap().contains("\u{201c}Maya Chen\u{201d}"));
    }

    #[test]
    fn with_nothing_before_it_writes_from_scratch() {
        let alone = Context { subject: String::new(), messages: vec![], me: person("", "maya@moss.app"), past: vec![], now: TODAY, signature: vec![], to: None };
        let (system, user) = prompt(&Job::Accept, "", &alone).unwrap();
        assert!(system.contains("maya@moss.app"), "{system}");
        assert!(!system.contains("You are replying to"), "{system}");
        assert!(!user.contains("Subject:"), "{user}");
        assert!(user.starts_with("Today is Thu 5 Nov 2026."), "{user}");
        assert!(!user.contains("How you usually write"), "{user}");
        assert!(user.ends_with("the language of their latest message."), "{user}");
    }

    #[test]
    fn each_rewrite_asks_for_its_own_change() {
        let c = context();
        let task = |job: Job| prompt(&job, "Thursday at 3 works.", &c).unwrap().1;
        let all = [
            (Job::Friendlier, "friendlier"),
            (Job::Formal, "formal"),
            (Job::Shorter, "shorter"),
            (Job::Longer, "fuller"),
            (Job::Fix, "spelling"),
            (Job::Translate { language: "French".into() }, "French"),
            (Job::Accept, "says yes"),
            (Job::Decline, "says no"),
            (Job::MoreTime, "more time"),
        ];
        for (job, word) in all {
            let user = task(job.clone());
            assert!(user.contains(word), "{job:?}: {user}");
            assert!(user.contains("Your draft:\nThursday at 3 works."), "{job:?}: {user}");
        }
        // Rewrites keep the draft's language; answers take the conversation's; a translation
        // names its own.
        for job in [Job::Friendlier, Job::Formal, Job::Shorter, Job::Longer, Job::Fix] {
            assert!(task(job).ends_with("\nKeep the language your draft is written in."));
        }
        assert!(task(Job::Decline).ends_with("\nWrite it in the language of Dana's latest message."));
        assert!(task(Job::Translate { language: "French".into() }).ends_with("not in the language of the conversation."));
        // Fixing and translating don't need the conversation.
        assert!(!task(Job::Fix).contains("Subject:"));
        assert!(!task(Job::Translate { language: "French".into() }).contains("Subject:"));
        assert!(task(Job::Shorter).contains("Subject: Q4 deck"));
    }

    #[test]
    fn rewriting_an_empty_draft_is_nothing_but_answers_can_start_from_nothing() {
        let c = context();
        for job in [Job::Friendlier, Job::Formal, Job::Shorter, Job::Longer, Job::Fix, Job::Translate { language: "German".into() }] {
            assert_eq!(prompt(&job, "  \n", &c), None, "{job:?}");
        }
        for job in [Job::Accept, Job::Decline, Job::MoreTime, Job::Instructions { instructions: "no".into() }] {
            let (_, user) = prompt(&job, "", &c).unwrap();
            assert!(!user.contains("Your draft"), "{job:?}");
        }
    }

    #[test]
    fn quick_replies_are_chosen_first_as_what_each_says() {
        let (system, user) = options_prompt(&context());
        assert!(system.contains("Maya Chen"), "{system}");
        assert!(user.contains("2 or 3"), "{user}");
        assert!(user.contains("\"label\"") && user.contains("\"says\""), "{user}");
        // What each says is your answer, as you: written as what they said, a small model wrote
        // their words back to them as yours.
        assert!(user.contains("\"says\": what you say back to Dana in that reply, in one sentence in the first person"), "{user}");
        assert!(user.contains("your answer, not a retelling of what Dana wrote"), "{user}");
        assert!(user.contains("with names, days and amounts as the conversation gives them"), "{user}");
        assert!(user.contains("Dana's latest message"), "{user}");
        assert!(user.contains("--- From: You, Sat 31 Oct 2026\nWorking on it."), "{user}");
    }

    #[test]
    fn jobs_come_from_the_ui_by_kind() {
        let job: Job = serde_json::from_value(json!({"kind": "translate", "language": "Spanish"})).unwrap();
        assert_eq!(job, Job::Translate { language: "Spanish".into() });
        let job: Job = serde_json::from_value(json!({"kind": "moreTime"})).unwrap();
        assert_eq!(job, Job::MoreTime);
        let job: Job = serde_json::from_value(json!({"kind": "instructions", "instructions": "be brief"})).unwrap();
        assert_eq!(job, Job::Instructions { instructions: "be brief".into() });
    }

    #[test]
    fn the_signature_decides_how_the_email_ends() {
        let me = maya();
        let lines = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        // A first line that's short, not your name and not contact details is a sign-off.
        for signature in [&["Best", "Maya"][..], &["Best regards,", "Maya Chen", "Moss Studio"], &["Cheers!"], &["", "Talk soon"]] {
            assert_eq!(Ending::for_signature(&lines(signature), &me), Ending::Nothing, "{signature:?}");
        }
        for signature in [&["Maya Chen", "Design lead"][..], &["maya"], &["Maya."], &["maya@moss.app"], &["+1 555 0100"], &["moss.app"], &["Design lead, Northwind"], &["Ross Studio photography and design"]] {
            assert_eq!(Ending::for_signature(&lines(signature), &me), Ending::SignOff, "{signature:?}");
        }
        assert_eq!(Ending::for_signature(&[], &me), Ending::SignOffAndName);
        assert_eq!(Ending::for_signature(&lines(&["  "]), &me), Ending::SignOffAndName);
        assert_eq!(Ending::for_signature(&lines(&["Maya"]), &person("", "m@x.co")), Ending::Nothing);

        // The model is told how to end, and how you usually sign off only when it should.
        let mut c = email_context();
        c.past = vec!["Hi Dana,\n\nSure thing, see you then.\n\nBest,\nMaya Chen".into()];
        let (system, user) = prompt(&Job::Accept, "", &c).unwrap();
        assert!(system.contains("3. A sign-off on its own line") && system.contains("nothing after the sign-off"), "{system}");
        assert!(user.contains("you sign off with"), "{user}");
        c.signature = lines(&["Best", "Maya"]);
        let (system, user) = prompt(&Job::Accept, "", &c).unwrap();
        assert!(system.contains("3. No sign-off and no name"), "{system}");
        assert!(!system.contains("3. A sign-off"), "{system}");
        assert!(!user.contains("sign off"), "{user}");
        c.signature.clear();
        let (system, user) = prompt(&Job::Accept, "", &c).unwrap();
        assert!(system.contains("then your first name on the line below it"), "{system}");
        assert!(user.contains("you sign off with"), "{user}");
    }

    #[test]
    fn the_end_of_the_email_is_tidied_to_fit_the_signature() {
        let me = maya();
        // Your signature has its own sign-off: the model's goes, with your name.
        assert_eq!(clean("Hi Dana,\n\nYes.\n\nBest,", &me, Ending::Nothing), "Hi Dana,\n\nYes.");
        assert_eq!(clean("Hi Dana,\n\nYes.\n\nBest regards,\nMaya", &me, Ending::Nothing), "Hi Dana,\n\nYes.");
        assert_eq!(clean("Hi Dana,\n\nYes.\n\nAll the best\n\n", &me, Ending::Nothing), "Hi Dana,\n\nYes.");
        // A last sentence, a line inside a paragraph and a one-line note stay.
        assert_eq!(clean("Hi Dana,\n\nSee you Thursday!", &me, Ending::Nothing), "Hi Dana,\n\nSee you Thursday!");
        assert_eq!(clean("Hi Dana,\n\nYes, and\nsee you then", &me, Ending::Nothing), "Hi Dana,\n\nYes, and\nsee you then");
        assert_eq!(clean("Sounds good", &me, Ending::Nothing), "Sounds good");
        // No signature: your name stays after the sign-off, and what the model made up after it goes.
        assert_eq!(clean("Yes.\n\nBest,\nMaya\nMoss Studio\n+1 555 0100", &me, Ending::SignOffAndName), "Yes.\n\nBest,\nMaya");
        assert_eq!(clean("Yes.\n\nBest,\nMaya Chen", &me, Ending::SignOffAndName), "Yes.\n\nBest,\nMaya Chen");
    }

    #[test]
    fn the_email_text_is_tidied() {
        let me = maya();
        // The sign-off stays; your name and anything the model made up after it go, because
        // your saved signature follows.
        assert_eq!(clean("  Hi Dana,\n\nYes.\n\nBest,\nMaya  ", &me, Ending::SignOff), "Hi Dana,\n\nYes.\n\nBest,");
        assert_eq!(clean("Hi Dana,\n\nYes.\n\nBest regards,\n\nMaya Chen\nMoss Studio\n+1 555 0100", &me, Ending::SignOff), "Hi Dana,\n\nYes.\n\nBest regards,");
        assert_eq!(clean("Yes.\n\nThanks,\n[Your name]", &me, Ending::SignOff), "Yes.\n\nThanks,");
        assert_eq!(clean("Yes.\n\n- Maya", &me, Ending::SignOff), "Yes.");
        assert_eq!(clean("Yes.\n\nBest,\nMaya.", &me, Ending::SignOff), "Yes.\n\nBest,");
        // A stray line of punctuation at the end goes.
        assert_eq!(clean("Hi Dana,\n\nYes.\n\nBest,\n.", &me, Ending::SignOff), "Hi Dana,\n\nYes.\n\nBest,");
        assert_eq!(clean("Yes!\n-", &me, Ending::SignOff), "Yes!");
        // Your name far from the end is part of the email.
        assert_eq!(clean("Maya\n\na\n\nb\n\nc", &me, Ending::SignOff), "Maya\n\na\n\nb\n\nc");
        assert_eq!(clean("Here's a friendlier version:\n\nHi Dana!", &me, Ending::SignOff), "Hi Dana!");
        assert_eq!(clean("Sure! Here is the reply:\nHi.", &me, Ending::SignOff), "Hi.");
        assert_eq!(clean("Subject: Re: Q4 deck\n\nHi Dana,", &me, Ending::SignOff), "Hi Dana,");
        assert_eq!(clean("```\nHi Dana,\n```", &me, Ending::SignOff), "Hi Dana,");
        assert_eq!(clean("```text\nHi.\n\n\n\nBye now.\n```", &me, Ending::SignOff), "Hi.\n\nBye now.");
        assert_eq!(clean("Dear Dana:\n\nThanks!", &me, Ending::SignOff), "Dear Dana:\n\nThanks!");
        assert_eq!(clean("   ", &me, Ending::SignOff), "");
        // An email is plain text: HTML line breaks a model writes are line breaks.
        assert_eq!(clean("Hi Simon,\n\n<br>\n\nIt's been a while.<BR/>Let's meet.<br />\n\nBest,", &me, Ending::SignOff), "Hi Simon,\n\nIt's been a while.\nLet's meet.\n\nBest,");
        assert_eq!(clean("Hi <brand> team, see <br", &me, Ending::SignOff), "Hi <brand> team, see <br", "not a line break");
        // A made-up signature after a placeholder goes with it.
        assert_eq!(clean("Hi Dana,\n\nYes.\n\nBest,\n[Your Signature]\nAlex", &me, Ending::SignOff), "Hi Dana,\n\nYes.\n\nBest,");
        // Without a name there's no name to drop.
        assert_eq!(clean("Yes.\n\nCheers,", &person("", "m@x.co"), Ending::SignOff), "Yes.\n\nCheers,");
    }

    /// The writing model in llama-server at `endpoint`.
    struct Llama(Endpoint);

    #[async_trait::async_trait]
    impl ChatModel for Llama {
        async fn chat(&self, request: &ChatRequest, on_text: &mut crate::chat::OnText) -> Result<crate::chat::Reply, AiError> {
            crate::chat::stream(&crate::http::local(), &self.0, request, on_text).await
        }
    }

    /// An answer in one streamed piece (none when empty).
    fn content(text: &str) -> String {
        sse(text)
    }

    fn sse(text: &str) -> String {
        let event = if text.is_empty() { String::new() } else { format!("data: {}\n\n", json!({"choices": [{"delta": {"content": text}}]})) };
        format!("{event}data: [DONE]\n\n")
    }

    /// A model that answers the choice with `choices`, and writes every reply as `written`,
    /// except the one asked to "Ask what it's about", which it leaves empty.
    async fn options_server(choices: String, written: &str) -> (MockServer, Llama) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({"response_format": {"type": "json_schema"}})))
            .respond_with(ResponseTemplate::new(200).set_body_raw(choices, "text/event-stream"))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_string_contains("Ask what it"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse(""), "text/event-stream"))
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({"stream": true})))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse(written), "text/event-stream"))
            .mount(&server)
            .await;
        let endpoint = Endpoint { url: server.uri(), key: "k".into() };
        (server, Llama(endpoint))
    }

    #[tokio::test]
    async fn quick_replies_are_chosen_then_each_written_as_a_whole_email() {
        let choices = json!({"options": [
            {"label": " Yes ", "says": "Say Thursday works"},
            {"label": "Yes", "says": "A second yes"},
            {"label": "Empty", "says": "  "},
            {"label": "", "says": "No label"},
            {"label": "Ask", "says": "Ask what it's about"},
            {"label": "Not Thursday", "says": "Say Thursday won't work"},
            {"label": "Fourth", "says": "Too many"}
        ]});
        let (server, endpoint) = options_server(content(&choices.to_string()), "Hi Dana,\n\nThursday works.\n\nBest,\nMaya").await;
        let got = options(&endpoint, &context(), &mut |_| true).await.unwrap();
        // Labels are distinct, at most 3 are written, and one written empty is left out.
        assert_eq!(
            got,
            [
                ReplyOption { label: "Yes".into(), draft: "Hi Dana,\n\nThursday works.\n\nBest,".into() },
                ReplyOption { label: "Not Thursday".into(), draft: "Hi Dana,\n\nThursday works.\n\nBest,".into() },
            ]
        );
        // Each is written from what it says, as plain text.
        let asked: Vec<String> = server.received_requests().await.unwrap().iter().map(|r| String::from_utf8_lossy(&r.body).to_string()).collect();
        assert_eq!(asked.len(), 4);
        assert!(asked[1].contains("Say Thursday works") && !asked[1].contains("response_format"));
        assert!(asked[3].contains("Say Thursday won't work"));
    }

    #[tokio::test]
    async fn quick_replies_stream_in_one_by_one_and_stop_when_asked() {
        let choices = json!({"options": [
            {"label": "Yes", "says": "Say Thursday works"},
            {"label": "Ask", "says": "Ask what it's about"},
            {"label": "Not Thursday", "says": "Say Thursday won't work"}
        ]});
        let (server, endpoint) = options_server(content(&choices.to_string()), "Hi Dana,\n\nThursday works.").await;
        let option = |label: &str| ReplyOption { label: label.into(), draft: "Hi Dana,\n\nThursday works.".into() };
        let mut heard: Vec<Vec<ReplyOption>> = Vec::new();
        let got = options(&endpoint, &context(), &mut |so_far| {
            heard.push(so_far.to_vec());
            true
        })
        .await
        .unwrap();
        // Each one shows as it's written, after the ones before; one written empty never shows.
        assert_eq!(heard, [vec![option("Yes")], vec![option("Yes"), option("Not Thursday")]]);
        assert_eq!(got, [option("Yes"), option("Not Thursday")]);
        // Stopping keeps what's written so far, and writes no more.
        let before = server.received_requests().await.unwrap().len();
        let got = options(&endpoint, &context(), &mut |_| false).await.unwrap();
        assert_eq!(got, [option("Yes")]);
        assert_eq!(server.received_requests().await.unwrap().len(), before + 2);
    }

    #[tokio::test]
    async fn quick_replies_that_cannot_be_read_or_come_back_empty_are_an_error() {
        for text in ["not json", r#"{"options": []}"#] {
            let (_s, endpoint) = options_server(content(text), "x").await;
            assert!(matches!(options(&endpoint, &context(), &mut |_| true).await, Err(AiError::Model(_))), "{text}");
        }
        let (_s, endpoint) = options_server("data: {\"choices\": []}\n\n".into(), "x").await;
        assert!(matches!(options(&endpoint, &context(), &mut |_| true).await, Err(AiError::Model(_))));
        // JSON wrapped in words or a code fence still reads.
        let fenced = format!("```json\n{}\n```", json!({"options": [{"label": "Yes", "says": "Say yes"}]}));
        let (_s, endpoint) = options_server(content(&fenced), "Yes.").await;
        assert_eq!(options(&endpoint, &context(), &mut |_| true).await.unwrap().len(), 1);
        let only_empty = json!({"options": [{"label": "Ask", "says": "Ask what it's about"}]});
        let (_s, endpoint) = options_server(content(&only_empty.to_string()), "x").await;
        assert!(matches!(options(&endpoint, &context(), &mut |_| true).await, Err(AiError::Model(_))));
        // A reply that fails to be written fails them all.
        let one = json!({"options": [{"label": "Yes", "says": "Say yes"}]});
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"response_format": {"type": "json_schema"}})))
            .respond_with(ResponseTemplate::new(200).set_body_raw(content(&one.to_string()), "text/event-stream"))
            .mount(&server)
            .await;
        let endpoint = Llama(Endpoint { url: server.uri(), key: "k".into() });
        assert!(matches!(options(&endpoint, &context(), &mut |_| true).await, Err(AiError::Model(_))));
    }

    #[tokio::test]
    async fn a_reply_streams_in_tidied() {
        let server = MockServer::start().await;
        let event = |piece: &str| format!("data: {}\n\n", json!({"choices": [{"delta": {"content": piece}}]}));
        let sse = format!("{}{}{}{}data: [DONE]\n\n", event("Hi Dana,"), event("\n\nThursday works."), event("\n\nBest,"), event("\nMaya"));
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(body_partial_json(json!({"stream": true})))
            .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
            .mount(&server)
            .await;
        let endpoint = Endpoint { url: server.uri(), key: "k".into() };
        let mut seen = Vec::new();
        let text = draft(&Llama(endpoint.clone()), &Job::Accept, "", &context(), &mut |t| {
            seen.push(t.to_string());
            true
        })
        .await
        .unwrap();
        assert_eq!(text, "Hi Dana,\n\nThursday works.\n\nBest,");
        // Only changes are heard: your name never shows.
        assert_eq!(seen, ["Hi Dana,", "Hi Dana,\n\nThursday works.", "Hi Dana,\n\nThursday works.\n\nBest,"]);
        // Nothing to rewrite: nothing is asked.
        let text = draft(&Llama(endpoint.clone()), &Job::Shorter, "", &context(), &mut |_| true).await;
        assert_eq!(text, Ok(String::new()));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

}
