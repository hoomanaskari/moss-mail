//! What Ask Moss can do with your mail: search it, read a thread, list files and read what's in
//! them, and find people. Everything else it does with mail (Done, remind, move, delete, block,
//! drafts) comes from the app as [`Actions`]: the same tools agents get over MCP. Without them it
//! can only draft a reply. Nothing here sends: an email waits for the user to press Send.

use async_trait::async_trait;
use moss_core::{ComposeMode, Contact, Draft, FileHit, FileKind, FileQuery, Person, SearchHit, Thread, ThreadSummary};
use serde::Serialize;
use serde_json::{json, Value};

/// The mail the tools read and the drafts they save. The app passes the engine; tests a fake.
#[async_trait]
pub trait Mail: Send + Sync {
    /// Search by words and meaning, best first.
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, String>;
    fn thread(&self, id: &str) -> Result<Thread, String>;
    /// A thread to read whole, with the text inside its files (`Attachment::text`) where Moss
    /// can read it. By default, as [`Mail::thread`] has it.
    async fn read_thread(&self, id: &str) -> Result<Thread, String> {
        self.thread(id)
    }
    fn files(&self, query: &moss_core::FileQuery) -> Result<Vec<moss_core::FileHit>, String>;
    /// One file: attachment `index` of a message.
    fn file(&self, message_id: &str, index: u32) -> Result<FileHit, String>;
    /// The text inside a file, read now if it hasn't been. `None` when Moss can't read it.
    async fn file_text(&self, message_id: &str, index: u32) -> Result<Option<String>, String>;
    fn people(&self, query: &str, limit: usize) -> Result<Vec<Contact>, String>;
    /// Saves a draft, as the composer does. Never sends it.
    fn save_draft(&self, draft: &Draft) -> Result<(), String>;
    /// What Quick reply reads to write `draft`'s email: the user as its account, the
    /// conversation, what they wrote that person before, and the signature it goes above.
    /// None where there's none to be had.
    fn reply_context(&self, draft: &Draft) -> Option<crate::reply::Context>;
}

/// What a tool did: text for the model, the threads it showed (`[t:ID]`), the threads it read
/// in full or replied to, the files it showed (`[f:ID]`) and read in full, and a draft it saved.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolOutput {
    pub text: String,
    pub threads: Vec<String>,
    pub opened: Vec<String>,
    pub files: Vec<FileSource>,
    pub read_files: Vec<FileSource>,
    pub draft: Option<Draft>,
    /// The user asked for the draft to go then (ms), with Send later.
    pub send_at: Option<i64>,
    /// What it changed, as the answer lists it: "Marked done: Lunch on Friday".
    pub did: Option<String>,
}

/// What an action did: text for the model, threads it listed (to cite), a draft it wrote or
/// changed, when the user wants that draft sent, and what it changed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Acted {
    pub text: String,
    pub threads: Vec<String>,
    pub draft: Option<Draft>,
    pub send_at: Option<i64>,
    pub did: Option<String>,
}

/// The app's own actions on mail, offered beside the tools here.
#[async_trait]
pub trait Actions: Send + Sync {
    /// The OpenAI-style `tools` entries.
    fn specs(&self) -> Vec<Value>;
    /// What the UI shows while `name` runs. `None`: not one of these.
    fn label(&self, name: &str, args: &Value) -> Option<String>;
    /// Runs `name`. `None`: not one of these. A failure is text for the model.
    async fn act(&self, name: &str, args: &Value) -> Option<Result<Acted, String>>;
}

/// A file an answer can cite as `[f:ID]`, shown under it to open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileSource {
    pub thread_id: String,
    pub message_id: String,
    /// Its place among the message's attachments (`Attachment::index`).
    pub index: u32,
    pub name: String,
}

impl FileSource {
    /// What the model cites it as: `[f:ID]`.
    pub fn id(&self) -> String {
        file_id(&self.message_id, self.index)
    }
}

/// A file's id for the model: its message and index, "4406-2".
pub fn file_id(message_id: &str, index: u32) -> String {
    format!("{message_id}-{index}")
}

/// The message and index of a file id, with or without its `[f:…]` mark.
pub fn parse_file_id(text: &str) -> Option<(String, u32)> {
    let id = text.trim().trim_matches(|c| c == '[' || c == ']').trim_start_matches("f:");
    let (message, index) = id.rsplit_once('-')?;
    Some((message.to_string(), index.parse().ok()?)).filter(|(m, _)| !m.is_empty())
}

/// The files of a thread (not images shown in the body), to cite.
pub fn thread_files(thread: &Thread) -> Vec<FileSource> {
    let files = thread.messages.iter().flat_map(|m| {
        m.attachments.iter().filter(|a| !a.inline).map(|a| FileSource {
            thread_id: thread.summary.id.clone(),
            message_id: m.id.clone(),
            index: a.index,
            name: a.name.clone(),
        })
    });
    files.collect()
}

impl ToolOutput {
    fn text(text: impl Into<String>) -> ToolOutput {
        ToolOutput { text: text.into(), ..Default::default() }
    }
}

/// Tools as the agent sees them.
#[async_trait]
pub trait Tools: Send + Sync {
    /// The OpenAI-style `tools` entries to offer the model.
    fn specs(&self) -> Vec<Value>;
    /// What the UI shows while a tool runs, e.g. "Searching mail for “Q4 budget”".
    fn label(&self, name: &str, args: &Value) -> String;
    /// Runs a tool. A failure is text for the model, so it can try something else.
    async fn call(&self, name: &str, args: &Value) -> ToolOutput;
    /// The threads to show as cards, by id, in that order. Leaves out ones that are gone.
    fn sources(&self, ids: &[String]) -> Vec<ThreadSummary>;
}

/// At most this much of a thread reaches the model when it reads one: a long email and its
/// replies, whole (about 10,000 tokens). Two of them still fit the writer's 32k context.
pub const READ_CHARS: usize = 40_000;
/// At most this much of one file's text is in a thread the model reads: about 8 pages. The
/// whole file is there with read_attachment.
pub const FILE_CHARS: usize = 12_000;
/// At most this much of a list (search hits, files, people) reaches the model. A list only points
/// at emails; the model reads the ones it needs in full.
pub const LIST_CHARS: usize = 4_000;
/// Search hits and files per answer.
const HITS: usize = 8;
const FILES: usize = 12;
const PEOPLE: usize = 8;

/// Every tool, by name.
pub const ALL: [&str; 6] = ["search_mail", "read_thread", "list_attachments", "read_attachment", "find_people", "draft_reply"];

/// The tools over a mailbox. `now` (ms) dates new drafts.
pub struct MailTools<'a> {
    mail: &'a dyn Mail,
    now: i64,
    only: Vec<&'static str>,
    actions: Option<&'a dyn Actions>,
    /// What the user asked: where an action names no email, the emails it names.
    question: String,
    /// Writes an email's words in a step of its own, when the model left them out of write_email.
    writer: Option<&'a dyn crate::chat::ChatModel>,
    /// The email on screen, which a reply the user asks for answers.
    open: Option<ThreadSummary>,
}

impl<'a> MailTools<'a> {
    /// Every tool.
    pub fn new(mail: &'a dyn Mail, now: i64) -> MailTools<'a> {
        MailTools { mail, now, only: ALL.to_vec(), actions: None, question: String::new(), writer: None, open: None }
    }

    /// The model that writes an email's words when write_email comes without them: a small
    /// model leaves them out of a tool call, and writes them well as a job of its own.
    pub fn writing_with(mut self, writer: &'a dyn crate::chat::ChatModel) -> MailTools<'a> {
        self.writer = Some(writer);
        self
    }

    /// What a read thread or file may hold: [`READ_CHARS`], less for a writer with a smaller
    /// context.
    fn read_chars(&self) -> usize {
        self.writer.map_or(READ_CHARS, |w| crate::chat::room(READ_CHARS, w))
    }

    /// The email on screen, if any: a new email to its sender may be the reply the user asked for.
    pub fn reading(mut self, open: Option<&Thread>) -> MailTools<'a> {
        self.open = open.map(|t| t.summary.clone());
        self
    }

    /// What the user asked, for an action that names no email.
    pub fn asked(mut self, question: &str) -> MailTools<'a> {
        self.question = question.to_string();
        self
    }

