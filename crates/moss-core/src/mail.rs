//! Mail types shared by storage, sync and the UI: messages, threads, drafts and the
//! metadata a provider hands over with each raw message.
//!
//! Times are milliseconds since the Unix epoch. The UI formats them ("9:12 AM",
//! "Sunday, 1 November"), so `Thread.time` and `Message.sentFull` from `types.ts`
//! become `lastMessageAt` and `sentAt` here.

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};

use crate::{Person, Section, SplitId};

impl Person {
    /// Up to two initials for the avatar: "Daniel Okafor" → "DO".
    pub fn initials(&self) -> String {
        self.name
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .take(2)
            .flat_map(char::to_uppercase)
            .collect()
    }

    /// First name for compact lists: "Daniel Okafor" → "Daniel".
    pub fn short(&self) -> String {
        self.name.split_whitespace().next().unwrap_or_default().into()
    }
}

impl Serialize for Person {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut out = serializer.serialize_struct("Person", 4)?;
        out.serialize_field("name", &self.name)?;
        out.serialize_field("email", &self.email)?;
        out.serialize_field("initials", &self.initials())?;
        out.serialize_field("short", &self.short())?;
        out.end()
    }
}

/// Attachment size as the UI shows it: "512 B", "240 KB", "1.2 MB".
pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let n = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if n < KB * KB {
        format!("{} KB", (n / KB).round())
    } else if n < KB * KB * KB {
        format!("{:.1} MB", n / (KB * KB))
    } else {
        format!("{:.1} GB", n / (KB * KB * KB))
    }
}

/// What a mailbox is for. IMAP SPECIAL-USE and Gmail labels map onto these.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum MailboxRole {
    Inbox,
    Sent,
    Drafts,
    Archive,
    Trash,
    Junk,
    /// Every message, e.g. Gmail's All Mail.
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Mailbox {
    pub account_id: String,
    /// The provider's name for it, e.g. "INBOX" or "[Gmail]/Sent Mail".
    pub name: String,
    pub role: Option<MailboxRole>,
}

/// What a provider knows about a message besides its raw RFC 822 bytes.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MessageSource {
    pub account_id: String,
    /// Mailbox name, as in [`Mailbox::name`]. Created on first use.
    pub mailbox: String,
    /// The provider's id for the message in that mailbox: an IMAP UID, a Resend email id.
    pub provider_id: String,
    /// Already read elsewhere (IMAP `\Seen`).
    pub seen: bool,
    /// IMAP `\Flagged` / Gmail starred.
    pub flagged: bool,
    /// Forwarded already, from Moss or another mail app (IMAP `$Forwarded`).
    pub forwarded: bool,
    /// Provider labels or IMAP keywords.
    pub labels: Vec<String>,
    /// When the server received it. Falls back to the Date header, then to now.
    pub received_at: Option<i64>,
    /// Only the headers were fetched (older Sent mail); the body comes when it's opened.
    pub headers_only: bool,
}

/// Where sync got to, per account and mailbox, so the next sync only fetches what's new.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncCheckpoint {
    /// IMAP UIDVALIDITY: if it changes, every stored UID for the mailbox is stale.
    pub uid_validity: Option<u32>,
    /// IMAP UIDNEXT.
    pub uid_next: Option<u32>,
    /// IMAP CONDSTORE HIGHESTMODSEQ.
    pub highest_modseq: Option<u64>,
    /// Opaque page cursor, e.g. the last Resend email id seen.
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    /// Position among the message's attachments, used to fetch its content.
    pub index: u32,
    pub name: String,
    /// For display, e.g. "240 KB". See [`format_size`].
    pub size: String,
    pub bytes: u64,
    pub content_type: String,
    /// Shown in the body (e.g. an embedded image) rather than as a file.
    pub inline: bool,
    /// The words inside the file, once Moss has read it on this Mac: for Ask Moss and the
    /// writing model, filled in only when they ask. Never sent to the window.
    #[serde(skip)]
    pub text: Option<String>,
}

