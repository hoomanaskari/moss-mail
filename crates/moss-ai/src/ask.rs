//! Ask Moss: one chat, with the email on screen as its context when there is one. Moss works
//! out from the question whether it's about that email or any other, and looks up or acts on
//! the rest of the mail with its tools. Each chat remembers its turns until Moss quits.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use moss_core::Thread;
use serde_json::json;

use crate::agent::{self, Answer, AskEvent, Run};
use crate::chat::{ChatModel, Msg};
use crate::tools::{cited_thread_text, day, match_line, thread_files, zone, Actions, Mail, MailTools};
use crate::AiError;

/// The open email, in the prompt: a long conversation whole (about 16,000 tokens), leaving room
/// in the writer's 32k context for the tools and the chat. Only a longer one loses its oldest
/// messages.
pub const THREAD_CHARS: usize = 48_000;
/// Turns each chat keeps. Older ones would be dropped to fit the context anyway.
const TURNS: usize = 20;

/// What Moss can do, with the app's actions or without. `now`: when, for the times it writes.
fn can(acts: bool, now: i64) -> String {
    if acts {
        let zone = zone(now);
        let tomorrow = day(now + 86_400_000);
        format!("Times are the user's, UTC{zone}: write them like {tomorrow}T09:00:00{zone}. ") + "Use the tools to look things up in the user's mail and to do what they ask with it: mark mail Done or read, remind them, move it, delete it, block or accept senders, and write, reply to and forward emails. \
Emails that may be the ones the user means are listed beside the question: use their ids. To find other emails by a name or words in the subject, use search_mail. To go through the inbox, use list_inbox. Give list_inbox a split only when the user names one (newsletters, receipts, calendar, important or other). Never ask the user which email. \
When they ask for something on several emails, find them, then act on each one. \
To reply to someone, find their latest email with search_mail, then reply to that thread. \
To write a new email to someone, find their address with find_people, or put just their name in to. Never make up an address. \
Use write_email for every email: new ones, replies and forwards, with send_at when they want it to go later. \
Moss shows the email under your answer and the user presses Send, so never say it was sent. \
When asked to reply without being told what to say, write a short, sensible reply from the email yourself: the user changes it before sending. \
Never write an email out in your answer: put it in write_email, then say in a sentence what you wrote. \
Write emails as the user, in the first person. Turn what the user says into the email's own words: \"tell him I'll call\" becomes \"I'll call you\". End an email's body (not your answer) with a sign-off such as \"Best,\" and no name after it: the user's own signature goes below it. \
When a tool says it couldn't, fix what it says and try again, or tell the user it didn't work. \
Only delete mail or block a sender when the user asked for exactly that. Name emails by their subject or sender, never by id. "
    } else {
        "Use the tools to look things up in the user's mail. To write to someone, find their latest thread with search_mail and draft a reply to it. \
You can draft replies for the user to check, but you can't send, delete, archive or mark mail done; if asked to, say so and offer a draft. "
            .to_string()
    }
}

/// What Moss is told before your question. `open`: the email on screen, whole, to answer from
/// when the question is about it.
pub fn moss_system(now: i64, open: Option<&Thread>, acts: bool) -> String {
    moss_system_within(now, open, acts, THREAD_CHARS)
}

/// [`moss_system`] with at most `thread_chars` of the open email, for a model with a smaller
/// context.
pub fn moss_system_within(now: i64, open: Option<&Thread>, acts: bool, thread_chars: usize) -> String {
    let mut system = format!(
        "You are Moss, the assistant in the user's mail app. Today is {}. \
{}Never guess or make up emails, names, dates or amounts. \
Find threads and people yourself; never ask the user for ids, and never ask whether to look further: read what you need, then answer. \
Files people sent are marked [f:ID]; when the answer may be inside one (a letter, an invitation, an invoice), read it with read_attachment. \
Answer in a few short sentences. After each fact from an email, cite its thread as [t:ID], e.g. [t:41]. \
When a fact comes from inside a file, say which file by its name and cite it as [f:ID] too, e.g. [t:41, f:4406-2].",
        day(now),
        can(acts, now)
    );
    if let Some(thread) = open {
        system.push_str(&format!(
            "\n\nThe user has the email below open, thread {id}. When they ask about \"this\", \"it\" or \"them\", or to reply, without naming another email, they mean this one: answer from it and the text inside its attachments, and act on thread {id}. \
For anything else, use the tools. Cite this email's messages as [m:ID], e.g. [m:{m}], not as [t:ID].\n\n{text}",
            id = thread.summary.id,
            m = thread.messages.last().map_or("ID", |m| m.id.as_str()),
            text = cited_thread_text(thread, thread_chars)
        ));
        if let Some(at) = thread.summary.snoozed_until {
            let reminder = format!(" It has a reminder: it comes back on {}. To stop it, call remind_me without until.", day(at));
            let at = system.find("\n\nThread [t:").unwrap_or(system.len());
            system.insert_str(at, &reminder);
        }
    }
    system
}