    /// The app's actions too. Their drafts (`create_draft`) take over from `draft_reply`.
    pub fn with_actions(mut self, actions: &'a dyn Actions) -> MailTools<'a> {
        self.only.retain(|n| *n != "draft_reply");
        self.actions = Some(actions);
        self
    }

    /// Only the tools named.
    pub fn only(mut self, names: &[&'static str]) -> MailTools<'a> {
        self.only.retain(|n| names.contains(n));
        self
    }
}

/// Hits looked through before filtering by sender and date.
const SEARCHED: usize = 30;

/// A tool's definition, for the model.
fn spec(name: &str) -> Value {
    let day = |what: &str| json!({"type": "string", "description": format!("{what}, as YYYY-MM-DD")});
    let (description, properties, required) = match name {
        "search_mail" => (
            "Search the user's mail by words or meaning. Use one or two telling words, such as a name or a word from the subject: every word has to match. Returns threads, newest match first, each with its id as [t:ID].",
            json!({
                "query": {"type": "string", "description": "What to look for"},
                "from": {"type": "string", "description": "Part of the sender's name or address"},
                "after": day("Only mail on or after this day"),
                "before": day("Only mail before this day"),
            }),
            vec!["query"],
        ),
        "read_thread" => (
            "Read every message of one thread: who wrote, when, the text, and the text inside its attachments.",
            json!({"thread_id": {"type": "string", "description": "The id from [t:ID]"}}),
            vec!["thread_id"],
        ),
        "list_attachments" => (
            "List files people sent, newest first, filtered by sender, file name, type or date.",
            json!({
                "from": {"type": "string", "description": "Part of the sender's name or address"},
                "name": {"type": "string", "description": "Part of the file name"},
                "type": {"type": "string", "enum": FileKind::ALL.map(FileKind::name)},
                "after": day("Only files sent on or after this day"),
                "before": day("Only files sent before this day"),
            }),
            vec![],
        ),
        "read_attachment" => (
            "Read the text inside one file someone sent: a PDF, a scan or photo of a letter, a Word, Excel or PowerPoint file. Use it when the answer may be in the file, not the email.",
            json!({"file_id": {"type": "string", "description": "The id from [f:ID]"}}),
            vec!["file_id"],
        ),
        "find_people" => (
            "Find people the user emails with, by part of their name or address.",
            json!({"query": {"type": "string", "description": "A name or address"}}),
            vec!["query"],
        ),
        _ => (
            "Write a draft reply to a thread, for the user to check and send themselves. It is never sent. Write it as the user, in the first person. End with a sign-off such as \"Best,\" and no name: the user's own signature goes under it.",
            json!({
                "thread_id": {"type": "string", "description": "The id from [t:ID]"},
                "body": {"type": "string", "description": "The reply's text"},
            }),
            vec!["thread_id", "body"],
        ),
    };
    json!({"type": "function", "function": {
        "name": name,
        "description": description,
        "parameters": {"type": "object", "properties": properties, "required": required},
    }})
}

/// A text argument, trimmed; `None` when missing or blank.
fn text_arg(args: &Value, name: &str) -> Option<String> {
    args[name].as_str().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string)
}

/// A thread id, as text or a number, with or without its `[t:…]` mark.
fn id_arg(args: &Value, name: &str) -> Option<String> {
    let id = match &args[name] {
        Value::Number(n) => n.to_string(),
        other => other.as_str()?.trim().trim_matches(|c| c == '[' || c == ']').trim_start_matches("t:").to_string(),
    };
    Some(id).filter(|id| !id.is_empty())
}

/// The ids in an action's arguments as the app has them: the model may write a thread it saw as
/// `[t:41]` or `t:41`, or as a number.
fn plain_ids(args: &Value) -> Value {
    let mut args = args.clone();
    if let Value::Object(map) = &mut args {
        for key in ["thread_id", "reply_to_thread", "forward_thread", "draft_id", "scheduled_id"] {
            if let Some(id) = id_arg(&Value::Object(map.clone()), key) {
                map.insert(key.into(), Value::String(id));
            }
        }
    }
    args
}

/// Lines joined, as many as fit in [`LIST_CHARS`].
fn fit_lines(lines: Vec<String>) -> String {
    let mut text = String::new();
    for line in lines {
        if text.chars().count() + line.chars().count() + 1 > LIST_CHARS {
            break;
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&line);
    }
    text
}

/// "Dana Lee" or, with no name, the address.
fn name_of(p: &Person) -> &str {
    if p.name.is_empty() { &p.email } else { &p.name }
}

/// "Re: Q4 budget", without piling up "Re:"s.
fn reply_subject(subject: &str) -> String {
    let mut rest = subject.trim();
    while rest.get(..3).is_some_and(|head| head.eq_ignore_ascii_case("re:")) {
        rest = rest[3..].trim_start();
    }
    format!("Re: {rest}")
}

/// Plain text as HTML, keeping line breaks (like the engine's `text_to_html`).
fn html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\n', "<br>")
}

/// A random id in the shape of a UUID, like the drafts the composer makes.
fn new_id() -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("the system has randomness");
    let hex: String = b.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &hex[..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..])
}

/// Emails offered to choose from.
const MATCHES: usize = 5;

/// How the model picks the email an action is about, from what Moss's search found.
const PICK: &str = "You pick which email an action in the user's mail app is about. Each choice shows its id, subject, sender and how it stands. \
Pick the one the user means, thinking about what the action does: stopping a reminder is about an email that's coming back later, marking Done about one that isn't Done yet, a reply about the email being answered. \
Answer \"none\" when none of them is it.";

/// How the model says whether a new email was meant as a reply to the open conversation.
const KIND: &str = "You check what kind of email an assistant in the user's mail app should write. The user has an email open, \
and the assistant wrote a new email. Say whether the user asked for a reply to the email they have open (it goes in that \
conversation, to the person who sent it), or for a new email of its own.";


/// An email as a list of choices shows it to the model, with how it stands.
pub fn match_line(t: &ThreadSummary) -> String {
    let stands = if t.done {
        " (Done)"
    } else if t.snoozed_until.is_some() {
        " (coming back later)"
    } else {
        ""
    };
    format!("[t:{}] “{}” from {}{stands}", t.id, t.subject, name_of(&t.from))
}