/// Whether a message's remote images load by themselves, and why. See `docs/research/remote-images.md`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ImageTrust {
    /// A new sender: Moss asks first.
    #[default]
    Ask,
    /// You sent it.
    Yours,
    /// You chose to show images from this sender.
    YouAllowed,
    /// You have written to this address.
    YouWrote,
    /// You have written to someone at this sender's company domain.
    SameCompany,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub thread_id: String,
    pub from: Person,
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
    pub bcc: Vec<Person>,
    pub subject: String,
    pub sent_at: i64,
    /// The new text of this message, without quoted replies or signature, split into paragraphs.
    pub paragraphs: Vec<String>,
    /// The whole plain-text body, quotes and all.
    #[serde(rename = "text")]
    pub body_text: String,
    /// The HTML as sent, unsanitised: the UI sanitises it and shows it sandboxed.
    #[serde(rename = "html", default, skip_serializing_if = "Option::is_none")]
    pub body_html: Option<String>,
    pub attachments: Vec<Attachment>,
    pub read: bool,
    #[serde(default)]
    pub image_trust: ImageTrust,
    /// The account's mail server didn't confirm this comes from the domain on its From line,
    /// so who it says it's from can't load its images: only "just this once" does.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sender_unverified: bool,
    /// Only the headers are on this device; the engine fetches the body when it's opened.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub headers_only: bool,
    /// The HTML shows images sent with it that aren't on this device yet; the engine
    /// fetches them when it's opened.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub images_missing: bool,
    /// Forwards of this message sent from Moss, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forwards: Vec<Forward>,
    /// Marked forwarded on the server (IMAP `$Forwarded`), by Moss or another mail app.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub forwarded: bool,
}

/// A forward you sent from Moss: who it went to, when, and the conversation it started.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Forward {
    pub thread_id: String,
    /// Everyone it went to: To, then CC, then BCC.
    pub to: Vec<Person>,
    pub sent_at: i64,
}

/// A row in the thread list. [`Thread`] adds the messages.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: String,
    pub account_id: String,
    pub split: SplitId,
    pub section: Section,
    /// The other person: the latest sender who isn't you.
    pub from: Person,
    pub subject: String,
    /// Preview of the latest message.
    pub summary: String,
    pub last_message_at: i64,
    pub snoozed_until: Option<i64>,
    pub unread: bool,
    pub done: bool,
    /// When it was marked done (ms), for the Done list. `None` while open, and for mail done
    /// before Moss kept the time.
    pub done_at: Option<i64>,
    pub labels: Vec<String>,
    pub message_count: u32,
    pub has_attachments: bool,
    /// When the decision model put it in its section (Reply or Info): how sure it was, in
    /// percent. `None` when the rules did, or you moved it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_sure: Option<u8>,
    /// One or two lines on what the conversation is about, from the writing model, for the
    /// conversations that matter. `None` until it has written one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_summary: Option<String>,
    /// Which of your addresses the conversation came to: the latest email from someone
    /// else, sent to the account's own address or one of its other addresses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_address: Option<String>,
    /// Why there is one: `important` or `replied`, for its label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ai_summary_why: Option<crate::SummaryWhy>,
    /// Who the other person is to you, for the "Block this sender?" bar. Every row says, so
    /// the bar is decided before the thread is opened.
    #[serde(default)]
    pub sender: SenderStatus,
    /// You moved it to the tab it's in, so Moss's sorting leaves it there. Moved to Important,
    /// it is starred (IMAP `\Flagged`) on the server, and a star from another app moves it too.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub moved_by_you: bool,
    /// A deadline in the latest email from someone else, for the amber due chip.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Due>,
    /// The decision model moved it to its split tab. `None` when the rules put it there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split_why: Option<Why>,
    /// Your email is the latest and expects an answer: if none comes, it's back at `until`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting: Option<Waiting>,
    /// The latest email is yours, so Moss can remind you if no one replies.
    #[serde(default)]
    pub you_wrote_last: bool,
}

/// Who made a decision Moss shows: the decision model, a rule, or you.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DecidedBy {
    Ai,
    Rule,
    You,
}

