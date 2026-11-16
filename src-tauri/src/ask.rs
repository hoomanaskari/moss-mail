//! Ask Moss, as commands. The work is in `moss_ai::ask` (the chat), `moss_ai::agent` (the loop)
//! and `moss_ai::tools` (reading mail); this is the mail it reads, and the app's actions on it:
//! the same tools agents get over MCP (`moss_mcp::MossTools`), so the chat can do what the
//! window does with mail. Answers stream to the window as `moss://ask` events.
//!
//! It reads the text inside a thread's files where Moss can (see `moss_engine::reading`). It
//! never sends: an email it writes waits under the answer for you to press Send.

use std::path::Path;
use std::sync::Arc;

use moss_ai::agent::{Answer, AskEvent};
use moss_ai::ask::{Ask, Chats};
use moss_ai::tools::{Acted, Actions, Mail};
use moss_ai::Ai;
use moss_core::{Contact, Draft, FileHit, FileQuery, SearchHit, Thread};
use moss_engine::{Engine, Events};
use moss_mcp::MossTools;
use serde_json::Value;
use tauri::{Emitter, Runtime, State};

/// The window event answers stream as.
pub const ASK_EVENT: &str = "moss://ask";

/// An engine error, as the model reads it.
fn text(e: moss_engine::Error) -> String {
    e.to_string()
}

/// The mail the agent reads: search, threads, files and people, and saving drafts.
pub struct EngineMail(pub Engine);

#[async_trait::async_trait]
impl Mail for EngineMail {
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, String> {
        self.0.find(query, None, limit).await.map_err(text)
    }

    fn thread(&self, id: &str) -> Result<Thread, String> {
        self.0.thread(id).map_err(text)
    }

    async fn read_thread(&self, id: &str) -> Result<Thread, String> {
        self.0.thread_with_files(id).await.map_err(text)
    }

    fn files(&self, query: &FileQuery) -> Result<Vec<FileHit>, String> {
        self.0.find_files(query).map_err(text)
    }

    fn file(&self, message_id: &str, index: u32) -> Result<FileHit, String> {
        self.0.file(message_id, index).map_err(text)
    }

    async fn file_text(&self, message_id: &str, index: u32) -> Result<Option<String>, String> {
        self.0.file_text(message_id, index).await.map_err(text)
    }

    fn people(&self, query: &str, limit: usize) -> Result<Vec<Contact>, String> {
        self.0.find_people(query, limit).map_err(text)
    }

    fn save_draft(&self, draft: &Draft) -> Result<(), String> {
        self.0.save_draft(draft).map(|_| ()).map_err(text)
    }

    /// As Quick reply reads it (`replies::context`), with the signature the draft goes out with.
    fn reply_context(&self, draft: &Draft) -> Option<moss_ai::reply::Context> {
        let signatures = self.0.signatures().ok()?.signatures;
        let signature = draft.signature_id.as_ref().and_then(|id| signatures.into_iter().find(|s| &s.id == id)).map(|s| s.lines).unwrap_or_default();
        crate::replies::context(&self.0, draft.thread_id.as_deref(), draft.to.first(), &draft.from_account_id, signature).ok()
    }
}

/// What the chat does with mail beyond reading it: Done, remind, move, delete, block, drafts,
/// as agents do over MCP.
pub struct EngineActions(pub MossTools);

/// The actions over `engine`. Files saved from mail go beside the database in `data_dir`.
pub fn actions(engine: &Engine, events: Arc<dyn Events>, data_dir: &Path) -> EngineActions {
    EngineActions(MossTools::new(engine.clone(), events, crate::now_ms, data_dir.join("attachments")))
}

#[async_trait::async_trait]
impl Actions for EngineActions {
    fn specs(&self) -> Vec<Value> {
        self.0.chat_specs()
    }

    fn label(&self, name: &str, args: &Value) -> Option<String> {
        MossTools::chat_label(name, args)
    }

    async fn act(&self, name: &str, args: &Value) -> Option<Result<Acted, String>> {
        let ran = self.0.chat_run(name, args.clone()).await?;
        Some(ran.map(|r| Acted { text: r.text, threads: r.threads, draft: r.draft, send_at: r.send_at, did: r.did }))
    }
}

/// Sends each step of an answer to the window.
fn to_window<R: Runtime>(app: tauri::AppHandle<R>) -> impl Fn(AskEvent) + Send + Sync {
    move |event| {
        // Nothing to do if the window is gone; the command still returns the answer.
        let _ = app.emit(ASK_EVENT, event);
    }
}

/// A question in chat `chat` (any id the window picks). `open`: the email on screen, if any.
/// Errors keep their engine or AI shape (`notReady`: no writing model yet).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ask_moss<R: Runtime>(
    app: tauri::AppHandle<R>,
    engine: State<'_, Engine>,
    ai: State<'_, Ai>,
    chats: State<'_, Chats>,
    actions: State<'_, EngineActions>,
    chat: String,
    question: String,
    open: Option<String>,
) -> Result<Answer, Value> {
    let open: Option<Thread> = match open {
        Some(id) => Some(engine.thread_with_files(&id).await.map_err(crate::json)?),
        None => None,
    };
    let mail = EngineMail(engine.inner().clone());
    let emit = to_window(app);
    let ask = Ask { model: ai.inner(), mail: &mail, actions: Some(actions.inner()), now: crate::now_ms(), emit: &emit };
    chats.ask(ask, &chat, open.as_ref(), &question).await.map_err(crate::json)
}

/// Stops the answer being written in `chat`; it returns with what it wrote so far.
#[tauri::command]
pub fn ask_stop(chats: State<'_, Chats>, chat: String) -> bool {
    chats.stop(&chat)
}