impl MailTools<'_> {
    /// The emails Moss's search finds for `text` (by words, then by meaning), at most [`MATCHES`].
    pub async fn likely(&self, text: &str) -> Vec<ThreadSummary> {
        let mut threads: Vec<ThreadSummary> = vec![];
        for hit in self.mail.search(text, MATCHES).await.unwrap_or_default() {
            if !threads.iter().any(|t| t.id == hit.thread.id) {
                threads.push(hit.thread);
            }
        }
        threads
    }


    /// An action's email, where the model gave none or something that isn't an email's id (a
    /// subject, a name): Moss searches for it, or for what the user asked, and the model picks
    /// the one meant. When it can't, the choices go back to the model to call again.
    async fn resolved(&self, name: &str, mut args: Value) -> Result<Value, ToolOutput> {
        let spec = self.actions.map(|a| a.specs()).unwrap_or_default().into_iter().find(|s| s["function"]["name"] == name);
        let required = spec.and_then(|s| s["function"]["parameters"]["required"].as_array().cloned()).unwrap_or_default();
        for key in ["thread_id", "reply_to_thread", "forward_thread"] {
            let given = args[key].as_str().map(str::trim).filter(|g| !g.is_empty()).map(str::to_string);
            let needed = given.is_none() && required.iter().any(|r| r == key);
            if !needed && given.as_deref().is_none_or(|g| self.mail.thread(g).is_ok()) {
                continue;
            }
            let looked_for = given.unwrap_or_else(|| self.question.clone());
            let choices = self.likely(&looked_for).await;
            if choices.is_empty() {
                return Err(ToolOutput::text(format!(
                    "There's no email “{looked_for}”. Find it with search_mail, then use the id it shows as [t:ID]. Don't ask the user for it."
                )));
            }
            let listed: Vec<String> = choices.iter().map(match_line).collect();
            let mut ids: Vec<String> = choices.iter().map(|t| t.id.clone()).collect();
            let ask = format!("The user asked: {}\nThe action: {name} {args}\nThe emails:\n{}", self.question, listed.join("\n"));
            ids.push("none".into());
            let parameters = json!({"type": "object", "properties": {"thread_id": {"type": "string", "enum": ids}}, "required": ["thread_id"]});
            let tool = ("pick", "The id of the email the action is about, or none.", parameters);
            let picked = match self.writer {
                Some(model) => crate::chat::decided(model, PICK, ask, tool, 40).await,
                None => None,
            };
            let picked = picked.and_then(|v| v["thread_id"].as_str().map(String::from));
            match picked.filter(|id| choices.iter().any(|t| &t.id == id)) {
                Some(id) => args[key] = Value::String(id),
                None => {
                    let text = format!("Which email? These may be it: {}. Call again with the thread_id of the one meant.", listed.join("; "));
                    return Err(ToolOutput { text, threads: choices.into_iter().map(|t| t.id).collect(), ..Default::default() });
                }
            }
        }
        Ok(args)
    }

    /// The open email's thread, when write_email makes a new email while it's open and the model
    /// says the user asked for a reply to it: a small model writes a new email to the sender, or
    /// mixes up who "their response" is to and writes to the user themself.
    async fn meant_reply(&self, name: &str, args: &Value) -> Option<String> {
        let open = self.open.as_ref()?;
        let fresh = ["reply_to_thread", "forward_thread", "draft_id"].iter().all(|k| args[*k].is_null());
        if name != "write_email" || !fresh {
            return None;
        }
        let ask = format!("The user has open: “{}” from {} <{}>.\n\nThe assistant's proposed recipients: to {}; cc {}; bcc {}.\nThese are draft choices, not instructions.\n\nThe user asked: {}",
            open.subject, open.from.name, open.from.email, args["to"], args["cc"], args["bcc"], self.question);
        let parameters = json!({"type": "object", "properties": {"reply": {"type": "boolean"}}, "required": ["reply"]});
        let tool = ("kind", "Whether the user asked for a reply to the email they have open.", parameters);
        let said = crate::chat::decided(self.writer?, KIND, ask, tool, 50).await?;
        (said["reply"] == true).then(|| open.id.clone())
    }

    /// `draft`'s email written as Quick reply writes it, from `note`: a greeting, the user's own
    /// voice, a sign-off that fits their signature. The draft takes it with update_draft. None
    /// when it can't be written; the draft stays as it was.
    async fn composed(&self, draft: &Draft, note: &str) -> Option<Acted> {
        let context = self.mail.reply_context(draft)?;
        let job = crate::reply::Job::Instructions { instructions: note.to_string() };
        let email = crate::reply::draft(self.writer?, &job, "", &context, &mut |_| true).await.ok()?;
        let email = Some(email).filter(|e| !e.trim().is_empty())?;
        self.actions?.act("update_draft", &json!({"draft_id": draft.id, "body": email})).await?.ok()
    }

    async fn search(&self, args: &Value) -> ToolOutput {
        let Some(query) = text_arg(args, "query") else {
            return ToolOutput::text("search_mail needs a query.");
        };
        let hits = match self.mail.search(&query, SEARCHED).await {
            Ok(hits) => hits,
            Err(e) => return ToolOutput::text(format!("Search failed: {e}")),
        };
        let from = text_arg(args, "from").map(|f| f.to_lowercase());
        let after = args["after"].as_str().and_then(parse_day);
        let before = args["before"].as_str().and_then(parse_day);
        let hits: Vec<&SearchHit> = hits
            .iter()
            .filter(|h| {
                let t = &h.thread;
                let who = format!("{} {}", t.from.name, t.from.email).to_lowercase();
                from.as_ref().is_none_or(|f| who.contains(f))
                    && after.is_none_or(|a| t.last_message_at >= a)
                    && before.is_none_or(|b| t.last_message_at < b)
            })
            .take(HITS)
            .collect();
        if hits.is_empty() {
            return ToolOutput::text(format!("No mail found for “{query}”."));
        }
        let mut files = vec![];
        let lines = hits
            .iter()
            .map(|h| {
                let t = &h.thread;
                let snippet: String = h.snippet.iter().map(|p| p.text.as_str()).collect();
                let snippet = if snippet.trim().is_empty() { t.summary.clone() } else { snippet };
                let snippet = cut(&snippet.split_whitespace().collect::<Vec<_>>().join(" "), 200);
                let found = match &h.file {
                    Some(f) => {
                        let file = FileSource { thread_id: t.id.clone(), message_id: h.message_id.clone(), index: f.index, name: f.name.clone() };
                        let line = format!("in the file {} [f:{}]: {snippet}", f.name, file.id());
                        files.push(file);
                        line
                    }
                    None => snippet,
                };
                format!("[t:{}] {} · {} · {}\n  {found}", t.id, day(t.last_message_at), crate::prompt::sender(&t.from), t.subject)
            })
            .collect();
        ToolOutput { text: fit_lines(lines), threads: hits.iter().map(|h| h.thread.id.clone()).collect(), files, ..Default::default() }
    }

    async fn read(&self, args: &Value) -> ToolOutput {
        let Some(id) = id_arg(args, "thread_id") else {
            return ToolOutput::text("read_thread needs a thread_id.");
        };
        match self.mail.read_thread(&id).await {
            Ok(thread) => ToolOutput {
                text: thread_text(&thread, self.read_chars()),
                threads: vec![id.clone()],
                opened: vec![id],
                files: thread_files(&thread),
                ..Default::default()
            },
            Err(e) => ToolOutput::text(format!("Couldn’t open thread {id}: {e}")),
        }
    }

    async fn read_file(&self, args: &Value) -> ToolOutput {
        let Some((message_id, index)) = args["file_id"].as_str().and_then(parse_file_id) else {
            return ToolOutput::text("read_attachment needs a file_id, from [f:ID].");
        };
        let id = file_id(&message_id, index);
        let hit = match self.mail.file(&message_id, index) {
            Ok(hit) => hit,
            Err(e) => return ToolOutput::text(format!("Couldn’t find file {id}: {e}")),
        };
        let head = format!(
            "File [f:{id}] {} from {} on {}, in [t:{}] “{}”",
            hit.file.name,
            name_of(&hit.from),
            day(hit.sent_at),
            hit.thread_id,
            hit.subject
        );
        let text = match self.mail.file_text(&message_id, index).await {
            Ok(Some(text)) if !text.trim().is_empty() => format!("{head}:\n\n{text}"),
            Ok(Some(_)) => format!("{head}: there is no text in it."),
            Ok(None) => format!("{head}: Moss can’t read the text of this file."),
            Err(e) => return ToolOutput::text(format!("Couldn’t read {}: {e}", hit.file.name)),
        };
        let file = FileSource { thread_id: hit.thread_id.clone(), message_id, index, name: hit.file.name };
        ToolOutput {
            text,
            threads: vec![hit.thread_id.clone()],
            opened: vec![hit.thread_id],
            files: vec![file.clone()],
            read_files: vec![file],
            ..Default::default()
        }
    }

    fn files(&self, args: &Value) -> ToolOutput {
        let query = FileQuery {
            from: text_arg(args, "from"),
            name: text_arg(args, "name"),
            kind: args["type"].as_str().and_then(FileKind::parse),
            after: args["after"].as_str().and_then(parse_day),
            before: args["before"].as_str().and_then(parse_day),
            limit: FILES,
        };
        let files = match self.mail.files(&query) {
            Ok(files) => files,
            Err(e) => return ToolOutput::text(format!("Couldn’t list files: {e}")),
        };
        if files.is_empty() {
            return ToolOutput::text("No files found.");
        }
        let lines = files
            .iter()
            .map(|f| {
                let kind = FileKind::of(&f.file.name, &f.file.content_type).name();
                format!(
                    "[t:{}] [f:{}] {} ({kind}, {}) from {} on {} · “{}”",
                    f.thread_id,
                    file_id(&f.message_id, f.file.index),
                    f.file.name,
                    f.file.size,
                    name_of(&f.from),
                    day(f.sent_at),
                    f.subject
                )
            })
            .collect();
        let mut threads: Vec<String> = vec![];
        for f in &files {
            if !threads.contains(&f.thread_id) {
                threads.push(f.thread_id.clone());
            }
        }
        let files = files
            .into_iter()
            .map(|f| FileSource { thread_id: f.thread_id, message_id: f.message_id, index: f.file.index, name: f.file.name })
            .collect();
        ToolOutput { text: fit_lines(lines), threads, files, ..Default::default() }
    }

    fn people(&self, args: &Value) -> ToolOutput {
        let Some(query) = text_arg(args, "query") else {
            return ToolOutput::text("find_people needs a query.");
        };
        match self.mail.people(&query, PEOPLE) {
            Ok(people) if people.is_empty() && query.contains('@') => {
                ToolOutput::text(format!("No one in the user's mail has the address {query} yet. It's an email address: use it as it is."))
            }
            Ok(people) if people.is_empty() => ToolOutput::text(format!("Nobody found for “{query}”.")),
            Ok(people) => ToolOutput::text(fit_lines(people.iter().map(known).collect())),
            Err(e) => ToolOutput::text(format!("Couldn’t look up people: {e}")),
        }
    }

    fn draft(&self, args: &Value) -> ToolOutput {
        let (Some(id), Some(body)) = (id_arg(args, "thread_id"), text_arg(args, "body")) else {
            return ToolOutput::text("draft_reply needs a thread_id and a body.");
        };
        let thread = match self.mail.thread(&id) {
            Ok(thread) => thread,
            Err(e) => return ToolOutput::text(format!("Couldn’t open thread {id}: {e}")),
        };
        let draft = reply_draft(&thread, &body, self.now);
        if let Err(e) = self.mail.save_draft(&draft) {
            return ToolOutput::text(format!("Couldn’t save the draft: {e}"));
        }
        let to: Vec<String> = draft.to.iter().map(crate::prompt::sender).collect();
        ToolOutput {
            text: format!(
                "Saved a draft reply to {} on “{}”. It is not sent: the user checks it first.",
                to.join(", "),
                thread.summary.subject
            ),
            threads: vec![id.clone()],
            opened: vec![id],
            draft: Some(draft),
            ..Default::default()
        }
    }
}