/// Why Moss shows something, so the UI can say it: who decided, how sure the model was,
/// and what a rule went by.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Why {
    pub by: DecidedBy,
    /// How sure the model was, in percent. Only when it decided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sure: Option<u8>,
    /// What a rule went by, in the email's own words: "by Friday", "Can you send it?".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

/// A deadline found in an email.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Due {
    /// The day, as YYYY-MM-DD.
    pub date: String,
    pub why: Why,
}

/// You're waiting on an answer to your email.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Waiting {
    /// When it comes back if no one has replied (ms).
    pub until: i64,
    pub why: Why,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    #[serde(flatten)]
    pub summary: ThreadSummary,
    /// Oldest first.
    pub messages: Vec<Message>,
}

/// Who a thread's sender is to you. Decided by rules, never by AI: see `moss-store`'s
/// `classify::sender_status`.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SenderStatus {
    /// You have written to them, accepted them, or they are at your company.
    #[default]
    Known,
    /// Someone you have never written to: Moss asks whether to block them.
    New,
    /// You blocked them: their mail goes to the Blocked folder.
    Blocked,
}

/// Someone you blocked or accepted, or everyone at a domain, as Settings → Senders lists them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SenderRule {
    pub account_id: String,
    /// An address, or a domain when the rule is about everyone there.
    pub sender: String,
    pub whole_domain: bool,
    /// Blocked; otherwise accepted.
    pub blocked: bool,
    /// When you made it, in milliseconds.
    pub created_at: i64,
}

/// Tab counts: how many threads wait in each split, leaving out those reminded for later.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SplitCounts {
    pub important: u32,
    pub other: u32,
    pub calendar: u32,
    pub newsletters: u32,
    pub receipts: u32,
}

impl SplitCounts {
    pub fn get(&self, split: SplitId) -> u32 {
        match split {
            SplitId::Important => self.important,
            SplitId::Other => self.other,
            SplitId::Calendar => self.calendar,
            SplitId::Newsletters => self.newsletters,
            SplitId::Receipts => self.receipts,
        }
    }

    pub fn get_mut(&mut self, split: SplitId) -> &mut u32 {
        match split {
            SplitId::Important => &mut self.important,
            SplitId::Other => &mut self.other,
            SplitId::Calendar => &mut self.calendar,
            SplitId::Newsletters => &mut self.newsletters,
            SplitId::Receipts => &mut self.receipts,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ComposeMode {
    Reply,
    ReplyAll,
    Forward,
    New,
}

/// An unsent email. Mirrors the UI's `Draft`, minus layout state such as `fieldsOpen`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub id: String,
    /// Expected store-issued content identity for an external edit or handoff.
    /// None authorizes insertion only. Internal recovery snapshots omit this token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    pub mode: ComposeMode,
    pub thread_id: Option<String>,
    pub from_account_id: String,
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
    pub bcc: Vec<Person>,
    pub subject: String,
    pub body: String,
    pub body_html: String,
    pub remind_if_no_reply: bool,
    pub updated_at: i64,
    /// The signature added under the body; `None` for none.
    #[serde(default)]
    pub signature_id: Option<String>,
    /// You picked the signature yourself, so changing From no longer swaps it.
    #[serde(default)]
    pub signature_chosen: bool,
    /// Which of the account's addresses it goes from; `None` for the account's own.
    #[serde(default)]
    pub from_address: Option<String>,
    /// The files that go with it, in the order you added them.
    #[serde(default)]
    pub attachments: Vec<DraftFile>,
}

/// A file going with a draft: one you attached, kept on this device until it is sent, or
/// one of the forwarded email's files, fetched when it goes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DraftFile {
    /// Its id among the files kept for drafts; for a forwarded file, `"<message id>:<index>"`.
    pub id: String,
    pub name: String,
    pub bytes: u64,
    pub content_type: String,
    /// The forwarded email's file this is; `None` for a file you attached.
    #[serde(default)]
    pub forwarded: Option<FileRef>,
}

/// Attachment `index` of a message.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileRef {
    pub message_id: String,
    pub index: u32,
}

