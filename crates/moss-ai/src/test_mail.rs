//! A mailbox in memory, and a model that answers from a script, for the Ask Moss tests.

use std::collections::VecDeque;
use std::sync::Mutex;

use async_trait::async_trait;
use moss_core::{Attachment, Contact, Draft, FileHit, FileQuery, Message, Person, SearchHit, SnippetPart, Thread, ThreadSummary};

use crate::chat::{ChatModel, ChatRequest, OnText, Reply, ToolCall};
use crate::reply::Context;
use crate::tools::{reply_draft, Acted, Actions, Mail};
use crate::AiError;

/// 1 November 2026, 09:12 UTC.
pub const NOV_1: i64 = 1_793_524_320_000;
pub const DAY: i64 = 86_400_000;

pub fn person(name: &str, email: &str) -> Person {
    Person { name: name.into(), email: email.into() }
}

pub fn dana() -> Person {
    person("Dana Lee", "dana@northwind.co")
}

pub fn maya() -> Person {
    person("Maya Ross", "maya@northwind.co")
}

pub fn message(id: &str, from: Person, at: i64, text: &str) -> Message {
    Message {
        id: id.into(),
        thread_id: "41".into(),
        from,
        to: vec![maya()],
        cc: vec![],
        bcc: vec![],
        subject: "Q4 budget".into(),
        sent_at: at,
        paragraphs: text.split("\n\n").map(str::to_string).collect(),
        body_text: text.into(),
        body_html: None,
        attachments: vec![],
        read: true,
        image_trust: Default::default(),
        sender_unverified: false,
        headers_only: false,
        images_missing: false,
        forwards: vec![],
        forwarded: false,
    }
}

pub fn file(name: &str) -> Attachment {
    Attachment { index: 0, name: name.into(), size: "48 KB".into(), bytes: 48_000, content_type: String::new(), inline: false, text: None }
}