/// Beside the question when an email is open: a small model heeds what's next to the question
/// more than what's far up in the prompt.
pub fn open_note(thread: &Thread) -> String {
    format!(
        "(I have the email “{}” from {} open. Here “this”, “it”, “she”, “he” and “they” mean that email and its sender, unless I name another.)",
        thread.summary.subject, thread.summary.from.name
    )
}

/// How the model picks the words to look up before it answers.
const FIND: &str = "You help an assistant in the user's mail app find the emails a request is about, before it answers. \
Give the words to search the user's mail for: a person's name, a subject, a topic. Use the chat so far for who \"him\", \"her\" or \"it\" is. \
Leave the query empty when the request isn't about particular emails.";

/// Every chat this session: what was asked and answered, and the one being answered now.
#[derive(Default)]
pub struct Chats {
    /// (question, answer as the model wrote it), oldest first.
    turns: Mutex<HashMap<String, Vec<(String, String)>>>,
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

/// Where a question goes, and who hears the answer.
pub struct Ask<'a> {
    pub model: &'a dyn ChatModel,
    pub mail: &'a dyn Mail,
    /// The app's actions on mail. Without them Moss reads and drafts replies only.
    pub actions: Option<&'a dyn Actions>,
    /// Milliseconds since the epoch: today's date, and when a draft was written.
    pub now: i64,
    pub emit: &'a (dyn Fn(AskEvent) + Send + Sync),
}