/// An email set to send later. It waits on this device until `send_at`, then the engine
/// sends it; a failed try is tried again from `next_try`, so it is never lost.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledEmail {
    /// Stable identity for creation, cancellation and rescheduling.
    pub operation_id: String,
    /// User-intent revision, bounded to JavaScript-safe positive integers.
    pub revision: i64,
    /// The email as it was written, to open again for editing.
    pub draft: Draft,
    /// When it goes (ms).
    pub send_at: i64,
    /// When it is tried next (ms): `send_at`, or later after a failed try.
    pub next_try: i64,
    /// Tries that failed.
    pub tries: u32,
    /// Why the last try failed.
    pub error: Option<String>,
}

/// A piece of a search snippet. Matched words are shown highlighted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SnippetPart {
    pub text: String,
    pub matched: bool,
}

/// One search result: the thread, the best matching message in it, and where it matched.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub thread: ThreadSummary,
    pub message_id: String,
    pub snippet: Vec<SnippetPart>,
    /// Lower is better (SQLite FTS5 bm25).
    pub rank: f64,
    /// The file it matched in, when the match was inside an attachment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<MatchedFile>,
}

/// The attachment a search matched in: its index in the message (see `Attachment::index`)
/// and its name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MatchedFile {
    pub index: u32,
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(name: &str) -> Person {
        Person { name: name.into(), email: "x@example.com".into() }
    }

    fn daniel() -> Person {
        Person { name: "Daniel Okafor".into(), email: "daniel@northwind.co".into() }
    }

    fn summary() -> ThreadSummary {
        ThreadSummary {
            id: "7".into(),
            account_id: "work".into(),
            split: SplitId::Important,
            section: Section::Reply,
            from: daniel(),
            subject: "Q4 budget".into(),
            summary: "Can you sign off?".into(),
            last_message_at: 1_761_988_320_000,
            snoozed_until: None,
            unread: true,
            done: false,
            done_at: None,
            labels: vec!["Finance".into()],
            message_count: 1,
            has_attachments: true,
            ai_sure: None,
            ai_summary: None,
            to_address: None,
            ai_summary_why: None,
            sender: SenderStatus::Known,
            due: None,
            split_why: None,
            waiting: None,
            moved_by_you: false,
            you_wrote_last: false,
        }
    }

    #[test]
    fn thread_flattens_its_summary_like_the_typescript_thread() {
        let thread = Thread {
            summary: summary(),
            messages: vec![Message {
                id: "12".into(),
                thread_id: "7".into(),
                from: daniel(),
                to: vec![],
                cc: vec![],
                bcc: vec![],
                subject: "Q4 budget".into(),
                sent_at: 1_761_988_320_000,
                paragraphs: vec!["Can you sign off?".into()],
                body_text: "Can you sign off?\n".into(),
                body_html: None,
                attachments: vec![Attachment {
                    index: 0,
                    name: "Q4.pdf".into(),
                    size: format_size(245_760),
                    bytes: 245_760,
                    content_type: "application/pdf".into(),
                    inline: false,
                    text: None,
                }],
                read: false,
                image_trust: ImageTrust::YouWrote,
                sender_unverified: false,
                headers_only: false,
                images_missing: false,
                forwards: vec![],
                forwarded: false,
            }],
        };
        let json = serde_json::to_value(&thread).unwrap();
        assert_eq!(json["accountId"], "work");
        assert_eq!(json["section"], "reply");
        assert_eq!(json["from"]["initials"], "DO");
        assert_eq!(json["lastMessageAt"], 1_761_988_320_000_i64);
        assert_eq!(json["messages"][0]["sentAt"], 1_761_988_320_000_i64);
        assert_eq!(json["messages"][0]["attachments"][0]["size"], "240 KB");
        assert_eq!(json["messages"][0]["attachments"][0]["contentType"], "application/pdf");
        // The email view reads `html` first, then `text`, then `paragraphs`.
        assert_eq!(json["messages"][0]["text"], "Can you sign off?\n");
        assert!(json["messages"][0].get("html").is_none());
        assert_eq!(json["messages"][0]["imageTrust"], "you-wrote");
        assert!(json["messages"][0].get("senderUnverified").is_none());
        assert_eq!(json["doneAt"], serde_json::Value::Null);
        let back: Thread = serde_json::from_value(json).unwrap();
        assert_eq!(back, thread);
        let unverified = Message { sender_unverified: true, ..thread.messages[0].clone() };
        assert_eq!(serde_json::to_value(&unverified).unwrap()["senderUnverified"], true);

        let mut html = thread.messages[0].clone();
        html.body_html = Some("<p>Can you sign off?</p>".into());
        let json = serde_json::to_value(&html).unwrap();
        assert_eq!(json["html"], "<p>Can you sign off?</p>");
        assert_eq!(serde_json::from_value::<Message>(json).unwrap(), html);
    }

    #[test]
    fn a_row_and_a_thread_say_whether_the_sender_is_new_or_blocked() {
        let row = |sender| ThreadSummary { sender, ..summary() };
        let thread = |sender| Thread { summary: row(sender), messages: vec![] };
        for (sender, name) in [
            (SenderStatus::Known, "known"),
            (SenderStatus::New, "new"),
            (SenderStatus::Blocked, "blocked"),
        ] {
            let json = serde_json::to_value(row(sender)).unwrap();
            assert_eq!(json["sender"], name);
            assert_eq!(serde_json::from_value::<ThreadSummary>(json).unwrap().sender, sender);
            let json = serde_json::to_value(thread(sender)).unwrap();
            assert_eq!(json["sender"], name);
            assert_eq!(serde_json::from_value::<Thread>(json).unwrap().summary.sender, sender);
        }
        // Threads saved before senders were checked read as known.
        let mut json = serde_json::to_value(thread(SenderStatus::New)).unwrap();
        json.as_object_mut().unwrap().remove("sender");
        assert_eq!(serde_json::from_value::<Thread>(json).unwrap().summary.sender, SenderStatus::Known);
    }

    #[test]
    fn a_done_thread_says_when_it_was_done() {
        let done = ThreadSummary { done: true, done_at: Some(1_761_988_320_000), ..summary() };
        let json = serde_json::to_value(&done).unwrap();
        assert_eq!(json["doneAt"], 1_761_988_320_000_i64);
        assert_eq!(serde_json::from_value::<ThreadSummary>(json).unwrap(), done);
    }

    #[test]
    fn says_which_of_your_addresses_it_came_to_only_when_one_did() {
        let json = serde_json::to_value(summary()).unwrap();
        assert!(json.get("toAddress").is_none());
        let to = ThreadSummary { to_address: Some("hi@rossstudio.art".into()), ..summary() };
        let json = serde_json::to_value(&to).unwrap();
        assert_eq!(json["toAddress"], "hi@rossstudio.art");
        assert_eq!(serde_json::from_value::<ThreadSummary>(json).unwrap(), to);
    }

    #[test]
    fn says_how_sure_the_ai_was_only_when_it_sorted_the_thread() {
        let json = serde_json::to_value(summary()).unwrap();
        assert!(json.get("aiSure").is_none());
        let sorted = ThreadSummary { ai_sure: Some(91), ..summary() };
        let json = serde_json::to_value(&sorted).unwrap();
        assert_eq!(json["aiSure"], 91);
        assert_eq!(serde_json::from_value::<ThreadSummary>(json).unwrap(), sorted);
    }

    #[test]
    fn says_you_moved_it_only_when_you_did() {
        let json = serde_json::to_value(summary()).unwrap();
        assert!(json.get("movedByYou").is_none());
        let moved = ThreadSummary { moved_by_you: true, ..summary() };
        let json = serde_json::to_value(&moved).unwrap();
        assert_eq!(json["movedByYou"], true);
        assert_eq!(serde_json::from_value::<ThreadSummary>(json).unwrap(), moved);
    }

    #[test]
    fn says_what_moss_found_and_why_only_when_it_found_something() {
        let json = serde_json::to_value(summary()).unwrap();
        for key in ["due", "splitWhy", "waiting"] {
            assert!(json.get(key).is_none(), "{key}");
        }
        assert_eq!(json["youWroteLast"], false);
        let found = ThreadSummary {
            due: Some(Due {
                date: "2026-11-13".into(),
                why: Why { by: DecidedBy::Rule, sure: None, evidence: Some("by Friday".into()) },
            }),
            split_why: Some(Why { by: DecidedBy::Ai, sure: Some(88), evidence: None }),
            waiting: Some(Waiting { until: 1_762_000_000_000, why: Why { by: DecidedBy::You, sure: None, evidence: None } }),
            you_wrote_last: true,
            ..summary()
        };
        let json = serde_json::to_value(&found).unwrap();
        assert_eq!(json["due"], serde_json::json!({"date": "2026-11-13", "why": {"by": "rule", "evidence": "by Friday"}}));
        assert_eq!(json["splitWhy"], serde_json::json!({"by": "ai", "sure": 88}));
        assert_eq!(json["waiting"], serde_json::json!({"until": 1_762_000_000_000_i64, "why": {"by": "you"}}));
        assert_eq!(json["youWroteLast"], true);
        assert_eq!(serde_json::from_value::<ThreadSummary>(json).unwrap(), found);
    }

    #[test]
    fn split_counts_read_and_write_by_split() {
        let mut counts = SplitCounts::default();
        let all = [
            SplitId::Important,
            SplitId::Other,
            SplitId::Calendar,
            SplitId::Newsletters,
            SplitId::Receipts,
        ];
        for (n, split) in all.iter().enumerate() {
            *counts.get_mut(*split) += n as u32 + 1;
        }
        for (n, split) in all.iter().enumerate() {
            assert_eq!(counts.get(*split), n as u32 + 1);
        }
        assert_eq!(serde_json::to_value(counts).unwrap()["newsletters"], 4);
    }

    #[test]
    fn draft_and_compose_mode_match_the_ui() {
        let draft = Draft {
        revision: None,
            id: "d1".into(),
            mode: ComposeMode::ReplyAll,
            thread_id: Some("7".into()),
            from_account_id: "work".into(),
            to: vec![daniel()],
            cc: vec![],
            bcc: vec![],
            subject: "Re: Q4 budget".into(),
            body: "Signed.".into(),
            body_html: "<p>Signed.</p>".into(),
            remind_if_no_reply: true,
            updated_at: 1,
            signature_id: Some("work-short".into()),
            signature_chosen: true,
            from_address: Some("hi@rossstudio.art".into()),
            attachments: vec![
                DraftFile {
                    id: "3".into(),
                    name: "plan.pdf".into(),
                    bytes: 2048,
                    content_type: "application/pdf".into(),
                    forwarded: None,
                },
                DraftFile {
                    id: "9:1".into(),
                    name: "deck.key".into(),
                    bytes: 10,
                    content_type: String::new(),
                    forwarded: Some(FileRef { message_id: "9".into(), index: 1 }),
                },
            ],
        };
        let json = serde_json::to_value(&draft).unwrap();
        assert_eq!(json["attachments"][0]["contentType"], "application/pdf");
        assert_eq!(json["attachments"][0]["forwarded"], serde_json::Value::Null);
        assert_eq!(json["attachments"][1]["forwarded"], serde_json::json!({"messageId": "9", "index": 1}));
        assert_eq!(json["fromAddress"], "hi@rossstudio.art");
        assert_eq!(json["mode"], "reply-all");
        assert_eq!(json["fromAccountId"], "work");
        assert_eq!(json["remindIfNoReply"], true);
        assert_eq!(json["signatureId"], "work-short");
        assert_eq!(json["signatureChosen"], true);
        assert_eq!(serde_json::from_value::<Draft>(json).unwrap(), draft);
    }

    #[test]
    fn a_draft_without_a_signature_choice_reads_as_none() {
        let json = serde_json::json!({
            "id": "d1", "mode": "new", "threadId": null, "fromAccountId": "work", "to": [], "cc": [],
            "bcc": [], "subject": "", "body": "", "bodyHtml": "", "remindIfNoReply": false, "updatedAt": 1
        });
        let draft: Draft = serde_json::from_value(json).unwrap();
        assert_eq!((draft.signature_id.as_deref(), draft.signature_chosen), (None, false));
        // Nor a From address: it goes from the account's own.
        assert_eq!(draft.from_address, None);
        // Nor files.
        assert!(draft.attachments.is_empty());
        assert_eq!(serde_json::to_value(&draft).unwrap()["fromAddress"], serde_json::Value::Null);
    }

    #[test]
    fn provider_metadata_and_checkpoints_use_camel_case() {
        let source = MessageSource {
            account_id: "work".into(),
            mailbox: "INBOX".into(),
            provider_id: "42".into(),
            received_at: Some(5),
            ..Default::default()
        };
        let json = serde_json::to_value(&source).unwrap();
        assert_eq!(json["providerId"], "42");
        assert_eq!(json["receivedAt"], 5);
        let checkpoint = SyncCheckpoint { uid_validity: Some(9), ..Default::default() };
        assert_eq!(serde_json::to_value(&checkpoint).unwrap()["uidValidity"], 9);
        let mailbox = Mailbox {
            account_id: "work".into(),
            name: "Sent".into(),
            role: Some(MailboxRole::Sent),
        };
        assert_eq!(serde_json::to_value(&mailbox).unwrap()["role"], "sent");
    }

    #[test]
    fn search_hits_carry_the_thread_and_snippet() {
        let hit = SearchHit {
            thread: summary(),
            message_id: "12".into(),
            snippet: vec![
                SnippetPart { text: "the ".into(), matched: false },
                SnippetPart { text: "budget".into(), matched: true },
            ],
            rank: -1.5,
            file: None,
        };
        let json = serde_json::to_value(&hit).unwrap();
        assert_eq!(json["thread"]["subject"], "Q4 budget");
        assert_eq!(json["messageId"], "12");
        assert_eq!(json["snippet"][1]["matched"], true);
        assert!(json.get("file").is_none(), "only a match inside a file names one");
        let in_file = SearchHit { file: Some(MatchedFile { index: 2, name: "budget-q4.xlsx".into() }), ..hit };
        assert_eq!(serde_json::to_value(&in_file).unwrap()["file"], serde_json::json!({"index": 2, "name": "budget-q4.xlsx"}));
    }

    #[test]
    fn initials_take_the_first_letter_of_up_to_two_words() {
        assert_eq!(person("Daniel Okafor").initials(), "DO");
        assert_eq!(person("Ana de la Cruz").initials(), "AD");
        assert_eq!(person("mum").initials(), "M");
        assert_eq!(person("  Lena   Vogt ").initials(), "LV");
        assert_eq!(person("élodie roux").initials(), "ÉR");
        assert_eq!(person("").initials(), "");
    }

    #[test]
    fn short_name_is_the_first_word() {
        assert_eq!(person("Daniel Okafor").short(), "Daniel");
        assert_eq!(person(" Mum ").short(), "Mum");
        assert_eq!(person("").short(), "");
    }

    #[test]
    fn person_serialises_with_initials_and_short_name() {
        let daniel = Person { name: "Daniel Okafor".into(), email: "daniel@northwind.co".into() };
        let json = serde_json::to_value(&daniel).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "name": "Daniel Okafor",
                "email": "daniel@northwind.co",
                "initials": "DO",
                "short": "Daniel"
            })
        );
        let back: Person = serde_json::from_value(json).unwrap();
        assert_eq!(back, daniel);
    }

    #[test]
    fn sizes_read_like_the_design() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(245_760), "240 KB");
        assert_eq!(format_size(1_048_575), "1024 KB");
        assert_eq!(format_size(1_258_291), "1.2 MB");
        assert_eq!(format_size(3 * 1024 * 1024 * 1024), "3.0 GB");
    }
}
