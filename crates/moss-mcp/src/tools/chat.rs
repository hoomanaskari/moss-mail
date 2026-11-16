//! Moss's own chat (Ask Moss, on the writing model on this Mac) uses these same tools, without
//! the server, so it can do with mail what an agent can. Two differences:
//!
//! - It has its own tools to search, read threads and files, and find people, which cite what
//!   they found, so the ones here that do the same are left out ([`CHAT_LEAVES_OUT`]).
//! - One tool writes every email, `write_email`: `send` with its own words. It never sends from
//!   the chat: the email waits as a draft, under the answer, for the user to press Send (or Send
//!   later, at the time they asked for). A small model picks between fewer tools better.

use rmcp::handler::server::wrapper::Parameters;
use serde::de::DeserializeOwned;

use super::*;

/// MCP tools the chat has its own of: `search_mail`, `read_thread` and `find_people` in
/// `moss_ai::tools`. `get_attachment` saves a file for an agent to open; the chat reads files
/// with `read_attachment` and shows them as chips.
/// `create_draft` is `write_email` here.
pub const CHAT_LEAVES_OUT: &[&str] = &[
    "search_mail",
    "get_thread",
    "find_contacts",
    "get_attachment",
    "create_draft",
];

/// The reminders, told apart for a small model, which reached for remind_if_no_reply to stop
/// a reminder.
const REMIND_ME: &str = "Remind me: hides an email until a time (until), then brings it back to the top of the inbox. To stop a reminder and bring the email back now, call it without until.";
const REMIND_IF_NO_REPLY: &str = "Only for an email the user sent: brings it back if no one replies in a few days (on=false stops waiting). Not for other reminders: those are remind_me.";

/// What the chat's `write_email` is, for the model.
const WRITE_EMAIL: &str = "Writes an email for the user: a new one, a reply (reply_to_thread, and reply_all to answer everyone), a forward (forward_thread), or one of their drafts by draft_id. Always give body, the email's text (empty for a forward with nothing to add). It is never sent by itself: Moss shows it under your answer and the user presses Send. Give send_at when the user wants it to go later.";

/// What one tool did, for the chat.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ran {
    /// What the model reads.
    pub text: String,
    /// Threads it listed, by id: the model may cite them.
    pub threads: Vec<String>,
    /// A draft it wrote or changed, or the one waiting to be sent: shown under the answer.
    pub draft: Option<Draft>,
    /// When the user asked for the draft to go, with Send later (ms).
    pub send_at: Option<i64>,
    /// What it changed, as the chat lists it: "Marked done: Lunch on Friday".
    pub did: Option<String>,
}

/// The argument named `key`, when it's text.
fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map_or(String::new(), |first| {
        first.to_uppercase().chain(c).collect()
    })
}