#[async_trait]
impl Tools for MailTools<'_> {
    fn specs(&self) -> Vec<Value> {
        let mut specs: Vec<Value> = self.only.iter().map(|name| spec(name)).collect();
        specs.extend(self.actions.map(|a| a.specs()).unwrap_or_default());
        specs
    }

    fn label(&self, name: &str, args: &Value) -> String {
        match (name, text_arg(args, "query").or_else(|| text_arg(args, "from"))) {
            ("search_mail", Some(q)) => format!("Searching mail for “{q}”"),
            ("search_mail", None) => "Searching mail".into(),
            ("read_thread", _) => "Reading an email".into(),
            ("list_attachments", Some(from)) => format!("Looking for files from {from}"),
            ("list_attachments", None) => "Looking for files".into(),
            ("read_attachment", _) => "Reading a file".into(),
            ("find_people", Some(q)) => format!("Looking up {q}"),
            ("find_people", None) => "Looking up people".into(),
            ("draft_reply", _) => "Writing a draft reply".into(),
            _ => self.actions.and_then(|a| a.label(name, args)).unwrap_or_else(|| "Working".into()),
        }
    }

    async fn call(&self, name: &str, args: &Value) -> ToolOutput {
        if !self.only.contains(&name) {
            let mut args = match self.resolved(name, plain_ids(args)).await {
                Ok(args) => args,
                Err(out) => return out,
            };
            // A new email to the sender of the email on screen, where a small model may have meant
            // the reply the user asked for: the model says which, and a reply goes in its thread.
            if let Some(open) = self.meant_reply(name, &args).await {
                // To who wrote it, with its thread's "Re:" subject.
                (args["reply_to_thread"], args["subject"], args["to"]) = (Value::String(open), Value::Null, Value::Null);
            }
            // An email is written as Quick reply writes it: what the model put in write_email (or,
            // with nothing, what the user asked) is the note it writes from.
            let unwritten = args["body"].as_str().is_none_or(|b| b.trim().is_empty());
            // Quick reply writes to the sender; a forward goes to someone else, so it keeps its own words.
            let forward = args["forward_thread"].is_string() && args["reply_to_thread"].is_null();
            let note = if unwritten { self.question.clone() } else { args["body"].as_str().unwrap_or_default().to_string() };
            let composing = name == "write_email" && !forward;
            if composing && unwritten {
                args["body"] = Value::String(note.clone());
            }
            let acted = match self.actions {
                Some(actions) => actions.act(name, &args).await,
                None => None,
            };
            return match acted {
                None => ToolOutput::text(format!("There is no tool called {name}.")),
                Some(Err(e)) => ToolOutput::text(format!("Couldn’t do that: {e}")),
                Some(Ok(mut a)) => {
                    // The model keeps hearing what write_email said: which kind of email, and
                    // that it's under the answer. Not the written email, which it would paste.
                    if let (true, Some(draft)) = (composing, &a.draft) {
                        if let Some(rewritten) = self.composed(draft, &note).await {
                            a.draft = rewritten.draft.or(a.draft);
                        }
                    }
                    let opened = a.draft.iter().filter_map(|d| d.thread_id.clone()).collect();
                    let text = cut(&a.text, self.read_chars());
                    ToolOutput { text, threads: a.threads, opened, draft: a.draft, send_at: a.send_at, did: a.did, ..Default::default() }
                }
            };
        }
        let mut out = match name {
            "search_mail" => self.search(args).await,
            "read_thread" => self.read(args).await,
            "list_attachments" => self.files(args),
            "read_attachment" => self.read_file(args).await,
            "find_people" => self.people(args),
            _ => self.draft(args),
        };
        out.text = cut(&out.text, self.read_chars());
        out
    }

    fn sources(&self, ids: &[String]) -> Vec<ThreadSummary> {
        ids.iter().filter_map(|id| self.mail.thread(id).ok()).map(|t| t.summary).collect()
    }
}

/// A thread as the model reads it, in at most `max_chars`: the subject, then each message with
/// its sender, date, recipients and attachment names. When it's too long the oldest messages
/// are left out first, and then the newest is cut.
pub fn thread_text(thread: &Thread, max_chars: usize) -> String {
    thread_text_marked(thread, max_chars, false)
}

/// [`thread_text`] with each message marked `[m:ID]`, for answers that cite which message they
/// came from (Ask about this email).
pub fn cited_thread_text(thread: &Thread, max_chars: usize) -> String {
    thread_text_marked(thread, max_chars, true)
}