/// Emails on this Mac, for "All 4 accounts · 2,431 emails on this Mac".
#[tauri::command]
pub fn ask_mail_count(engine: State<'_, Engine>) -> crate::Result<u64> {
    engine.mail_count()
}

/// Forgets a chat, so it starts over.
#[tauri::command]
pub fn ask_forget(chats: State<'_, Chats>, chat: String) {
    chats.forget(&chat);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use moss_engine::{Event, Events};
    use std::sync::Arc;

    /// Events no one listens to, for tests that only need an engine.
    pub(crate) struct Quiet;
    impl Events for Quiet {
        fn emit(&self, _: Event) {}
    }

    #[tokio::test]
    async fn the_agent_reads_the_engines_mail_and_saves_drafts_there() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let mut store = moss_store::Store::open(data.join("moss.db")).unwrap();
        store.upsert_account(&crate::tests::work_account()).unwrap();
        let raw = b"From: Dana Lee <dana@acme.co>\r\nTo: maya@northwind.co\r\nSubject: Deck\r\n\
            Message-ID: <d@acme.co>\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\n\
            Content-Type: text/plain\r\n\r\nCan you send the deck?\r\n--m\r\n\
            Content-Type: application/pdf\r\nContent-Disposition: attachment; filename=deck.pdf\r\n\r\n%PDF\r\n--m--\r\n";
        let source = moss_core::MessageSource { account_id: "work".into(), mailbox: "INBOX".into(), provider_id: "1:1".into(), ..Default::default() };
        let thread_id = store.ingest(raw, &source).unwrap().thread_id;
        drop(store);
        let secrets = Arc::new(moss_accounts::MemorySecretStore::new());
        let engine = crate::engine(&data, secrets, Arc::new(Quiet), moss_providers::TlsRoots::Platform).unwrap();
        Quiet.emit(Event::MailChanged { account_id: "work".into() });
        let mail = EngineMail(engine.clone());

        let hits = mail.search("deck", 5).await.unwrap();
        assert_eq!(hits[0].thread.id, thread_id);
        let thread = mail.thread(&thread_id).unwrap();
        assert_eq!(thread.summary.subject, "Deck");
        assert!(mail.thread("999").unwrap_err().contains("999"));
        let files = mail.files(&FileQuery { limit: 5, ..Default::default() }).unwrap();
        assert_eq!(files[0].file.name, "deck.pdf");
        assert_eq!(mail.file(&files[0].message_id, 0).unwrap(), files[0]);
        assert!(mail.file(&files[0].message_id, 9).is_err());
        // With no reader set, a file's text isn't there, and a thread reads as it is.
        assert_eq!(mail.file_text(&files[0].message_id, 0).await.unwrap(), None);
        assert!(mail.file_text(&files[0].message_id, 9).await.is_err());
        assert_eq!(mail.read_thread(&thread_id).await.unwrap(), thread);
        assert!(mail.read_thread("999").await.is_err());
        assert_eq!(mail.people("dana", 5).unwrap()[0].person.email, "dana@acme.co");

        let draft = moss_ai::tools::reply_draft(&thread, "Here it is.", 1);
        mail.save_draft(&draft).unwrap();
        let persisted = engine.drafts().unwrap().remove(0);
        assert_eq!(persisted.revision.as_ref().map(String::len), Some(32));
        assert_eq!(Draft { revision: None, ..persisted }, draft);
        // What Quick reply reads for the draft: the account's sender, the conversation, the signature.
        let context = mail.reply_context(&draft).unwrap();
        assert_eq!(context.subject, "Deck");
        assert_eq!(context.me.email, "maya@northwind.co");
        assert!(context.messages.iter().any(|(_, _, text)| text.contains("Can you send the deck?")));
        assert!(context.signature.is_empty(), "no signature on it");
        let lines = vec!["Best,".to_string(), "Maya".to_string()];
        engine.save_signature(&moss_core::Signature { id: "work".into(), name: "Work".into(), lines: lines.clone() }).unwrap();
        let signed = Draft { signature_id: Some("work".into()), ..draft.clone() };
        assert_eq!(mail.reply_context(&signed).unwrap().signature, lines, "so the email ends to fit it");
        // A new email: no conversation, so the writer is told who it's for.
        let new = Draft { thread_id: None, ..draft.clone() };
        assert_eq!(mail.reply_context(&new).unwrap().to.as_ref(), draft.to.first());
        assert!(draft.to.first().is_some());
        let stranger = Draft { from_account_id: "nope".into(), ..draft };
        assert!(mail.save_draft(&stranger).is_err());
        assert!(mail.reply_context(&stranger).is_none(), "no such account");

        // What the window does with mail, the chat does too.
        let actions = actions(&engine, Arc::new(Quiet), &data);
        let names: Vec<String> = actions.specs().iter().map(|s| s["function"]["name"].as_str().unwrap().to_string()).collect();
        assert!(names.contains(&"mark_done".to_string()) && names.contains(&"write_email".to_string()));
        assert_eq!(actions.label("mark_done", &serde_json::json!({})).as_deref(), Some("Marking an email Done"));
        let done = actions.act("mark_done", &serde_json::json!({"thread_id": thread_id})).await.unwrap().unwrap();
        assert_eq!(done.did.as_deref(), Some("Marked done: Deck"));
        assert!(engine.done_threads(None, 5).unwrap().iter().any(|t| t.id == thread_id));
        let ready = actions.act("write_email", &serde_json::json!({"reply_to_thread": thread_id, "body": "Here."})).await.unwrap().unwrap();
        assert!(ready.draft.is_some(), "it waits for Send");
        assert!(actions.act("get_thread", &serde_json::json!({})).await.is_none(), "the chat reads threads its own way");
    }
}