impl Chats {
    /// A question in chat `chat` (any id the UI picks). `open`: the email on screen, if any.
    pub async fn ask(&self, ask: Ask<'_>, chat: &str, open: Option<&Thread>, question: &str) -> Result<Answer, AiError> {
        let tools = MailTools::new(ask.mail, ask.now).asked(question).writing_with(ask.model).reading(open);
        let tools = match ask.actions {
            Some(actions) => tools.with_actions(actions),
            None => tools,
        };
        let files = open.map(thread_files).unwrap_or_default();
        let system = moss_system_within(ask.now, open, ask.actions.is_some(), crate::chat::room(THREAD_CHARS, ask.model));
        let mut messages = vec![Msg::System(system)];
        for (q, a) in self.turns.lock().unwrap().get(chat).into_iter().flatten() {
            messages.push(Msg::User(q.clone()));
            messages.push(Msg::Assistant { text: a.clone(), calls: vec![] });
        }
        // Looked up before the model starts, with a search the model writes from the chat so far:
        // a small model asks the user instead of looking.
        let named = match ask.actions {
            Some(_) => self.looked_up(&ask, &tools, chat, question).await,
            None => vec![],
        };
        let named: Vec<_> = named.into_iter().filter(|t| open.is_none_or(|o| o.summary.id != t.id)).collect();
        let mut notes: Vec<String> = open.map(open_note).into_iter().collect();
        if !named.is_empty() {
            let listed: Vec<String> = named.iter().map(match_line).collect();
            notes.push(format!("(Emails that may be the ones I mean: {}.)", listed.join("; ")));
        }
        let threads: Vec<String> = named.iter().map(|t| t.id.clone()).collect();
        messages.push(Msg::User(match notes.is_empty() {
            true => question.to_string(),
            false => format!("{}\n\n{question}", notes.join("\n")),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        self.running.lock().unwrap().insert(chat.to_string(), stop.clone());
        let run = Run { chat, model: ask.model, tools: &tools, files: &files, threads: &threads, review: ask.actions.is_some(), emit: ask.emit, stop: &stop };
        let result = agent::run(run, messages).await;
        let mut running = self.running.lock().unwrap();
        if running.get(chat).is_some_and(|s| Arc::ptr_eq(s, &stop)) {
            running.remove(chat);
        }
        drop(running);
        let (mut answer, raw) = result?;
        // The open email is on screen already: no card for it.
        answer.sources.retain(|t| open.is_none_or(|o| o.summary.id != t.id));
        // Only the open email's messages have chips to scroll to.
        answer.messages.retain(|id| open.is_some_and(|t| t.messages.iter().any(|m| &m.id == id)));
        if !raw.trim().is_empty() {
            let mut turns = self.turns.lock().unwrap();
            let turns = turns.entry(chat.to_string()).or_default();
            turns.push((question.to_string(), raw));
            if turns.len() > TURNS {
                turns.remove(0);
            }
        }
        Ok(answer)
    }

    /// The emails the question is about, as Moss's search finds them for the words the model
    /// picks from the question and the chat so far ("just say I'll text him" → "Alex Morgan").
    async fn looked_up(&self, ask: &Ask<'_>, tools: &MailTools<'_>, chat: &str, question: &str) -> Vec<moss_core::ThreadSummary> {
        let turns = self.turns.lock().unwrap().get(chat).cloned().unwrap_or_default();
        let earlier: Vec<String> = turns.iter().skip(turns.len().saturating_sub(2)).map(|(q, a)| format!("User: {q}\nAssistant: {a}")).collect();
        let earlier = if earlier.is_empty() { "(nothing yet)".to_string() } else { earlier.join("\n") };
        let parameters = json!({"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]});
        let tool = ("search", "Words to search the user's mail for: names, subjects or topics. Empty when the request isn't about particular emails.", parameters);
        let asked = format!("The chat so far:\n{earlier}\n\nThe user's new request: {question}");
        let query = crate::chat::decided(ask.model, FIND, asked, tool, 40).await;
        match query.as_ref().and_then(|q| q["query"].as_str()).map(str::trim).filter(|q| !q.is_empty()) {
            Some(query) => tools.likely(query).await,
            None => vec![],
        }
    }

    /// Stops the answer being written in `chat`. Returns whether one was.
    pub fn stop(&self, chat: &str) -> bool {
        match self.running.lock().unwrap().get(chat) {
            Some(stop) => !stop.swap(true, Ordering::Relaxed),
            None => false,
        }
    }

    /// Forgets `chat`, so it starts over.
    pub fn forget(&self, chat: &str) {
        self.turns.lock().unwrap().remove(chat);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_mail::Step;
    use crate::test_mail::*;

    fn thread_with_no_messages() -> Thread {
        crate::test_mail::thread("9", "Empty", dana(), vec![])
    }

    fn ask<'a>(model: &'a ScriptedModel, mail: &'a FakeMail, emit: &'a (dyn Fn(AskEvent) + Send + Sync)) -> Ask<'a> {
        Ask { model, mail, actions: None, now: NOV_1, emit }
    }

    fn system(request: &crate::chat::ChatRequest) -> String {
        let Msg::System(system) = &request.messages[0] else { panic!("a system message first") };
        system.clone()
    }

    fn tool_names(request: &crate::chat::ChatRequest) -> Vec<String> {
        request.tools.iter().map(|t| t["function"]["name"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn the_prompt_says_what_moss_may_do_and_what_day_it_is() {
        let moss = moss_system(NOV_1, None, false);
        assert!(moss.contains("Today is 2026-11-01."));
        assert!(moss.contains("can't send, delete, archive or mark mail done"));
        assert!(moss.contains("[t:ID]"));
        assert!(moss.contains("read it with read_attachment") && moss.contains("cite it as [f:ID]"));
        assert!(moss.contains("never ask whether to look further"));
        assert!(!moss.contains("open"), "no email open, nothing about one");
        let acting = moss_system(NOV_1, None, true);
        assert!(acting.contains("mark mail Done") && acting.contains("block or accept senders"));
        assert!(acting.contains("never say it was sent"), "the user presses Send");
        assert!(acting.contains("Use write_email for every email") && acting.contains("with send_at when they want it to go later"));
        assert!(acting.contains("act on each one"), "several emails at once");
        assert!(acting.contains("To write a new email to someone, find their address with find_people, or put just their name in to. Never make up an address."));
        assert!(!acting.contains("can't send"));
        assert!(acting.contains("Never ask the user which email") && acting.contains("list_inbox"));
        assert!(acting.contains("Emails that may be the ones the user means are listed beside the question: use their ids. To find other emails by a name or words in the subject, use search_mail."));
        assert!(acting.contains("Give list_inbox a split only when the user names one"));
        assert!(acting.contains("Emails that may be the ones the user means are listed beside the question: use their ids."));
        assert!(acting.contains("When asked to reply without being told what to say, write a short, sensible reply from the email yourself"));
        assert!(acting.contains("Never write an email out in your answer: put it in write_email"));
        assert!(acting.contains("Turn what the user says into the email's own words: \"tell him I'll call\" becomes \"I'll call you\"."));
        assert!(acting.contains("never by id"));
        assert!(acting.contains("When a tool says it couldn't, fix what it says and try again, or tell the user it didn't work."));
        assert!(acting.contains("End an email's body (not your answer) with a sign-off"));
        assert!(acting.contains("and no name after it: the user's own signature goes below it"));
        assert!(!acting.contains("Moss adds"), "a small model then signs emails Moss");
        let zone = crate::tools::zone(NOV_1);
        assert!(acting.contains(&format!("Times are the user's, UTC{zone}: write them like 2026-11-02T09:00:00{zone}.")), "{acting}");
    }

    #[test]
    fn the_open_email_is_named_beside_the_question() {
        let note = open_note(&budget());
        assert!(note.starts_with("(I have the email “Q4 budget” from Dana Lee open."));
        assert!(note.contains("“this”, “it”, “she”, “he” and “they” mean that email"));
    }

    #[test]
    fn with_an_email_open_the_prompt_has_it_whole_and_says_when_a_question_is_about_it() {
        let open = moss_system(NOV_1, Some(&budget()), true);
        assert!(open.contains("The user has the email below open, thread 41."));
        assert!(open.contains("\"this\", \"it\" or \"them\""));
        assert!(open.contains("For anything else, use the tools."));
        assert!(open.contains("[m:ID], e.g. [m:m2]"));
        assert!(open.ends_with(&cited_thread_text(&budget(), THREAD_CHARS)));
        assert!(moss_system(NOV_1, Some(&thread_with_no_messages()), true).contains("e.g. [m:ID]"));
        assert!(!open.contains("It has a reminder"));
        let mut later = budget();
        later.summary.snoozed_until = Some(NOV_1 + 86_400_000);
        assert!(moss_system(NOV_1, Some(&later), true).contains("It has a reminder: it comes back on 2026-11-02. To stop it, call remind_me without until."));
    }

    #[tokio::test]
    async fn it_looks_up_the_emails_a_question_is_about_with_words_the_model_picks() {
        let done = ("review", r#"{"done": true, "next": ""}"#);
        let model = ScriptedModel::new(vec![
            Step::Call(vec![("search", r#"{"query": "Dana Lee"}"#)]),
            Step::Say(vec!["She sent it [t:41]."]),
            Step::Call(vec![done]),
            // A follow-up about no particular email: nothing is looked up.
            Step::Call(vec![("search", r#"{"query": ""}"#)]),
            Step::Say(vec!["You're welcome."]),
            Step::Call(vec![done]),
            // With an email open, it's never one of the emails beside the question.
            Step::Call(vec![("search", r#"{"query": "Q4 budget"}"#)]),
            Step::Say(vec!["Sam approved it."]),
            Step::Call(vec![done]),
        ]);
        let mail = FakeMail::new();
        let actions = FakeActions::default();
        let chats = Chats::default();
        let acting = || Ask { actions: Some(&actions), ..ask(&model, &mail, &|_| {}) };
        let answer = chats.ask(acting(), "c", None, "Did she send the budget?").await.unwrap();
        let ids: Vec<&str> = answer.sources.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["41"], "a card, though no tool ran");
        chats.ask(acting(), "c", None, "Thanks").await.unwrap();
        chats.ask(acting(), "o", Some(&budget()), "Did Sam agree?").await.unwrap();
        let requests = model.requests();
        let last = |n: usize| requests[n].messages.last().unwrap().to_json()["content"].as_str().unwrap().to_string();
        assert!(requests[0].must_call && requests[0].tools[0]["function"]["name"] == "search");
        assert_eq!(last(0), "The chat so far:\n(nothing yet)\n\nThe user's new request: Did she send the budget?");
        assert_eq!(last(1), "(Emails that may be the ones I mean: [t:41] “Q4 budget” from Dana Lee.)\n\nDid she send the budget?");
        assert!(last(3).starts_with("The chat so far:\nUser: Did she send the budget?\nAssistant: She sent it [t:41].\n\n"), "{}", last(3));
        assert_eq!(last(4), "Thanks", "nothing to look up, nothing beside it");
        assert_eq!(last(7), format!("{}\n(Emails that may be the ones I mean: [t:57] “Re: Q4 budget” from Sam Ortiz.)\n\nDid Sam agree?", open_note(&budget())));
    }

    #[tokio::test]
    async fn a_model_with_a_smaller_context_reads_less_of_the_open_email() {
        let model = ScriptedModel::new(vec![Step::Say(vec!["Long."])]).with_context(8_192);
        let mail = FakeMail::new();
        let mut open = budget();
        for m in &mut open.messages {
            m.paragraphs = vec!["word ".repeat(2_000)];
        }
        Chats::default().ask(ask(&model, &mail, &|_| {}), "c", Some(&open), "How long is it?").await.unwrap();
        let room = crate::chat::room(THREAD_CHARS, &model);
        assert_eq!(system(&model.requests()[0]), moss_system_within(NOV_1, Some(&open), false, room));
        assert!(system(&model.requests()[0]).contains("(1 earlier message left out)"));
        assert_ne!(moss_system_within(NOV_1, Some(&open), false, room), moss_system(NOV_1, Some(&open), false));
    }

    #[tokio::test]
    async fn the_open_email_is_not_shown_as_a_card() {
        let model = ScriptedModel::new(vec![Step::Call(vec![("read_thread", r#"{"thread_id":"41"}"#), ("read_thread", r#"{"thread_id":"57"}"#)]), Step::Say(vec!["By Friday [t:41], approved [t:57]."])]);
        let mail = FakeMail::new();
        let answer = Chats::default().ask(ask(&model, &mail, &|_| {}), "c", Some(&budget()), "When?").await.unwrap();
        let ids: Vec<&str> = answer.sources.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["57"], "it's on screen already");
    }

    #[tokio::test]
    async fn one_chat_follows_you_from_email_to_email_and_to_all_mail() {
        let model = ScriptedModel::new(vec![Step::Say(vec!["It's 182k."]), Step::Say(vec!["Sam approved it."]), Step::Say(vec!["Two."])]);
        let mail = FakeMail::new();
        let chats = Chats::default();
        let first = chats.ask(ask(&model, &mail, &|_| {}), "c", Some(&budget()), "How much?").await.unwrap();
        let question = &model.requests()[0].messages[1];
        assert_eq!(question, &Msg::User(format!("{}\n\nHow much?", open_note(&budget()))), "which email is open, beside the question");
        assert_eq!(first.text, "It's 182k.");
        chats.ask(ask(&model, &mail, &|_| {}), "c", Some(&approval()), "And this one?").await.unwrap();
        chats.ask(ask(&model, &mail, &|_| {}), "c", None, "How many emails about it?").await.unwrap();

        let requests = model.requests();
        assert_eq!(system(&requests[0]), moss_system(NOV_1, Some(&budget()), false));
        assert_eq!(system(&requests[1]), moss_system(NOV_1, Some(&approval()), false), "the email open now");
        assert_eq!(system(&requests[2]), moss_system(NOV_1, None, false));
        assert_eq!(tool_names(&requests[0]), crate::tools::ALL, "every tool, with an email open too");
        assert_eq!(
            requests[2].messages[1..],
            [
                Msg::User("How much?".into()),
                Msg::Assistant { text: "It's 182k.".into(), calls: vec![] },
                Msg::User("And this one?".into()),
                Msg::Assistant { text: "Sam approved it.".into(), calls: vec![] },
                Msg::User("How many emails about it?".into())
            ],
            "it remembers what was said about each email"
        );
    }

    #[tokio::test]
    async fn with_the_apps_actions_it_can_do_what_the_window_does() {
        let model = ScriptedModel::new(vec![
            Step::Call(vec![("search", r#"{"query": "budget"}"#)]),
            Step::Call(vec![("mark_done", r#"{"thread_id":"41"}"#), ("mark_done", r#"{"thread_id":"57"}"#)]),
            Step::Say(vec!["Both are Done."]),
            Step::Call(vec![("review", r#"{"done": true, "next": ""}"#)]),
        ]);
        let mail = FakeMail::new();
        let actions = FakeActions::default();
        let a = Ask { actions: Some(&actions), ..ask(&model, &mail, &|_| {}) };
        let answer = Chats::default().ask(a, "c", None, "Mark both budget emails done").await.unwrap();
        assert_eq!(answer.did, ["Marked done: 41", "Marked done: 57"]);
        let request = &model.requests()[1];
        assert!(system(request).contains("mark mail Done"));
        let names = tool_names(request);
        assert!(names.contains(&"mark_done".to_string()) && names.contains(&"write_email".to_string()));
        assert!(!names.contains(&"draft_reply".to_string()), "write_email writes every email");
    }

    #[tokio::test]
    async fn a_new_email_with_an_email_open_becomes_a_reply_when_the_model_says_one_was_asked_for() {
        let done = ("review", r#"{"done": true, "next": ""}"#);
        // To the user themself: a small model mixes up who "their response" is from.
        let new_to_dana = ("write_email", r#"{"to": ["Maya Ross"], "subject": "Hello", "body": "I'll text you soon."}"#);
        let model = ScriptedModel::new(vec![
            Step::Call(vec![("search", r#"{"query": ""}"#)]),
            Step::Call(vec![new_to_dana]),
            // A small model wrote a new email where a reply was asked for: Moss asks which.
            Step::Call(vec![("kind", r#"{"reply": true}"#)]),
            Step::Say(vec!["Hi Dana,\n\nI'll text you soon.\n\nBest,"]),
            Step::Say(vec!["Drafted."]),
            Step::Call(vec![done]),
        ]);
        let mail = FakeMail::new();
        let actions = FakeActions::default();
        let a = Ask { actions: Some(&actions), ..ask(&model, &mail, &|_| {}) };
        let answer = Chats::default().ask(a, "c", Some(&budget()), "Draft a response telling her I'll text her soon").await.unwrap();
        let acted = actions.acted.lock().unwrap().clone();
        assert_eq!(acted[0].0, "write_email");
        assert_eq!(acted[0].1["reply_to_thread"], "41", "a reply in the open conversation");
        assert!(acted[0].1["subject"].is_null(), "with its Re: subject");
        assert!(acted[0].1["to"].is_null(), "to the person who wrote it");
        assert_eq!(answer.drafts.len(), 1);
        let kind = &model.requests()[2];
        assert!(kind.must_call && tool_names(kind) == ["kind"]);
        let Msg::User(asked) = kind.messages.last().unwrap() else { panic!("a question") };
        assert!(asked.contains("“Q4 budget” from Dana Lee") && asked.contains("Draft a response telling her I'll text her soon"), "{asked}");
    }

    #[tokio::test]
    async fn a_new_email_stays_new_when_the_model_says_so_and_nothing_is_asked_with_no_email_open() {
        let done = ("review", r#"{"done": true, "next": ""}"#);
        let model = ScriptedModel::new(vec![
            Step::Call(vec![("search", r#"{"query": ""}"#)]),
            // To her by name, and the model says a new email was asked for.
            Step::Call(vec![("write_email", r#"{"to": ["dana lee"], "subject": "Lunch", "body": "Lunch Friday?"}"#)]),
            Step::Call(vec![("kind", r#"{"reply": false}"#)]),
            Step::Say(vec!["Hi Dana, lunch Friday?"]),
            Step::Say(vec!["Drafted."]),
            Step::Call(vec![done]),
            // With no email open: nothing to ask.
            Step::Call(vec![("search", r#"{"query": ""}"#)]),
            Step::Call(vec![("write_email", r#"{"to": ["sam@acme.co"], "subject": "Hi", "body": "Hi Sam"}"#)]),
            Step::Say(vec!["Hi Sam,"]),
            Step::Say(vec!["Drafted."]),
            Step::Call(vec![done]),
            // A reply already, with her email open: nothing to ask either.
            Step::Call(vec![("search", r#"{"query": ""}"#)]),
            Step::Call(vec![("write_email", r#"{"reply_to_thread": "41", "body": "Yes"}"#)]),
            Step::Say(vec!["Hi Dana, yes."]),
            Step::Say(vec!["Drafted."]),
            Step::Call(vec![done]),
        ]);
        let mail = FakeMail::new();
        let actions = FakeActions::default();
        let chats = Chats::default();
        let a = || Ask { actions: Some(&actions), ..ask(&model, &mail, &|_| {}) };
        chats.ask(a(), "c", Some(&budget()), "Write Dana a new email asking about lunch").await.unwrap();
        chats.ask(a(), "d", None, "Email Sam hi").await.unwrap();
        chats.ask(a(), "e", Some(&budget()), "Reply yes").await.unwrap();
        let writes: Vec<_> = actions.acted.lock().unwrap().iter().filter(|(n, _)| n == "write_email").map(|(_, a)| a.clone()).collect();
        assert!(writes[..2].iter().all(|w| w.get("reply_to_thread").is_none()), "{writes:?}");
        assert_eq!(model.requests().iter().filter(|r| tool_names(r) == ["kind"]).count(), 1, "asked once, with Dana's email open");
    }

    #[tokio::test]
    async fn reply_classification_sees_recipients_and_keeps_explicit_sender_and_copies() {
        for verdict in [Step::Call(vec![("kind", r#"{"reply": false}"#)]), Step::Call(vec![("kind", r#"{"reply": true}"#)]), Step::Fail(AiError::NotReady)] {
            let converts = matches!(&verdict, Step::Call(calls) if calls[0].1.contains("true"));
            let model = ScriptedModel::new(vec![
                Step::Call(vec![("search", r#"{"query": ""}"#)]),
                Step::Call(vec![("write_email", r#"{"to": ["sam@acme.co"], "cc": ["copy@example.test"], "bcc": ["private@example.test"], "account": "personal", "from": "alias@example.test", "subject": "Hello", "body": "Hi"}"#)]),
                verdict,
                Step::Say(vec!["Hi,"]),
                Step::Say(vec!["Drafted."]),
                Step::Call(vec![("review", r#"{"done": true, "next": ""}"#)]),
            ]);
            let mail = FakeMail::new();
            let actions = FakeActions::default();
            let a = Ask { actions: Some(&actions), ..ask(&model, &mail, &|_| {}) };
            Chats::default().ask(a, "c", Some(&budget()), "Email Sam about this, using my personal account and these copies").await.unwrap();
            let acted = actions.acted.lock().unwrap();
            let args = &acted[0].1;
            assert_eq!(args["account"], "personal");
            assert_eq!(args["from"], "alias@example.test");
            assert_eq!(args["cc"], serde_json::json!(["copy@example.test"]));
            assert_eq!(args["bcc"], serde_json::json!(["private@example.test"]));
            if converts {
                assert_eq!(args["reply_to_thread"], "41");
                assert!(args["to"].is_null() && args["subject"].is_null());
            } else {
                assert!(args["reply_to_thread"].is_null());
                assert_eq!(args["to"], serde_json::json!(["sam@acme.co"]));
                assert_eq!(args["subject"], "Hello");
            }
            let requests = model.requests();
            let kind = requests.iter().find(|r| tool_names(r) == ["kind"]).unwrap();
            let Msg::User(question) = kind.messages.last().unwrap() else { panic!("classification question") };
            for address in [&budget().summary.from.email, "sam@acme.co", "copy@example.test", "private@example.test"] {
                assert!(question.contains(address), "missing {address} in {question}");
            }
        }
    }

    #[tokio::test]
    async fn the_open_emails_files_can_be_cited_with_their_text_in_the_prompt() {
        let model = ScriptedModel::new(vec![Step::Say(vec!["182k [m:m1, f:m1-0], see [f:m3-0]."])]);
        let mut thread = budget();
        thread.messages[0].attachments[0].text = Some("Total: 182k".into());
        let answer = Chats::default().ask(ask(&model, &FakeMail::new(), &|_| {}), "c", Some(&thread), "How much?").await.unwrap();
        let names: Vec<&str> = answer.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["budget-q4.xlsx"], "a file of another thread isn't");
        assert!(system(&model.requests()[0]).contains("Inside the attachment budget-q4.xlsx [f:m1-0]:\nTotal: 182k"));
    }

    #[tokio::test]
    async fn only_messages_of_the_open_email_can_be_cited() {
        let model = ScriptedModel::new(vec![Step::Say(vec!["Yes [m:m1][m:m3][m:zz]."]), Step::Say(vec!["Yes [m:m1]."])]);
        let mail = FakeMail::new();
        let chats = Chats::default();
        let answer = chats.ask(ask(&model, &mail, &|_| {}), "c", Some(&budget()), "Agreed?").await.unwrap();
        assert_eq!(answer.messages, ["m1"], "m3 is in another thread, zz nowhere");
        let answer = chats.ask(ask(&model, &mail, &|_| {}), "c", None, "Agreed?").await.unwrap();
        assert!(answer.messages.is_empty(), "with no email open there are no messages to point at");
    }

    #[test]
    fn the_open_email_fits_the_context_with_room_for_the_tools_and_the_chat() {
        assert!(THREAD_CHARS >= 45_000, "a long conversation, whole");
        let tools = 6_000; // every tool's definition, the app's actions too
        assert!(crate::agent::tokens(THREAD_CHARS) + crate::agent::ANSWER_TOKENS as usize + tools + 8_000 < crate::agent::CONTEXT_TOKENS);
    }

    #[tokio::test]
    async fn a_long_conversation_is_read_whole() {
        let text = "Each message says something new. ".repeat(150); // about 5,000 characters each
        let messages = (0..9).map(|n| message(&format!("m{n}"), dana(), NOV_1 + n, &format!("{n}: {text}"))).collect();
        let long = thread("41", "Q4 budget", dana(), messages);
        let model = ScriptedModel::new(vec![Step::Say(vec!["ok"])]);
        let mail = FakeMail::new();
        Chats::default().ask(ask(&model, &mail, &|_| {}), "c", Some(&long), "What changed?").await.unwrap();
        let system = system(&model.requests()[0]);
        assert!(!system.contains("left out"), "no message left out");
        assert!((0..9).all(|n| system.contains(&format!("{n}: Each message says"))));
    }

    #[tokio::test]
    async fn each_chat_remembers_only_its_own_turns() {
        let model = ScriptedModel::new(vec![Step::Say(vec!["A"]), Step::Say(vec!["B"]), Step::Say(vec!["C"])]);
        let mail = FakeMail::new();
        let chats = Chats::default();
        chats.ask(ask(&model, &mail, &|_| {}), "m1", None, "one").await.unwrap();
        chats.ask(ask(&model, &mail, &|_| {}), "m2", None, "two").await.unwrap();
        chats.ask(ask(&model, &mail, &|_| {}), "m1", None, "three").await.unwrap();
        let requests = model.requests();
        assert_eq!(requests[1].messages.len(), 2, "m2 doesn't see m1");
        assert_eq!(requests[2].messages.len(), 4);
    }

    #[tokio::test]
    async fn forgetting_a_chat_starts_it_over() {
        let model = ScriptedModel::new(vec![Step::Say(vec!["A"]), Step::Say(vec!["B"])]);
        let mail = FakeMail::new();
        let chats = Chats::default();
        chats.ask(ask(&model, &mail, &|_| {}), "m", None, "one").await.unwrap();
        chats.forget("m");
        chats.ask(ask(&model, &mail, &|_| {}), "m", None, "two").await.unwrap();
        assert_eq!(model.requests()[1].messages.len(), 2);
    }

    #[tokio::test]
    async fn a_failed_or_empty_answer_is_not_remembered() {
        let model = ScriptedModel::new(vec![
            Step::Fail(AiError::NotReady),
            Step::Say(vec![""]),
            Step::Say(vec![""]),
            Step::Say(vec!["ok"]),
        ]);
        let mail = FakeMail::new();
        let chats = Chats::default();
        assert_eq!(chats.ask(ask(&model, &mail, &|_| {}), "m", None, "one").await, Err(AiError::NotReady));
        assert_eq!(chats.ask(ask(&model, &mail, &|_| {}), "m", None, "two").await.unwrap().text, "");
        chats.ask(ask(&model, &mail, &|_| {}), "m", None, "three").await.unwrap();
        assert_eq!(model.requests()[3].messages.len(), 2, "nothing earlier was kept");
    }

    #[tokio::test]
    async fn only_the_latest_turns_are_kept() {
        let model = ScriptedModel::new((0..TURNS + 1).map(|_| Step::Say(vec!["a"])).collect());
        let mail = FakeMail::new();
        let chats = Chats::default();
        for n in 0..=TURNS {
            chats.ask(ask(&model, &mail, &|_| {}), "m", None, &format!("q{n}")).await.unwrap();
        }
        let last = model.requests().pop().unwrap().messages;
        assert_eq!(last.len(), 2 + 2 * TURNS);
        assert_eq!(last[1], Msg::User("q0".into()));
        let model = ScriptedModel::new(vec![Step::Say(vec!["a"])]);
        chats.ask(ask(&model, &mail, &|_| {}), "m", None, "next").await.unwrap();
        assert_eq!(model.requests()[0].messages[1], Msg::User("q1".into()), "the oldest went");
    }

    #[tokio::test]
    async fn stopping_a_chat_stops_its_answer_and_keeps_what_it_wrote() {
        let model = ScriptedModel::new(vec![Step::Say(vec!["Dana ", "said ", "more"]), Step::Say(vec!["next"])]);
        let mail = FakeMail::new();
        let chats = Chats::default();
        assert!(!chats.stop("m"), "nothing to stop");
        let stopped = std::sync::Mutex::new(vec![]);
        let emit = |e: AskEvent| {
            if matches!(e, AskEvent::Text { .. }) {
                stopped.lock().unwrap().push(chats.stop("m"));
            }
        };
        let answer = chats.ask(ask(&model, &mail, &emit), "m", None, "one").await.unwrap();
        assert!(answer.stopped);
        assert_eq!(answer.text, "Dana");
        assert_eq!(stopped.lock().unwrap()[0], true);
        assert!(!chats.stop("m"), "it's finished now");
        chats.ask(ask(&model, &mail, &|_| {}), "m", None, "two").await.unwrap();
        let kept = &model.requests()[1].messages;
        assert_eq!(kept[2], Msg::Assistant { text: "Dana ".into(), calls: vec![] }, "the part it wrote is remembered");
    }
}