fn thread_text_marked(thread: &Thread, max_chars: usize, cite: bool) -> String {
    let head = format!("Thread [t:{}]: {}\n\n", thread.summary.id, thread.summary.subject);
    let blocks: Vec<String> = thread
        .messages
        .iter()
        .map(|m| if cite { format!("[m:{}] {}", m.id, message_text(m)) } else { message_text(m) })
        .collect();
    let all = blocks.len();
    for keep in (1..=all).rev() {
        let left_out = all - keep;
        let note = match left_out {
            0 => String::new(),
            1 => "(1 earlier message left out)\n\n".to_string(),
            n => format!("({n} earlier messages left out)\n\n"),
        };
        let text = format!("{head}{note}{}", blocks[left_out..].join("\n\n---\n\n"));
        if text.chars().count() <= max_chars || keep == 1 {
            return cut(&text, max_chars);
        }
    }
    cut(head.trim_end(), max_chars)
}

/// One message of a thread, as [`thread_text`] shows it.
fn message_text(m: &moss_core::Message) -> String {
    let list = |people: &[Person]| people.iter().map(crate::prompt::sender).collect::<Vec<_>>().join(", ");
    let mut text = format!("From: {} · {}", crate::prompt::sender(&m.from), when(m.sent_at));
    for (label, people) in [("To", &m.to), ("Cc", &m.cc)] {
        if !people.is_empty() {
            text.push_str(&format!("\n{label}: {}", list(people)));
        }
    }
    let files: Vec<&moss_core::Attachment> = m.attachments.iter().filter(|a| !a.inline).collect();
    let named: Vec<String> = files.iter().map(|a| format!("{} [f:{}]", a.name, file_id(&m.id, a.index))).collect();
    if !named.is_empty() {
        text.push_str(&format!("\nAttachments: {}", named.join(", ")));
    }
    let body = if !m.paragraphs.is_empty() {
        m.paragraphs.join("\n\n")
    } else if !m.body_text.trim().is_empty() {
        m.body_text.trim().to_string()
    } else {
        if m.headers_only { "(Not downloaded yet.)" } else { "" }.to_string()
    };
    let mut inside = String::new();
    for a in files {
        if let Some(words) = a.text.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            inside.push_str(&format!("\n\nInside the attachment {} [f:{}]:\n{}", a.name, file_id(&m.id, a.index), cut(words, FILE_CHARS)));
        }
    }
    format!("{text}\n\n{body}{inside}")
}

/// A reply to `thread` with `body`, from the account the thread is in, to its sender.
pub fn reply_draft(thread: &Thread, body: &str, now: i64) -> Draft {
    let placeholder = |l: &str| {
        let l = l.trim();
        l.starts_with('[') && l.ends_with(']')
    };
    let mut body = body.lines().filter(|l| !placeholder(l)).collect::<Vec<_>>().join("\n").trim().to_string();
    // The model sometimes signs as itself; the user's own signature goes under the reply.
    if let Some(cut) = body.strip_suffix("Moss").filter(|rest| rest.ends_with([',', '\n', ' '])) {
        body = cut.trim_end().to_string();
    }
    Draft {
        revision: None,
        id: new_id(),
        mode: ComposeMode::Reply,
        thread_id: Some(thread.summary.id.clone()),
        from_account_id: thread.summary.account_id.clone(),
        to: vec![thread.summary.from.clone()],
        cc: vec![],
        bcc: vec![],
        subject: reply_subject(&thread.summary.subject),
        body_html: html(&body),
        body,
        remind_if_no_reply: false,
        updated_at: now,
        signature_id: None,
        signature_chosen: false,
        from_address: None,
        attachments: vec![],
    }
}

/// "2026-11-01", in UTC.
/// The user's UTC offset at `ms`, as RFC 3339 writes it: "+01:00".
pub fn zone(ms: i64) -> String {
    use chrono::{Offset, TimeZone};
    let utc = chrono::DateTime::from_timestamp_millis(ms).unwrap_or_default().naive_utc();
    let secs = chrono::Local.offset_from_utc_datetime(&utc).fix().local_minus_utc();
    format!("{}{:02}:{:02}", if secs < 0 { '-' } else { '+' }, secs.abs() / 3600, secs.abs() % 3600 / 60)
}

pub fn day(ms: i64) -> String {
    let (y, m, d) = civil(ms.div_euclid(DAY_MS));
    format!("{y:04}-{m:02}-{d:02}")
}

const DAY_MS: i64 = 86_400_000;

/// The (year, month, day) of a day counted from 1970-01-01 (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The day counted from 1970-01-01 of a (year, month, day).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// "2026-11-01 09:12", in UTC.
pub fn when(ms: i64) -> String {
    let minutes = ms.rem_euclid(DAY_MS) / 60_000;
    format!("{} {:02}:{:02}", day(ms), minutes / 60, minutes % 60)
}

/// The start of a "YYYY-MM-DD" day (ms, UTC), or `None` if it isn't one.
pub fn parse_day(text: &str) -> Option<i64> {
    let parts: Vec<i64> = text.trim().split('-').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let [y, m, d] = parts[..] else {
        return None;
    };
    let days = days_from_civil(y, m, d);
    (civil(days) == (y, m, d)).then_some(days * DAY_MS)
}

/// At most `max` characters, with "…" when cut.
pub fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", kept.trim_end())
}

