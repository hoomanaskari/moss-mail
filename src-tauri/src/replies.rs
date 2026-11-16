//! AI replies for the composer: quick replies to pick from, a reply from your own words, and
//! rewrites of what's in the composer. They only ever write text: they never send, and never
//! mark mail Done.

use moss_ai::reply::{Context, Job, ReplyOption};
use moss_ai::Ai;
use moss_core::Person;
use moss_engine::{Engine, Error};
use serde_json::Value;
use tauri::{AppHandle, Runtime, State};

use crate::json;
use crate::writing::Writing;

/// What the writing model reads: you as `account_id`, and the conversation `thread_id` with
/// what you wrote its other person before. With no thread (a new email) it reads who it's `to`,
/// when known, and what you wrote them before. `signature` is the one added below the reply,
/// so it ends to fit.
pub fn context(engine: &Engine, thread_id: Option<&str>, to: Option<&Person>, account_id: &str, signature: Vec<String>) -> Result<Context, Value> {
    let accounts = engine.accounts().map_err(json)?;
    let account = accounts
        .into_iter()
        .find(|a| a.account.id == account_id)
        .ok_or_else(|| json(Error::UnknownAccount(account_id.into())))?
        .account;
    let me = Person { name: account.sender_name, email: account.address };
    let Some(id) = thread_id else {
        let past = match to {
            Some(to) => engine.my_words_to(account_id, &to.email, "").map_err(json)?,
            None => vec![],
        };
        return Ok(Context { subject: String::new(), messages: vec![], me, past, now: crate::now_ms(), signature, to: to.cloned() });
    };
    let thread = engine.thread(id).map_err(json)?;
    let past = engine.my_words_to(account_id, &thread.summary.from.email, id).map_err(json)?;
    Ok(Context { signature, ..Context::from_thread(&thread, me, past, crate::now_ms()) })
}

/// Two or three quick replies to a conversation, as `account_id`, to go above `signature`
/// (its lines; none when left out). They stream to the window as stream `id`: each text is
/// the replies so far, as JSON. `ai_stop` keeps what's written.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ai_reply_options<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, Engine>,
    ai: State<'_, Ai>,
    writing: State<'_, Writing>,
    id: String,
    thread_id: String,
    account_id: String,
    signature: Option<Vec<String>>,
) -> Result<Vec<ReplyOption>, Value> {
    let context = context(&engine, Some(&thread_id), None, &account_id, signature.unwrap_or_default())?;
    let stream = writing.begin(&app, &id);
    let mut on_options = |options: &[ReplyOption]| stream.send(&serde_json::to_string(options).unwrap_or_default());
    ai.reply_options(&context, &mut on_options).await.map_err(json)
}