pub fn thread(id: &str, subject: &str, from: Person, messages: Vec<Message>) -> Thread {
    let last = messages.last().map_or(NOV_1, |m| m.sent_at);
    Thread {
        summary: ThreadSummary {
            id: id.into(),
            account_id: "work".into(),
            split: moss_core::SplitId::Important,
            section: moss_core::Section::Reply,
            from,
            subject: subject.into(),
            summary: format!("About {subject}"),
            last_message_at: last,
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
        messages: messages.into_iter().map(|m| Message { thread_id: id.into(), ..m }).collect(),
    }
}

/// The budget thread from Dana (41) and Sam's approval (57).
pub fn budget() -> Thread {
    let mut first = message("m1", dana(), NOV_1 - 4 * DAY, "Hi Maya,\n\nThe final Q4 budget is 182k. Can you confirm by Friday?");
    first.attachments = vec![file("budget-q4.xlsx")];
    let second = message("m2", maya(), NOV_1 - 3 * DAY, "Looks right. Checking with Sam.");
    thread("41", "Q4 budget", dana(), vec![first, second])
}

pub fn approval() -> Thread {
    let sam = person("Sam Ortiz", "sam@northwind.co");
    thread("57", "Re: Q4 budget", sam.clone(), vec![message("m3", sam, NOV_1 - 2 * DAY, "Approved. Ship it.")])
}

/// A mailbox in memory. `failing` makes every lookup fail.
#[derive(Default)]
pub struct FakeMail {
    pub threads: Vec<Thread>,
    pub files: Vec<FileHit>,
    pub people: Vec<Contact>,
    pub saved: Mutex<Vec<Draft>>,
    pub failing: bool,
    /// Saving a draft fails.
    pub refuse_drafts: bool,
    /// Files can't be read: the server is offline.
    pub unreadable_files: bool,
    /// Search finds only mail with every word, as the engine's word search does.
    pub all_words: bool,
    /// The last file query asked.
    pub asked: Mutex<Option<FileQuery>>,
}

impl FakeMail {
    pub fn new() -> FakeMail {
        let files = vec![FileHit {
            thread_id: "41".into(),
            message_id: "m1".into(),
            subject: "Q4 budget".into(),
            from: dana(),
            sent_at: NOV_1 - 4 * DAY,
            file: file("budget-q4.xlsx"),
        }];
        FakeMail { threads: vec![budget(), approval()], files, people: vec![Contact { person: dana(), from_them: 6, to_them: 4, last_at: NOV_1 - 2 * DAY }], ..Default::default() }
    }

    fn check(&self) -> Result<(), String> {
        if self.failing {
            return Err("the database is locked".into());
        }
        Ok(())
    }
}

#[async_trait]
impl Mail for FakeMail {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
        self.check()?;
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        Ok(self
            .threads
            .iter()
            .filter(|t| {
                let text = format!("{} {} {} {}", t.summary.subject, t.summary.from.name, t.summary.from.email, t.messages[0].body_text).to_lowercase();
                if self.all_words {
                    words.iter().all(|w| text.contains(w))
                } else {
                    words.iter().any(|w| text.contains(w))
                }
            })
            .take(limit)
            .map(|t| SearchHit {
                thread: t.summary.clone(),
                message_id: t.messages[0].id.clone(),
                snippet: vec![SnippetPart { text: t.messages[0].body_text.clone(), matched: false }],
                rank: 1.0,
                file: None,
            })
            .collect())
    }

    fn thread(&self, id: &str) -> Result<Thread, String> {
        self.check()?;
        self.threads.iter().find(|t| t.summary.id == id).cloned().ok_or_else(|| format!("no thread {id}"))
    }

    fn files(&self, query: &FileQuery) -> Result<Vec<FileHit>, String> {
        self.check()?;
        *self.asked.lock().unwrap() = Some(query.clone());
        Ok(self.files.clone())
    }

    fn file(&self, message_id: &str, index: u32) -> Result<FileHit, String> {
        self.check()?;
        let hit = self.files.iter().find(|f| f.message_id == message_id && f.file.index == index);
        hit.cloned().ok_or_else(|| format!("no file {index} in {message_id}"))
    }

    /// The text its thread's attachment has.
    async fn file_text(&self, message_id: &str, index: u32) -> Result<Option<String>, String> {
        if self.unreadable_files {
            return Err("the server is offline".into());
        }
        let messages = self.threads.iter().flat_map(|t| &t.messages);
        let file = messages.filter(|m| m.id == message_id).flat_map(|m| &m.attachments).find(|a| a.index == index);
        Ok(file.and_then(|f| f.text.clone()))
    }

    fn people(&self, query: &str, limit: usize) -> Result<Vec<Contact>, String> {
        self.check()?;
        let q = query.to_lowercase();
        Ok(self.people.iter().filter(|c| c.person.name.to_lowercase().contains(&q)).take(limit).cloned().collect())
    }

    /// Maya, writing as she did to Dana before, with a signature that has her name.
    fn reply_context(&self, draft: &Draft) -> Option<Context> {
        let thread = draft.thread_id.as_deref().and_then(|id| self.thread(id).ok());
        let past = vec!["Hi Dana, sounds great. Cheers, M".to_string()];
        let context = match thread {
            Some(t) => Context::from_thread(&t, maya(), past, NOV_1),
            None => Context { subject: draft.subject.clone(), messages: vec![], me: maya(), past, now: NOV_1, signature: vec![], to: None },
        };
        Some(Context { signature: vec!["Maya Ross".into()], ..context })
    }

    fn save_draft(&self, draft: &Draft) -> Result<(), String> {
        self.check()?;
        if self.refuse_drafts {
            return Err("the disk is full".into());
        }
        self.saved.lock().unwrap().push(draft.clone());
        Ok(())
    }
}

/// One step of a scripted model: text to stream in pieces, tool calls, or an error.
pub enum Step {
    Say(Vec<&'static str>),
    Call(Vec<(&'static str, &'static str)>),
    Fail(AiError),
}

/// A model that answers from a script, and keeps every request it got.
#[derive(Default)]
pub struct ScriptedModel {
    pub steps: Mutex<VecDeque<Step>>,
    pub requests: Mutex<Vec<ChatRequest>>,
    /// Its context in tokens, when not the Gemma writer's.
    pub context: Option<usize>,
}

impl ScriptedModel {
    pub fn new(steps: Vec<Step>) -> ScriptedModel {
        ScriptedModel { steps: Mutex::new(steps.into()), requests: Mutex::default(), context: None }
    }

