use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A password, OAuth access token or API key. Debug output never shows it,
/// and it can't be serialised, so it can't leak into logs or saved state.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Secret(value.into())
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Secret::new(value)
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Secret::new(value)
    }
}

/// How to sign in to IMAP or SMTP. The caller fetches the secret (keychain,
/// OAuth refresh) and passes it in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Credentials {
    /// A password or app password, sent with LOGIN / AUTH PLAIN.
    Password { username: String, password: Secret },
    /// An OAuth 2 access token, sent with XOAUTH2.
    OAuth2 {
        username: String,
        access_token: Secret,
    },
}

impl Credentials {
    pub(crate) fn username(&self) -> &str {
        match self {
            Credentials::Password { username, .. } | Credentials::OAuth2 { username, .. } => {
                username
            }
        }
    }
}

/// How the connection is encrypted. Moss never talks to a mail server in
/// the clear.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Security {
    /// TLS from the first byte (IMAP 993, SMTP 465).
    #[default]
    Tls,
    /// Plain connection upgraded with STARTTLS before signing in (IMAP 143,
    /// SMTP 587). Fails if the server won't upgrade.
    StartTls,
}

/// What a provider can do, so the caller knows what to keep locally.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Read, flagged and folder state live on the server. When false (Resend),
    /// `apply` is a no-op and the local store is the only copy of that state.
    pub server_flags: bool,
}

/// A folder on the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mailbox {
    /// The name the server uses. Pass it back to every other call.
    pub id: String,
    /// The last part of the path, for display.
    pub name: String,
    /// The path separator, if the server uses folders inside folders.
    pub delimiter: Option<String>,
    /// What the folder is for, from the server's special-use flags.
    pub role: Option<MailboxRole>,
    /// False for folders that only hold other folders.
    pub selectable: bool,
}

/// What a folder is for (RFC 6154 special-use, plus Inbox).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailboxRole {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Junk,
    Archive,
    /// Every message in the account (Gmail's All Mail).
    All,
    Flagged,
    Important,
}

/// IMAP system flags plus any keywords.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub draft: bool,
    pub deleted: bool,
    /// Non-system flags such as `$Forwarded` or `$Junk`.
    pub keywords: Vec<String>,
}

impl Flags {
    /// Forwarded from some mail app: `$Forwarded`, or `Forwarded` as some older apps write it.
    pub fn forwarded(&self) -> bool {
        self.keywords.iter().any(|k| {
            k.eq_ignore_ascii_case("$Forwarded") || k.eq_ignore_ascii_case("Forwarded")
        })
    }
}

/// One message exactly as the server holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawMessage {
    /// The provider's id for this message, unique within `mailbox`.
    /// IMAP: `"<uidvalidity>:<uid>"`. Resend: the received email id.
    pub provider_id: String,
    pub mailbox: String,
    pub flags: Flags,
    /// When the server received it (IMAP INTERNALDATE, Resend `created_at`).
    pub received_at: Option<DateTime<Utc>>,
    /// The full RFC 822 message: headers, body and attachments.
    pub raw: Vec<u8>,
    /// Gmail only: the message's labels (X-GM-LABELS).
    pub labels: Vec<String>,
    /// Gmail only: the same message in every folder shares this id (X-GM-MSGID).
    pub gmail_msg_id: Option<u64>,
    /// `raw` is the header block only (see [`crate::Provider::fetch_since_shallow`]).
    pub headers_only: bool,
}

/// Gmail: one message of a conversation, without its body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConversationMessage {
    /// Its id in the mailbox that was asked (All Mail).
    pub provider_id: String,
    pub flags: Flags,
    /// Gmail labels, system ones included (`\Inbox`, `\Sent`, `\Draft`…).
    pub labels: Vec<String>,
    /// Just its Message-ID header, to tell whether it's stored already.
    pub headers: Vec<u8>,
}

/// New flags for a message the caller already has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagUpdate {
    pub provider_id: String,
    pub flags: Flags,
    pub labels: Vec<String>,
}

/// One step of a sync. Store everything in it, then save `checkpoint`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncBatch {
    /// The mailbox was reset on the server (IMAP UIDVALIDITY changed): drop
    /// every message stored for it before storing this batch.
    pub reset: bool,
    pub messages: Vec<RawMessage>,
    pub flag_updates: Vec<FlagUpdate>,
    /// Ids of messages removed from the mailbox on the server.
    pub vanished: Vec<String>,
    pub checkpoint: Checkpoint,
    /// There is more to fetch: call again with the new checkpoint.
    pub more: bool,
}

/// Where a sync got to. Opaque: store it (it's serde-serialisable) and hand
/// it back on the next `fetch_since`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Checkpoint(pub(crate) CheckpointState);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub(crate) enum CheckpointState {
    Imap(crate::imap::ImapCheckpoint),
    Resend(crate::resend::ResendCheckpoint),
}

/// Whether a Message-ID can be represented in the provider's quoted IMAP search.
pub fn searchable_message_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_graphic() && c != '"' && c != '\\')
}

/// A change to apply to messages on the server.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Seen(bool),
    Flagged(bool),
    Answered(bool),
    /// The `$Forwarded` keyword, which other mail apps show as forwarded.
    Forwarded(bool),
    /// Move to another mailbox, by its id.
    Move {
        to: String,
    },
    /// Move to the Archive folder (Gmail: All Mail, which removes the Inbox label).
    Archive,
    /// Move to the Trash folder.
    Trash,
}