/// Writes `job` as `account_id`, from the composer's `text` (plain text, paragraphs split by
/// blank lines), to go above `signature`, streaming it to the window as stream `id`. Returns
/// the finished text.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn ai_reply<R: Runtime>(
    app: AppHandle<R>,
    engine: State<'_, Engine>,
    ai: State<'_, Ai>,
    writing: State<'_, Writing>,
    id: String,
    thread_id: Option<String>,
    account_id: String,
    job: Job,
    text: String,
    signature: Option<Vec<String>>,
) -> Result<String, Value> {
    let context = context(&engine, thread_id.as_deref(), None, &account_id, signature.unwrap_or_default())?;
    let stream = writing.begin(&app, &id);
    let mut on_text = |text: &str| stream.send(text);
    ai.reply(&job, &text, &context, &mut on_text).await.map_err(json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime, INVOKE_KEY};
    use tauri::Manager;

    /// A work account with Dana's email in it, and an earlier one you sent her.
    fn data() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("data")).unwrap();
        let mut store = moss_store::Store::open(dir.path().join("data/moss.db")).unwrap();
        store
            .upsert_account(&moss_core::Account {
                id: "work".into(),
                name: "Work".into(),
                letter: "W".into(),
                address: "maya@northwind.co".into(),
                sender_name: "Maya Ross".into(),
                color: moss_core::AccountColor::Work,
                provider: moss_core::ProviderKind::Imap,
                org: None,
                org_domain: None,
            })
            .unwrap();
        let source = |id: &str| moss_core::MessageSource {
            account_id: "work".into(),
            mailbox: "INBOX".into(),
            provider_id: id.into(),
            ..Default::default()
        };
        let raw = b"From: Dana <dana@acme.co>\r\nTo: maya@northwind.co\r\nSubject: Deck\r\n\
            Message-ID: <d@acme.co>\r\n\r\nCan you send it?\r\n";
        let thread = store.ingest(raw, &source("1")).unwrap().thread_id;
        let sent = b"From: Maya Ross <maya@northwind.co>\r\nTo: Dana <dana@acme.co>\r\nSubject: Lunch\r\n\
            Message-ID: <l@northwind.co>\r\n\r\nHey Dana! Friday?\r\n\r\n-- \r\nMaya\r\n";
        store.ingest(sent, &source("2")).unwrap();
        (dir, thread)
    }

    fn engine(dir: &tempfile::TempDir) -> Engine {
        let secrets = Arc::new(moss_accounts::MemorySecretStore::new());
        struct Quiet;
        impl moss_engine::Events for Quiet {
            fn emit(&self, _: moss_engine::Event) {}
        }
        use moss_engine::Events;
        Quiet.emit(moss_engine::Event::MailChanged { account_id: "work".into() });
        crate::engine(&dir.path().join("data"), secrets, Arc::new(Quiet), moss_providers::TlsRoots::Platform).unwrap()
    }

    #[test]
    fn the_model_reads_you_the_conversation_and_your_earlier_words() {
        let (dir, thread) = data();
        let engine = engine(&dir);
        let c = context(&engine, Some(&thread), None, "work", vec!["Best".into(), "Maya".into()]).unwrap();
        assert_eq!(c.me, Person { name: "Maya Ross".into(), email: "maya@northwind.co".into() });
        assert_eq!(c.subject, "Deck");
        let messages: Vec<(Person, String)> = c.messages.iter().map(|(p, _, t)| (p.clone(), t.clone())).collect();
        assert_eq!(messages, [(Person { name: "Dana".into(), email: "dana@acme.co".into() }, "Can you send it?".into())]);
        // It knows what day it is.
        assert!(c.now > 1_700_000_000_000);
        assert_eq!(c.past, ["Hey Dana! Friday?"]);
        // It ends the reply to fit the signature that goes below it.
        assert_eq!(c.signature, ["Best", "Maya"]);
        assert_eq!(c.to, None, "a reply is to the conversation's sender");
        // A new email to nobody yet: only you.
        let c = context(&engine, None, None, "work", vec!["Maya".into()]).unwrap();
        assert!(c.messages.is_empty() && c.past.is_empty() && c.subject.is_empty() && c.to.is_none());
        assert_eq!(c.signature, ["Maya"]);
        // A new email to Dana: who it's for, and what you wrote her before, so it sounds like you.
        let dana = Person { name: "Dana".into(), email: "dana@acme.co".into() };
        let c = context(&engine, None, Some(&dana), "work", vec![]).unwrap();
        assert_eq!(c.to, Some(dana.clone()));
        assert_eq!(c.past, ["Hey Dana! Friday?"]);
        assert!(c.messages.is_empty());
        assert_eq!(context(&engine, None, None, "nope", vec![]).unwrap_err()["kind"], "unknownAccount");
        assert_eq!(context(&engine, Some("999"), None, "work", vec![]).unwrap_err()["kind"], "unknownThread");
    }

    fn call(webview: &tauri::WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
        let request = tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "tauri://localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(args),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        };
        tauri::test::get_ipc_response(webview, request).map(|body| body.deserialize::<Value>().unwrap())
    }

    #[test]
    fn the_commands_write_nothing_until_the_writing_model_is_in() {
        let (dir, thread) = data();
        let app = crate::with_commands(mock_builder()).build(mock_context(noop_assets())).unwrap();
        app.manage(crate::ai(&dir.path().join("data"), Arc::new(|_| {}), None, |_| Some(0)));
        app.manage(engine(&dir));
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        let kind = |cmd: &str, args: Value| call(&webview, cmd, args).unwrap_err()["kind"].clone();
        assert_eq!(kind("ai_reply_options", json!({"id": "q1", "threadId": thread, "accountId": "work"})), "notReady");
        assert_eq!(kind("ai_reply_options", json!({"id": "q1", "threadId": thread, "accountId": "work", "signature": ["Best", "Maya"]})), "notReady");
        assert_eq!(kind("ai_reply_options", json!({"id": "q1", "threadId": thread, "accountId": "nope"})), "unknownAccount");
        let reply = |job: Value, text: &str| {
            call(&webview, "ai_reply", json!({"id": "r1", "threadId": thread, "accountId": "work", "job": job, "text": text}))
        };
        assert_eq!(reply(json!({"kind": "accept"}), "").unwrap_err()["kind"], "notReady");
        assert_eq!(reply(json!({"kind": "translate", "language": "French"}), "Hi").unwrap_err()["kind"], "notReady");
        // Nothing to rewrite: nothing to write, model or not.
        assert_eq!(reply(json!({"kind": "shorter"}), " ").unwrap(), json!(""));
        let new = json!({"id": "r2", "accountId": "nope", "job": {"kind": "fix"}, "text": "Hi", "signature": ["Maya"]});
        assert_eq!(call(&webview, "ai_reply", new).unwrap_err()["kind"], "unknownAccount");
        assert_eq!(call(&webview, "ai_stop", json!({"id": "r1"})).unwrap(), json!(false));
    }
}