    /// The model with a context of `tokens`, as Apple Intelligence's 8k.
    pub fn with_context(mut self, tokens: usize) -> ScriptedModel {
        self.context = Some(tokens);
        self
    }

    pub fn requests(&self) -> Vec<ChatRequest> {
        self.requests.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChatModel for ScriptedModel {
    fn context_tokens(&self) -> usize {
        self.context.unwrap_or(crate::agent::CONTEXT_TOKENS)
    }

    async fn chat(&self, request: &ChatRequest, on_text: &mut OnText) -> Result<Reply, AiError> {
        self.requests.lock().unwrap().push(request.clone());
        let step = self.steps.lock().unwrap().pop_front().unwrap_or(Step::Say(vec!["(no more script)"]));
        match step {
            Step::Fail(e) => Err(e),
            Step::Call(calls) => Ok(Reply {
                text: String::new(),
                calls: calls
                    .into_iter()
                    .enumerate()
                    .map(|(n, (name, arguments))| ToolCall { id: format!("c{n}"), name: name.into(), arguments: arguments.into() })
                    .collect(),
                stopped: false,
            }),
            Step::Say(pieces) => {
                let mut text = String::new();
                for piece in pieces {
                    text.push_str(piece);
                    if !on_text(piece) {
                        return Ok(Reply { text, calls: vec![], stopped: true });
                    }
                }
                Ok(Reply { text, calls: vec![], stopped: false })
            }
        }
    }
}

/// The app's actions, faked: Done (done=false fails), list the inbox, and write_email, which
/// readies a reply to the budget thread (`send_at` as ms, when given).
#[derive(Default)]
pub struct FakeActions {
    /// Each call, with the arguments it got.
    pub acted: Mutex<Vec<(String, serde_json::Value)>>,
    /// What mark_done requires, as its definition says.
    pub required: Vec<String>,
}

impl FakeActions {
    /// With mark_done requiring `required`.
    pub fn schema(required: &[&str]) -> FakeActions {
        FakeActions { required: required.iter().map(|r| r.to_string()).collect(), ..Default::default() }
    }
}

#[async_trait]
impl Actions for FakeActions {
    fn specs(&self) -> Vec<serde_json::Value> {
        let required = self.required.clone();
        ["mark_done", "remind_me", "list_inbox", "write_email"]
            .map(|n| {
                let parameters = if n == "mark_done" || n == "remind_me" { serde_json::json!({"required": required}) } else { serde_json::json!({}) };
                serde_json::json!({"type": "function", "function": {"name": n, "parameters": parameters}})
            })
            .to_vec()
    }

    fn label(&self, name: &str, _: &serde_json::Value) -> Option<String> {
        (name == "mark_done").then(|| "Marking an email Done".into())
    }

    async fn act(&self, name: &str, args: &serde_json::Value) -> Option<Result<Acted, String>> {
        let id = args["thread_id"].as_str().unwrap_or_default().to_string();
        self.acted.lock().unwrap().push((name.to_string(), args.clone()));
        Some(match name {
            "mark_done" if args["done"] == false => Err("it's in the inbox already".into()),
            "mark_done" => Ok(Acted { text: "marked done".into(), did: Some(format!("Marked done: {id}")), ..Default::default() }),
            "remind_me" => Ok(Acted { text: "reminder changed".into(), ..Default::default() }),
            "update_draft" => {
                let mut draft = reply_draft(&budget(), args["body"].as_str().unwrap_or_default(), NOV_1);
                draft.id = args["draft_id"].as_str().unwrap_or_default().into();
                Ok(Acted { text: "draft changed".into(), draft: Some(draft), ..Default::default() })
            }
            "list_inbox" => Ok(Acted { text: "[41] Q4 budget, [57] Re: Q4 budget".into(), threads: vec!["41".into(), "57".into()], ..Default::default() }),
            "write_email" => {
                let mut draft = reply_draft(&budget(), args["body"].as_str().unwrap_or_default(), NOV_1);
                draft.id = "ready".into();
                let send_at = args["send_at"].as_i64();
                Ok(Acted { text: "waiting for the user to press Send".into(), draft: Some(draft), send_at, ..Default::default() })
            }
            _ => return None,
        })
    }
}