/// What `wait_for_change` saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wakeup {
    /// Something changed: sync the mailbox.
    Changed,
    /// Nothing happened before the timeout (or the provider can't push).
    Timeout,
}

/// A name and address, like `Ada Lovelace <ada@example.com>`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    pub name: Option<String>,
    pub email: String,
}

impl Address {
    pub fn new(email: impl Into<String>) -> Self {
        Address {
            name: None,
            email: email.into(),
        }
    }

    pub fn named(name: impl Into<String>, email: impl Into<String>) -> Self {
        Address {
            name: Some(name.into()),
            email: email.into(),
        }
    }
}

/// A file to attach. With `content_id` it's an inline part the HTML can show
/// with `<img src="cid:...">`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub filename: String,
    pub content_type: String,
    pub content_id: Option<String>,
    #[serde(with = "attachment_bytes")]
    pub data: Vec<u8>,
}

mod attachment_bytes {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        STANDARD.decode(encoded).map_err(serde::de::Error::custom)
    }
}

/// A message to send.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutgoingMessage {
    /// Frozen when the delivery is prepared, before network submission.
    #[serde(default)]
    pub date: Option<DateTime<Utc>>,
    pub from: Address,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    pub reply_to: Vec<Address>,
    pub subject: String,
    pub text: Option<String>,
    pub html: Option<String>,
    /// The Message-ID being replied to, so the reply threads.
    pub in_reply_to: Option<String>,
    /// The thread's Message-IDs, oldest first.
    pub references: Vec<String>,
    pub attachments: Vec<Attachment>,
    /// The Message-ID to use. Set it to make retries safe (Resend uses it as
    /// the idempotency key); a new one is made when empty.
    pub message_id: Option<String>,
    /// More headers, written after the threading ones: `X-` names only, such as
    /// `X-Moss-Draft` on a draft's server copy.
    pub headers: Vec<(String, String)>,
}

/// A complete listing from one selected mailbox generation.
/// This identifies the observation, not a verified binding to a local draft.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftListing {
    pub mailbox: String,
    pub generation: String,
    pub provider_ids: Vec<String>,
}

/// A draft's copy in the server's Drafts folder, just saved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedDraft {
    /// Its Message-ID, without <>. New for every save, so servers that drop a second
    /// message with the same id (Gmail) keep each one.
    pub message_id: String,
    /// Its id in the Drafts folder, when the server says.
    pub provider_id: Option<String>,
}

/// A message that was sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentMessage {
    /// The provider's id for the sent message, when it gives one (Resend).
    pub provider_id: Option<String>,
    /// The Message-ID header, with angle brackets.
    pub message_id: String,
    /// The RFC 822 message as Moss built it. File it in Sent: for Resend this
    /// is the only copy.
    pub raw: Vec<u8>,
    /// Moss also appended this copy to the server's Sent folder, so it will
    /// come back on the next sync of Sent.
    pub saved_on_server: bool,
}

#[cfg(test)]
mod tests {
    #[test]
    fn forwarded_is_the_keyword_any_mail_app_sets() {
        let with = |k: &[&str]| super::Flags { keywords: k.iter().map(|k| k.to_string()).collect(), ..Default::default() };
        assert!(with(&["$Forwarded"]).forwarded());
        assert!(with(&["$Junk", "$forwarded"]).forwarded());
        assert!(with(&["Forwarded"]).forwarded());
        assert!(!with(&[]).forwarded());
        assert!(!with(&["$Junk"]).forwarded());
    }

    use super::*;

    #[test]
    fn secrets_never_show_in_debug_output() {
        let secret = Secret::from("hunter2");
        assert_eq!(format!("{secret:?}"), "Secret(***)");
        assert_eq!(secret.expose(), "hunter2");
        assert_eq!(Secret::from(String::from("x")), Secret::new("x"));
    }

    #[test]
    fn credentials_know_their_username_and_hide_their_secret() {
        let password = Credentials::Password {
            username: "ada".into(),
            password: Secret::from("hunter2"),
        };
        let token = Credentials::OAuth2 {
            username: "bob".into(),
            access_token: Secret::from("ya29.secret"),
        };
        assert_eq!((password.username(), token.username()), ("ada", "bob"));
        let debug = format!("{password:?} {token:?}");
        assert!(
            !debug.contains("hunter2") && !debug.contains("ya29"),
            "{debug}"
        );
        assert_eq!(Security::default(), Security::Tls);
    }

    #[test]
    fn addresses_build_with_or_without_a_name() {
        assert_eq!(
            Address::new("a@x.io"),
            Address {
                name: None,
                email: "a@x.io".into()
            }
        );
        assert_eq!(Address::named("Ada", "a@x.io").name.as_deref(), Some("Ada"));
    }

    #[test]
    fn mailbox_roles_serialise_in_snake_case() {
        assert_eq!(serde_json::to_string(&MailboxRole::All).unwrap(), "\"all\"");
    }
    #[test]
    fn message_id_search_requires_a_nonempty_safe_value() {
        for id in ["", "a b", "a\"b", "a\\b", "a\nb", "é@x"] { assert!(!searchable_message_id(id), "{id:?}"); }
        for id in ["x@y", "a+b@example.test", "no-at-sign", "a[]{}<>@x"] { assert!(searchable_message_id(id), "{id:?}"); }
    }

}