/// Someone found, with how often and when last the user and they wrote, so a model takes the
/// address the user writes to now over an old one.
fn known(c: &Contact) -> String {
    let ways = match (c.to_them, c.from_them) {
        (0, heard) => format!("they wrote you {heard} times"),
        (wrote, 0) => format!("you wrote them {wrote} times"),
        (wrote, heard) => format!("you wrote them {wrote} times and they wrote you {heard}"),
    };
    let last = chrono::DateTime::from_timestamp_millis(c.last_at).unwrap_or_default().format("%Y-%m-%d");
    format!("{}: {ways}, last on {last}", crate::prompt::sender(&c.person))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_mail::*;

    fn tools(mail: &FakeMail) -> MailTools<'_> {
        MailTools::new(mail, NOV_1)
    }

    #[test]
    fn dates_are_written_and_read_as_days_in_utc() {
        assert_eq!(day(NOV_1), "2026-11-01");
        assert_eq!(when(NOV_1), "2026-11-01 09:12");
        assert_eq!(day(0), "1970-01-01");
        assert_eq!(day(951_782_400_000), "2000-02-29");
        assert_eq!(day(-DAY), "1969-12-31");
        assert_eq!(parse_day("2026-11-01"), Some(NOV_1 - (9 * 60 + 12) * 60_000));
        assert_eq!(parse_day(" 2000-02-29 "), Some(951_782_400_000));
        assert_eq!(parse_day("1970-01-01"), Some(0));
        for bad in ["2026-02-30", "2026-13-01", "2026-00-10", "yesterday", "2026-11", "2026-11-01-02", ""] {
            assert_eq!(parse_day(bad), None, "{bad}");
        }
    }

    #[test]
    fn cuts_long_text_with_an_ellipsis() {
        assert_eq!(cut("short", 10), "short");
        assert_eq!(cut("abcdéfgh", 5), "abcd…");
    }

    #[test]
    fn offers_every_tool_or_only_the_ones_asked_for() {
        let mail = FakeMail::new();
        let names = |t: &MailTools| -> Vec<String> {
            t.specs().iter().map(|s| s["function"]["name"].as_str().unwrap().to_string()).collect()
        };
        assert_eq!(names(&tools(&mail)), ALL);
        assert_eq!(names(&tools(&mail).only(&["draft_reply"])), ["draft_reply"]);
        for s in tools(&mail).specs() {
            assert_eq!(s["type"], "function");
            assert!(s["function"]["description"].as_str().unwrap().len() > 20);
            assert_eq!(s["function"]["parameters"]["type"], "object");
        }
        let files = spec("list_attachments");
        let kinds: Vec<&str> = files["function"]["parameters"]["properties"]["type"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| k.as_str().unwrap())
            .collect();
        assert_eq!(kinds, FileKind::ALL.map(FileKind::name));
    }

    #[tokio::test]
    async fn a_tool_that_is_not_on_offer_does_nothing() {
        let mail = FakeMail::new();
        let out = tools(&mail).only(&["read_thread"]).call("draft_reply", &json!({"thread_id": "41", "body": "Hi"})).await;
        assert_eq!(out, ToolOutput::text("There is no tool called draft_reply."));
        assert!(mail.saved.lock().unwrap().is_empty());
        let out = tools(&mail).call("send_email", &json!({})).await;
        assert_eq!(out.text, "There is no tool called send_email.");
    }

    #[test]
    fn labels_say_what_it_is_doing() {
        let mail = FakeMail::new();
        let t = tools(&mail);
        assert_eq!(t.label("search_mail", &json!({"query": "Q4 budget"})), "Searching mail for “Q4 budget”");
        assert_eq!(t.label("search_mail", &json!({})), "Searching mail");
        assert_eq!(t.label("read_thread", &json!({"thread_id": "41"})), "Reading an email");
        assert_eq!(t.label("list_attachments", &json!({"from": "Dana"})), "Looking for files from Dana");
        assert_eq!(t.label("list_attachments", &json!({})), "Looking for files");
        assert_eq!(t.label("find_people", &json!({"query": "Dana"})), "Looking up Dana");
        assert_eq!(t.label("find_people", &json!({})), "Looking up people");
        assert_eq!(t.label("draft_reply", &json!({})), "Writing a draft reply");
        assert_eq!(t.label("other", &json!({})), "Working");
    }

    #[tokio::test]
    async fn search_lists_threads_with_ids_and_filters_by_sender_and_date() {
        let mail = FakeMail::new();
        let t = tools(&mail);
        let out = t.call("search_mail", &json!({"query": "budget"})).await;
        assert_eq!(out.threads, ["41", "57"]);
        assert!(out.opened.is_empty());
        let first = out.text.lines().next().unwrap();
        assert_eq!(first, "[t:41] 2026-10-29 · Dana Lee <dana@northwind.co> · Q4 budget");
        assert!(out.text.contains("The final Q4 budget is 182k"), "{}", out.text);

        let from = t.call("search_mail", &json!({"query": "budget", "from": "SAM"})).await;
        assert_eq!(from.threads, ["57"]);
        let after = t.call("search_mail", &json!({"query": "budget", "after": "2026-10-30"})).await;
        assert_eq!(after.threads, ["57"]);
        let before = t.call("search_mail", &json!({"query": "budget", "before": "2026-10-30"})).await;
        assert_eq!(before.threads, ["41"]);

        let none = t.call("search_mail", &json!({"query": "holiday"})).await;
        assert_eq!(none, ToolOutput::text("No mail found for “holiday”."));
        let blank = t.call("search_mail", &json!({"query": "  "})).await;
        assert_eq!(blank.text, "search_mail needs a query.");
    }

    #[test]
    fn a_match_says_how_the_email_stands() {
        let mut t = budget().summary;
        assert_eq!(match_line(&t), "[t:41] “Q4 budget” from Dana Lee");
        t.snoozed_until = Some(NOV_1);
        assert_eq!(match_line(&t), "[t:41] “Q4 budget” from Dana Lee (coming back later)");
        t.done = true;
        assert_eq!(match_line(&t), "[t:41] “Q4 budget” from Dana Lee (Done)");
    }

    #[tokio::test]
    async fn search_shows_the_summary_when_a_hit_has_no_snippet() {
        let mut mail = FakeMail::new();
        mail.threads[0].messages[0].body_text = String::new();
        let out = tools(&mail).call("search_mail", &json!({"query": "Dana"})).await;
        assert!(out.text.contains("About Q4 budget"), "{}", out.text);
    }

    #[tokio::test]
    async fn a_writer_with_a_smaller_context_reads_less_of_a_thread() {
        let mut mail = FakeMail::new();
        for m in &mut mail.threads[0].messages {
            m.paragraphs = vec!["word ".repeat(1_600)];
        }
        let gemma = tools(&mail).call("read_thread", &json!({"thread_id": "41"})).await;
        assert!(!gemma.text.contains("left out"), "both messages, whole");
        let apple = crate::test_mail::ScriptedModel::new(vec![]).with_context(8_192);
        let small = MailTools::new(&mail, NOV_1).writing_with(&apple).call("read_thread", &json!({"thread_id": "41"})).await;
        assert!(small.text.contains("(1 earlier message left out)"), "{}", small.text);
        assert!(small.text.chars().count() <= crate::chat::room(READ_CHARS, &apple));
    }

    #[tokio::test]
    async fn reading_a_thread_gives_every_message_and_its_files() {
        let mail = FakeMail::new();
        let out = tools(&mail).call("read_thread", &json!({"thread_id": "41"})).await;
        assert_eq!((out.threads.clone(), out.opened.clone()), (vec!["41".to_string()], vec!["41".to_string()]));
        assert_eq!(out.text, thread_text(&budget(), READ_CHARS));
        let missing = tools(&mail).call("read_thread", &json!({"thread_id": "9"})).await;
        assert_eq!(missing, ToolOutput::text("Couldn’t open thread 9: no thread 9"));
        let none = tools(&mail).call("read_thread", &json!({})).await;
        assert_eq!(none.text, "read_thread needs a thread_id.");
        // Some models write the id as a number, or with the citation mark.
        assert_eq!(tools(&mail).call("read_thread", &json!({"thread_id": 41})).await.opened, ["41"]);
        assert_eq!(tools(&mail).call("read_thread", &json!({"thread_id": "[t:57]"})).await.opened, ["57"]);
    }

    #[test]
    fn a_thread_reads_oldest_first_with_who_when_and_what_files() {
        let text = thread_text(&budget(), 10_000);
        assert_eq!(
            text,
            "Thread [t:41]: Q4 budget\n\n\
             From: Dana Lee <dana@northwind.co> · 2026-10-28 09:12\nTo: Maya Ross <maya@northwind.co>\nAttachments: budget-q4.xlsx [f:m1-0]\n\n\
             Hi Maya,\n\nThe final Q4 budget is 182k. Can you confirm by Friday?\n\n---\n\n\
             From: Maya Ross <maya@northwind.co> · 2026-10-29 09:12\nTo: Maya Ross <maya@northwind.co>\n\n\
             Looks right. Checking with Sam."
        );
    }

    #[test]
    fn a_long_thread_leaves_out_the_oldest_messages_then_cuts_the_newest() {
        let mut long = budget();
        long.messages[1].cc = vec![dana()];
        let all = thread_text(&long, 10_000);
        assert!(all.contains("Cc: Dana Lee <dana@northwind.co>"));
        let newest_only = thread_text(&long, 260);
        assert!(newest_only.starts_with("Thread [t:41]: Q4 budget\n\n(1 earlier message left out)\n\n"), "{newest_only}");
        assert!(newest_only.contains("Checking with Sam."));
        assert!(!newest_only.contains("182k"));
        let tiny = thread_text(&long, 120);
        assert!(tiny.chars().count() <= 120, "{tiny}");
        assert!(tiny.ends_with('…'));
        let mut two = budget();
        two.messages.insert(0, message("m0", dana(), NOV_1 - 9 * DAY, "First."));
        assert!(thread_text(&two, 260).contains("(2 earlier messages left out)"));
    }

    #[test]
    fn a_thread_for_its_own_chat_marks_each_message_to_cite() {
        let text = cited_thread_text(&budget(), 10_000);
        assert!(text.contains("\n\n[m:m1] From: Dana Lee <dana@northwind.co>"), "{text}");
        assert!(text.contains("\n\n[m:m2] From: Maya Ross"), "{text}");
        assert!(!thread_text(&budget(), 10_000).contains("[m:"));
    }

    #[test]
    fn a_thread_with_no_messages_is_just_its_subject() {
        let empty = thread("41", "Q4 budget", dana(), vec![]);
        assert_eq!(thread_text(&empty, 100), "Thread [t:41]: Q4 budget");
    }

    #[test]
    fn a_message_not_downloaded_yet_says_so() {
        let mut t = budget();
        t.messages[1].headers_only = true;
        t.messages[1].paragraphs.clear();
        t.messages[1].body_text = String::new();
        assert!(thread_text(&t, 10_000).ends_with("(Not downloaded yet.)"));
        t.messages[1].headers_only = false;
        t.messages[1].body_text = "Plain only".into();
        assert!(thread_text(&t, 10_000).ends_with("Plain only"), "falls back to the plain text");
    }

    #[tokio::test]
    async fn files_are_listed_with_their_kind_sender_and_date() {
        let mail = FakeMail::new();
        let args = json!({"from": "Dana", "name": "budget", "type": "spreadsheet", "after": "2026-10-01", "before": "2026-11-01"});
        let out = tools(&mail).call("list_attachments", &args).await;
        assert_eq!(out.text, "[t:41] [f:m1-0] budget-q4.xlsx (spreadsheet, 48 KB) from Dana Lee on 2026-10-28 · “Q4 budget”");
        assert_eq!(out.threads, ["41"]);
        assert_eq!(out.files, [budget_file()]);
        assert!(out.read_files.is_empty(), "listed, not read");
        let asked = mail.asked.lock().unwrap().clone().unwrap();
        assert_eq!(
            asked,
            FileQuery {
                from: Some("Dana".into()),
                name: Some("budget".into()),
                kind: Some(FileKind::Spreadsheet),
                after: parse_day("2026-10-01"),
                before: parse_day("2026-11-01"),
                limit: FILES,
            }
        );
        // Unknown kinds and dates, and empty words, are no filter.
        tools(&mail).call("list_attachments", &json!({"type": "video", "after": "soon", "from": " "})).await;
        let asked = mail.asked.lock().unwrap().clone().unwrap();
        assert_eq!(asked, FileQuery { limit: FILES, ..Default::default() });

        let mut empty = FakeMail::new();
        empty.files.clear();
        assert_eq!(tools(&empty).call("list_attachments", &json!({})).await.text, "No files found.");
    }

    fn budget_file() -> FileSource {
        FileSource { thread_id: "41".into(), message_id: "m1".into(), index: 0, name: "budget-q4.xlsx".into() }
    }

    #[test]
    fn file_ids_are_the_message_and_index() {
        assert_eq!(budget_file().id(), "m1-0");
        assert_eq!(parse_file_id("4406-2"), Some(("4406".into(), 2)));
        assert_eq!(parse_file_id(" [f:4406-2] "), Some(("4406".into(), 2)));
        assert_eq!(parse_file_id("a-b-3"), Some(("a-b".into(), 3)));
        for bad in ["4406", "4406-x", "-2", "f:", ""] {
            assert_eq!(parse_file_id(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_thread_shows_the_text_inside_its_files_that_were_read() {
        let mut t = budget();
        let mut logo = file("logo.png");
        (logo.index, logo.inline, logo.text) = (1, true, Some("Northwind".into()));
        let mut blank = file("blank.png");
        (blank.index, blank.text) = (2, Some("  ".into()));
        t.messages[0].attachments[0].text = Some(format!("Total: 182k\n{}", "x".repeat(FILE_CHARS)));
        t.messages[0].attachments.extend([logo, blank]);
        let text = thread_text(&t, READ_CHARS);
        assert!(text.contains("Attachments: budget-q4.xlsx [f:m1-0], blank.png [f:m1-2]\n"), "{text}");
        assert!(text.contains("Can you confirm by Friday?\n\nInside the attachment budget-q4.xlsx [f:m1-0]:\nTotal: 182k\nxxx"), "{text}");
        assert!(text.contains("x…\n\n---"), "a long file is cut");
        assert!(!text.contains("Northwind") && !text.contains("Inside the attachment blank.png"), "{text}");
        assert_eq!(thread_files(&t).iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["budget-q4.xlsx", "blank.png"]);
    }

    #[tokio::test]
    async fn reading_a_file_gives_its_text_and_where_it_came_from() {
        let mut mail = FakeMail::new();
        mail.threads[0].messages[0].attachments[0].text = Some("Total: 182k".into());
        let out = tools(&mail).call("read_attachment", &json!({"file_id": "[f:m1-0]"})).await;
        assert_eq!(out.text, "File [f:m1-0] budget-q4.xlsx from Dana Lee on 2026-10-28, in [t:41] “Q4 budget”:\n\nTotal: 182k");
        assert_eq!((out.threads.clone(), out.opened.clone()), (vec!["41".to_string()], vec!["41".to_string()]));
        assert_eq!((out.files.clone(), out.read_files.clone()), (vec![budget_file()], vec![budget_file()]));
        // No words in it, or a file Moss can't read.
        mail.threads[0].messages[0].attachments[0].text = Some(" ".into());
        assert!(tools(&mail).call("read_attachment", &json!({"file_id": "m1-0"})).await.text.ends_with(": there is no text in it."));
        mail.threads[0].messages[0].attachments[0].text = None;
        assert!(tools(&mail).call("read_attachment", &json!({"file_id": "m1-0"})).await.text.ends_with(": Moss can’t read the text of this file."));
        // Mistakes, an unknown file, and a server that's offline.
        assert_eq!(tools(&mail).call("read_attachment", &json!({})).await.text, "read_attachment needs a file_id, from [f:ID].");
        assert_eq!(tools(&mail).call("read_attachment", &json!({"file_id": "m1-7"})).await.text, "Couldn’t find file m1-7: no file 7 in m1");
        mail.unreadable_files = true;
        let offline = tools(&mail).call("read_attachment", &json!({"file_id": "m1-0"})).await;
        assert_eq!(offline, ToolOutput::text("Couldn’t read budget-q4.xlsx: the server is offline"));
    }

    #[tokio::test]
    async fn a_search_match_inside_a_file_says_which_file() {
        struct InFile(FakeMail);
        #[async_trait]
        impl Mail for InFile {
            async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
                let mut hits = self.0.search(query, limit).await?;
                hits[0].file = Some(moss_core::MatchedFile { index: 0, name: "budget-q4.xlsx".into() });
                Ok(hits)
            }
            fn thread(&self, id: &str) -> Result<Thread, String> {
                self.0.thread(id)
            }
            fn files(&self, query: &FileQuery) -> Result<Vec<FileHit>, String> {
                self.0.files(query)
            }
            fn file(&self, message_id: &str, index: u32) -> Result<FileHit, String> {
                self.0.file(message_id, index)
            }
            async fn file_text(&self, message_id: &str, index: u32) -> Result<Option<String>, String> {
                self.0.file_text(message_id, index).await
            }
            fn people(&self, query: &str, limit: usize) -> Result<Vec<Contact>, String> {
                self.0.people(query, limit)
            }
            fn save_draft(&self, draft: &Draft) -> Result<(), String> {
                self.0.save_draft(draft)
            }
            fn reply_context(&self, draft: &Draft) -> Option<crate::reply::Context> {
                self.0.reply_context(draft)
            }
        }
        let mail = InFile(FakeMail::new());
        assert!(mail.reply_context(&reply_draft(&budget(), "Hi", NOV_1)).is_some());
        let out = MailTools::new(&mail, NOV_1).call("search_mail", &json!({"query": "budget"})).await;
        let second = out.text.lines().nth(1).unwrap();
        assert!(second.starts_with("  in the file budget-q4.xlsx [f:m1-0]: Hi Maya,"), "{second}");
        assert_eq!(out.files, [budget_file()]);
        // The default thread reader is the plain thread, and the rest pass through.
        assert_eq!(mail.read_thread("41").await.unwrap(), budget());
        assert_eq!(mail.people("dana", 1).unwrap()[0].person, dana());
        mail.save_draft(&reply_draft(&budget(), "Hi", NOV_1)).unwrap();
        assert_eq!(mail.files(&FileQuery::default()).unwrap()[0].file.name, "budget-q4.xlsx");
        assert_eq!(mail.file("m1", 0).unwrap().thread_id, "41");
        assert_eq!(mail.file_text("m1", 0).await.unwrap(), None);
    }

    #[tokio::test]
    async fn people_are_found_by_name() {
        let mail = FakeMail::new();
        let out = tools(&mail).call("find_people", &json!({"query": "dana"})).await;
        // How often and when last, so the model takes the address the user writes to now.
        assert_eq!(out, ToolOutput::text("Dana Lee <dana@northwind.co>: you wrote them 4 times and they wrote you 6, last on 2026-10-30"));
        let mut mail = FakeMail::new();
        mail.people = vec![
            Contact { person: Person { name: "Sam".into(), email: "sam@old.co".into() }, from_them: 0, to_them: 2, last_at: NOV_1 - 400 * DAY },
            Contact { person: Person { name: "Sam".into(), email: "sam@new.co".into() }, from_them: 3, to_them: 0, last_at: NOV_1 },
        ];
        let out = tools(&mail).call("find_people", &json!({"query": "sam"})).await.text;
        assert_eq!(out, "Sam <sam@old.co>: you wrote them 2 times, last on 2025-09-27\nSam <sam@new.co>: they wrote you 3 times, last on 2026-11-01");
        assert_eq!(tools(&mail).call("find_people", &json!({"query": "zed"})).await.text, "Nobody found for “zed”.");
        // An address the user gave needs no finding: a small model otherwise writes a name instead.
        let new = tools(&mail).call("find_people", &json!({"query": "sam@acme.co"})).await.text;
        assert_eq!(new, "No one in the user's mail has the address sam@acme.co yet. It's an email address: use it as it is.");
        assert_eq!(tools(&mail).call("find_people", &json!({})).await.text, "find_people needs a query.");
    }

    #[tokio::test]
    async fn a_draft_reply_is_saved_and_never_sent() {
        let mail = FakeMail::new();
        let args = json!({"thread_id": "41", "body": "Hi Dana,\n\nI confirm 182k.\n\nBest,\n[Your Name]"});
        let out = tools(&mail).call("draft_reply", &args).await;
        let saved = mail.saved.lock().unwrap().clone();
        assert_eq!(saved.len(), 1);
        assert_eq!(out.draft.as_ref(), Some(&saved[0]));
        assert_eq!((out.threads.clone(), out.opened.clone()), (vec!["41".to_string()], vec!["41".to_string()]));
        assert_eq!(
            out.text,
            "Saved a draft reply to Dana Lee <dana@northwind.co> on “Q4 budget”. It is not sent: the user checks it first."
        );
        let d = &saved[0];
        assert_eq!(d.body, "Hi Dana,\n\nI confirm 182k.\n\nBest,", "the made-up name line goes");
        assert_eq!(d.mode, ComposeMode::Reply);

        assert_eq!(tools(&mail).call("draft_reply", &json!({"thread_id": "41"})).await.text, "draft_reply needs a thread_id and a body.");
        assert_eq!(tools(&mail).call("draft_reply", &json!({"thread_id": "9", "body": "x"})).await.text, "Couldn’t open thread 9: no thread 9");
        let failing = FakeMail { failing: true, ..FakeMail::new() };
        let saving = tools(&failing).call("draft_reply", &json!({"thread_id": "41", "body": "x"})).await;
        assert_eq!(saving.text, "Couldn’t open thread 41: the database is locked");
        let full = FakeMail { refuse_drafts: true, ..FakeMail::new() };
        let saving = tools(&full).call("draft_reply", &json!({"thread_id": "41", "body": "x"})).await;
        assert_eq!(saving, ToolOutput::text("Couldn’t save the draft: the disk is full"));
    }

    #[test]
    fn a_reply_goes_to_the_sender_from_the_threads_account() {
        let d = reply_draft(&budget(), "  Yes.\n[Your name]\n", NOV_1);
        assert_eq!(d.thread_id.as_deref(), Some("41"));
        assert_eq!(d.from_account_id, "work");
        assert_eq!(d.to, [dana()]);
        assert!(d.cc.is_empty() && d.bcc.is_empty());
        assert_eq!(d.subject, "Re: Q4 budget");
        assert_eq!(d.body, "Yes.");
        assert_eq!(d.body_html, "Yes.");
        assert_eq!(d.updated_at, NOV_1);
        assert!(!d.remind_if_no_reply);
        assert_eq!((d.signature_id.clone(), d.signature_chosen), (None, false));
        assert_eq!(reply_draft(&approval(), "<b>&", NOV_1).subject, "Re: Q4 budget", "no Re: Re:");
        assert_eq!(reply_subject("Noël"), "Re: Noël", "a letter of more than one byte across the third byte");
        assert_eq!(reply_subject("re: Grüße"), "Re: Grüße");
        assert_eq!(reply_draft(&budget(), "Yes.\n\nBest,\nMoss", NOV_1).body, "Yes.\n\nBest,", "the model doesn't sign as itself");
        assert_eq!(reply_draft(&budget(), "Best, Moss", NOV_1).body, "Best,");
        assert_eq!(reply_draft(&budget(), "Moss is great", NOV_1).body, "Moss is great");
        assert_eq!(reply_draft(&approval(), "a <b> & \"c\"\nd", NOV_1).body_html, "a &lt;b&gt; &amp; &quot;c&quot;<br>d");
        let id = reply_draft(&budget(), "x", NOV_1).id;
        assert_eq!(id.len(), 36);
        assert_eq!(id.matches('-').count(), 4);
        assert_ne!(id, reply_draft(&budget(), "x", NOV_1).id);
    }

    #[tokio::test]
    async fn a_lookup_that_fails_tells_the_model_why() {
        let mail = FakeMail { failing: true, ..FakeMail::new() };
        let t = tools(&mail);
        assert_eq!(t.call("search_mail", &json!({"query": "x"})).await.text, "Search failed: the database is locked");
        assert_eq!(t.call("list_attachments", &json!({})).await.text, "Couldn’t list files: the database is locked");
        assert_eq!(t.call("find_people", &json!({"query": "x"})).await.text, "Couldn’t look up people: the database is locked");
    }

    #[test]
    fn sources_are_the_threads_in_order_without_the_missing_ones() {
        let mail = FakeMail::new();
        let ids: Vec<String> = ["57", "9", "41"].map(String::from).to_vec();
        let found: Vec<String> = tools(&mail).sources(&ids).into_iter().map(|t| t.id).collect();
        assert_eq!(found, ["57", "41"]);
    }

    #[tokio::test]
    async fn a_long_email_reaches_the_model_whole() {
        let long = "Every word of this matters. ".repeat(1_100); // about 31,000 characters
        let mut mail = FakeMail::new();
        mail.threads[0].messages[0].paragraphs = vec![long.trim().to_string()];
        let out = tools(&mail).call("read_thread", &json!({"thread_id": "41"})).await;
        assert!(out.text.contains(long.trim()), "the whole email, not its start");
        assert!(out.text.contains("Looks right. Checking with Sam."), "and the reply after it");
    }

    #[tokio::test]
    async fn long_results_are_cut_to_fit() {
        let mut mail = FakeMail::new();
        mail.people = (0..200).map(|n| Contact { person: person(&format!("Dana {n}"), &format!("dana{n}@x.co")), from_them: 1, to_them: 0, last_at: NOV_1 }).collect();
        let out = tools(&mail).call("find_people", &json!({"query": "dana"})).await;
        assert_eq!(out.text.lines().count(), PEOPLE);
        mail.files = (0..300).map(|_| mail.files[0].clone()).collect();
        let out = tools(&mail).call("list_attachments", &json!({})).await;
        assert!(out.text.chars().count() <= LIST_CHARS);
    }
}