impl MossTools {
    /// The chat's tools, as the function definitions the model reads.
    pub fn chat_specs(&self) -> Vec<Value> {
        self.tool_router
            .list_all()
            .into_iter()
            .filter(|t| !CHAT_LEAVES_OUT.contains(&t.name.as_ref()))
            .map(|t| {
                let mut schema = (*t.input_schema).clone();
                // Words the model doesn't need, in a context that's short.
                schema.remove("$schema");
                schema.remove("title");
                if let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) {
                    properties.remove("files");
                    properties.remove("attachments");
                    // The chat's write_email never sends, so has nothing to retry.
                    properties.remove("request_id");
                }
                let mut parameters = Value::Object(schema);
                let (name, description) = match t.name.as_ref() {
                    "send" => {
                        // A small model leaves out what isn't required, and an email needs its text.
                        parameters["required"] = json!(["body"]);
                        ("write_email".into(), WRITE_EMAIL.into())
                    }
                    "remind_me" => (t.name.to_string(), REMIND_ME.into()),
                    "remind_if_no_reply" => (t.name.to_string(), REMIND_IF_NO_REPLY.into()),
                    _ => (
                        t.name.to_string(),
                        t.description.unwrap_or_default().to_string(),
                    ),
                };
                json!({"type": "function", "function": {
                    "name": name,
                    "description": description,
                    "parameters": parameters,
                }})
            })
            .collect()
    }

    /// What the chat shows while tool `name` runs. `None`: not one of the chat's tools here.
    pub fn chat_label(name: &str, args: &Value) -> Option<String> {
        let on = |key: &str| args.get(key).and_then(Value::as_bool).unwrap_or(true);
        let label = match name {
            "list_accounts" => "Looking at your accounts",
            "list_inbox" => "Looking through the inbox",
            "list_drafts" => "Looking at your drafts",
            "list_done" => "Looking through Done",
            "list_blocked" => "Looking through Blocked",
            "list_scheduled" => "Looking at emails set to send later",
            "mark_done" if on("done") => "Marking an email Done",
            "mark_done" => "Moving an email back to the inbox",
            "mark_read" if on("read") => "Marking an email read",
            "mark_read" => "Marking an email unread",
            "remind_me" if args.get("until").is_some_and(|u| !u.is_null()) => "Setting a reminder",
            "remind_me" => "Stopping a reminder",
            "remind_if_no_reply" if on("on") => "Reminding you if no one replies",
            "remind_if_no_reply" => "Stopping waiting for a reply",
            "move_thread" => "Moving an email",
            "delete_thread" => "Deleting an email",
            "restore_thread" => "Bringing an email back from Trash",
            "block_sender" => "Blocking a sender",
            "accept_sender" => "Accepting a sender",
            "unblock_sender" => "Unblocking a sender",
            "write_email" => "Writing an email",
            "update_draft" => "Changing a draft",
            "delete_draft" => "Deleting a draft",
            "cancel_scheduled" => "Cancelling a send later",
            "sync_now" => "Checking for new mail",
            _ => return None,
        };
        Some(label.to_string())
    }

    /// Runs tool `name` for the chat. `None`: not one of the chat's tools here.
    pub async fn chat_run(&self, name: &str, args: Value) -> Option<Result<Ran, String>> {
        Self::chat_label(name, &args)?;
        // Schema omission is not authorization: reject payloads even if the model invents them.
        if ["files", "attachments"].iter().any(|key| args.get(*key).is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))) {
            return Some(Err("Attach files in Moss yourself, or forward attachments already in your mail. Ask Moss cannot read local paths or create file payloads.".into()));
        }
        Some(self.ran(name, args).await)
    }

    async fn ran(&self, name: &str, args: Value) -> Result<Ran, String> {
        // A draft that's about to go: its subject, for what the chat says it did.
        let draft_subject = arg(&args, "draft_id")
            .or(arg(&args, "scheduled_id"))
            .and_then(|id| self.engine.drafts().ok()?.into_iter().find(|d| d.id == id))
            .map(|d| d.subject);
        let mut send_at = None;
        let moved_to = arg(&args, "to").map(|to| to.trim().to_ascii_lowercase());
        let args_until = args.get("until").is_some_and(|u| !u.is_null());
        let text = match name {
            "write_email" => {
                let mut args: SendArgs = parsed(args)?;
                // A small model puts who it writes to in from as well. from is only ever one of
                // the user's addresses, so one that isn't goes, and Moss picks it as it would.
                let accounts = self.engine.accounts().map_err(super::text)?;
                if args.from.as_deref().is_some_and(|f| !accounts.iter().any(|a| super::sends_from(a, f).is_ok())) {
                    args.from = None;
                }
                // A small model may leave the text out. Only a forward goes without words of its own.
                let words = args.body.as_deref().is_some_and(|b| !b.trim().is_empty());
                if !words && args.draft_id.is_none() && args.forward_thread.is_none() {
                    return Err("Nothing was written. Write it yourself: a short, sensible email from what the user asked (read the thread first if it's a reply), and call write_email again with it in body. Don't ask the user for the words.".into());
                }
                let new = args.draft_id.is_none() && args.reply_to_thread.is_none() && args.forward_thread.is_none();
                if new && args.subject.as_deref().is_none_or(|s| s.trim().is_empty()) {
                    return Err("A new email needs a subject. Call write_email again with one.".into());
                }
                send_at = args.send_at.as_deref().map(time).transpose()?;
                let forwarded = args.forward_thread.as_deref().and_then(|id| self.engine.thread(id.trim()).ok()).map(|t| t.summary.subject);
                let draft = self.drafted(send_fields(args))?;
                let draft = self.engine.save_draft(&draft).map_err(super::text)?;
                self.drafts_changed(&draft.from_account_id);
                let mut v = self.view(&draft)?;
                // Its words are under the answer already: a small model given them back pastes
                // them into its answer too. list_drafts has them when it's to be changed.
                v["body"] = Value::Null;
                // Said plainly, so a model that meant a reply sees when it wrote a new email.
                let kind = match draft.mode {
                    ComposeMode::Reply | ComposeMode::ReplyAll => "A reply in its thread".to_string(),
                    ComposeMode::Forward => format!("A forward of “{}”", forwarded.unwrap_or_default()),
                    ComposeMode::New => "A new email, not a reply in any thread".to_string(),
                };
                v["note"] = json!(format!(
                    "{kind}. Not sent yet: Moss shows it under your answer with a Send button. Don't write it out in your answer: say in one sentence what you wrote, and tell the user to check it and press Send."
                ));
                compact(&v)
            }
            _ => self.dispatch(name, args).await?,
        };
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let threads = v["threads"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t["id"].as_str().map(String::from))
            .collect();
        let draft = match v["draftId"].as_str() {
            Some(id) if name != "delete_draft" => self
                .engine
                .drafts()
                .map_err(super::text)?
                .into_iter()
                .find(|d| d.id == id),
            _ => None,
        };
        // As the chat says it: an agent's wording has times and split ids in it.
        let said = match (name, moved_to.as_deref()) {
            ("remind_me", _) if args_until => Some("reminder set".to_string()),
            ("remind_me", _) => Some("reminder stopped".to_string()),
            ("move_thread", Some("auto")) => Some("back to Moss’s sorting".to_string()),
            ("move_thread", Some(to)) => Some(format!("moved to {}", capitalised(to))),
            _ => None,
        };
        let did = said.as_deref().or(v["done"].as_str()).map(|done| {
            let about = v["threadId"]
                .as_str()
                .and_then(|id| self.engine.thread(id).ok())
                .map(|t| t.summary.subject)
                .or(draft_subject)
                .or(draft.as_ref().map(|d| d.subject.clone()));
            match about {
                Some(subject) => format!("{}: {subject}", capitalised(done)),
                None => capitalised(done),
            }
        });
        Ok(Ran {
            text,
            threads,
            draft,
            send_at,
            did,
        })
    }

    async fn dispatch(&self, name: &str, args: Value) -> Result<String, String> {
        match name {
            "list_accounts" => self.list_accounts().await,
            "list_inbox" => self.list_inbox(Parameters(parsed(args)?)).await,
            "list_drafts" => self.list_drafts().await,
            "list_done" => self.list_done(Parameters(parsed(args)?)).await,
            "list_blocked" => self.list_blocked(Parameters(parsed(args)?)).await,
            "list_scheduled" => self.list_scheduled().await,
            "mark_done" => self.mark_done(Parameters(parsed(args)?)).await,
            "mark_read" => self.mark_read(Parameters(parsed(args)?)).await,
            "remind_me" => self.remind_me(Parameters(parsed(args)?)).await,
            "remind_if_no_reply" => self.remind_if_no_reply(Parameters(parsed(args)?)).await,
            "move_thread" => self.move_thread(Parameters(parsed(args)?)).await,
            "delete_thread" => self.delete_thread(Parameters(parsed(args)?)).await,
            "restore_thread" => self.restore_thread(Parameters(parsed(args)?)).await,
            "block_sender" => self.block_sender(Parameters(parsed(args)?)).await,
            "accept_sender" => self.accept_sender(Parameters(parsed(args)?)).await,
            "unblock_sender" => self.unblock_sender(Parameters(parsed(args)?)).await,
            "update_draft" => self.update_draft(Parameters(parsed(args)?)).await,
            "delete_draft" => self.delete_draft(Parameters(parsed(args)?)).await,
            "cancel_scheduled" => self.cancel_scheduled(Parameters(parsed(args)?)).await,
            _ => self.sync_now(Parameters(parsed(args)?)).await,
        }
    }
}

/// The model's arguments as a tool's, or why not. No arguments reads as `{}`.
fn parsed<T: DeserializeOwned>(args: Value) -> Result<T, String> {
    let args = if args.is_null() { json!({}) } else { args };
    serde_json::from_value(args).map_err(|e| {
        // A small model may act before it has looked: it asked the user instead of searching.
        if args_without_thread(&e) {
            "Say which email in thread_id: find it with search_mail first, then use the id it shows as [t:ID]. Don't ask the user for it.".into()
        } else {
            format!("Couldn't read the arguments: {e}")
        }
    })
}

fn args_without_thread(e: &serde_json::Error) -> bool {
    e.to_string().contains("missing field `thread_id`")
}
