//! Moss's mail server backends.
//!
//! Every backend implements [`Provider`]. Messages come out as raw RFC 822
//! bytes ([`RawMessage`]); parsing and storing them is `moss-store`'s job.
//!
//! - [`ResendProvider`]: Resend's receiving API, with sending through Resend.
//! - [`ImapProvider`]: IMAP over TLS (Gmail, iCloud, Yahoo, Fastmail, any
//!   IMAP server), with sending through SMTP.
//!
//! Syncing is incremental and resumable: call [`Provider::fetch_since`] with
//! the last saved [`Checkpoint`], store the batch, save its checkpoint, and
//! repeat while [`SyncBatch::more`] is true. Delivery is at-least-once, so the
//! store should ignore a `provider_id` it already has.

mod error;
pub mod imap;
mod mime;
mod parts;
pub mod resend;
pub mod smtp;
mod stall;
#[cfg(test)]
mod testing;
mod tls;
mod types;

use std::time::Duration;

use async_trait::async_trait;

pub use error::{Error, Result};
pub use imap::{ImapConfig, ImapProvider};
pub use resend::{ResendDomain, ResendProvider};
pub use smtp::SmtpConfig;
pub use tls::TlsRoots;
pub use types::*;

/// A mail account's server, whatever the protocol.
#[async_trait]
pub trait Provider: Send + Sync {
    fn capabilities(&self) -> Capabilities;

    /// Every mailbox, with its special-use role when the server says.
    async fn list_mailboxes(&self) -> Result<Vec<Mailbox>>;

    /// The next batch of changes in `mailbox` since `checkpoint` (`None` on
    /// the first sync): new mail first, then older mail newest-first, at most
    /// `limit` messages per batch.
    async fn fetch_since(
        &self,
        mailbox: &str,
        checkpoint: Option<&Checkpoint>,
        limit: usize,
    ) -> Result<SyncBatch>;

    /// Like [`Provider::fetch_since`], but older mail (the backfill) comes as headers only
    /// ([`RawMessage::headers_only`]), for mail that's only needed for threading. New mail
    /// still comes whole. Providers that can't fetch headers alone fetch whole messages.
    async fn fetch_since_shallow(
        &self,
        mailbox: &str,
        checkpoint: Option<&Checkpoint>,
        limit: usize,
    ) -> Result<SyncBatch> {
        self.fetch_since(mailbox, checkpoint, limit).await
    }

    /// One full message by its `provider_id`.
    async fn fetch_message(&self, mailbox: &str, provider_id: &str) -> Result<RawMessage>;

    /// The decoded bytes of one part of a message (an attachment), by its IMAP body
    /// section ("2", "1.2"; "1" when the message is a single part). The default fetches
    /// the whole message and picks the part out.
    async fn fetch_part(&self, mailbox: &str, provider_id: &str, section: &str) -> Result<Vec<u8>> {
        let message = self.fetch_message(mailbox, provider_id).await?;
        parts::part(&message.raw, section)
            .ok_or_else(|| Error::NotFound(format!("part {section} of message {provider_id}")))
    }

    /// Mark read, flag, archive or move messages in `mailbox`.
    async fn apply(&self, mailbox: &str, provider_ids: &[String], change: &Change) -> Result<()>;

    /// Undo an archive: put the messages with these Message-IDs (without <>)
    /// back in the Inbox. Ids the archive doesn't hold are skipped. Providers
    /// without folders have nothing to do.
    async fn unarchive(&self, _message_ids: &[String]) -> Result<()> {
        Ok(())
    }

    /// Delete: moves the messages with these Message-IDs (without <>) to Trash, from the
    /// Inbox or the archive. Providers without folders have nothing to do.
    async fn trash(&self, _message_ids: &[String]) -> Result<()> {
        Ok(())
    }

    /// Undo a delete: from Trash back to the Inbox, or with `to_inbox` false to the archive.
    /// Ids no longer in Trash are skipped.
    async fn untrash(&self, _message_ids: &[String], _to_inbox: bool) -> Result<()> {
        Ok(())
    }

    /// Makes a folder, such as Blocked. One that's there already is fine. Providers without
    /// folders have nothing to do.
    async fn create_mailbox(&self, _name: &str) -> Result<()> {
        Ok(())
    }

    /// Appends a draft in `mailbox`, marked \\Draft and \\Seen, and returns its acknowledged receipt.
    /// No lookup or cleanup follows acceptance. The caller owns durable receipt bookkeeping
    /// and replacement cleanup. A provider ID may be unknown; it is not verified origin.
    /// `None` from providers without folders (Resend): their drafts stay on this device.
    async fn save_draft(
        &self,
        _mailbox: &str,
        _draft: &OutgoingMessage,
    ) -> Result<Option<SavedDraft>> {
        Ok(None)
    }

    /// Removes drafts from the Drafts folder `mailbox` for good, by Message-ID (without <>):
    /// sent or discarded. Ones that aren't there are skipped.
    async fn delete_drafts(&self, _mailbox: &str, _message_ids: &[String]) -> Result<()> {
        Ok(())
    }

    /// Send a message. The result holds the RFC 822 copy to file in Sent.
    async fn send(&self, message: &OutgoingMessage) -> Result<SentMessage>;

    /// Wait until `mailbox` changes on the server (IMAP IDLE), or `timeout`.
    /// Providers without push just wait out the timeout, so the caller can
    /// loop on this and sync after every wakeup.
    async fn wait_for_change(&self, mailbox: &str, timeout: Duration) -> Result<Wakeup>;

    /// Every message id the server has in `mailbox`, when `fetch_since` can't report
    /// deletions itself (IMAP without QRESYNC, like Gmail). `None` when there's no need.
    async fn all_ids(&self, _mailbox: &str) -> Result<Option<Vec<String>>> {
        Ok(None)
    }

    /// A complete draft listing with its selected mailbox generation, including when empty.
    /// Unsupported providers return `None`; they must not invent an empty listing.
    /// Connection identity and local-copy verification remain the caller's responsibility.
    async fn list_drafts(&self, _mailbox: &str) -> Result<Option<DraftListing>> {
        Ok(None)
    }

    /// Several full messages from one mailbox, newest first. Ones no longer there are
    /// left out.
    async fn fetch_messages(
        &self,
        mailbox: &str,
        provider_ids: &[String],
    ) -> Result<Vec<RawMessage>> {
        let mut out = Vec::new();
        for id in provider_ids {
            match self.fetch_message(mailbox, id).await {
                Ok(message) => out.push(message),
                Err(Error::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(out)
    }

    /// Like [`Provider::fetch_messages`], but only each message's header block, marked
    /// `headers_only`: for older mail, whose body is fetched when someone opens it.
    async fn fetch_headers(
        &self,
        mailbox: &str,
        provider_ids: &[String],
    ) -> Result<Vec<RawMessage>> {
        let mut messages = self.fetch_messages(mailbox, provider_ids).await?;
        for m in &mut messages {
            let end = m.raw.windows(4).position(|w| w == b"\r\n\r\n").map_or(m.raw.len(), |i| i + 4);
            m.raw.truncate(end);
            m.headers_only = true;
        }
        Ok(messages)
    }

    /// Gmail: the conversation (X-GM-THRID) of each of these messages in `mailbox`, for
    /// the ones still there. `None` when the server doesn't group mail into conversations.
    async fn conversations_of(
        &self,
        _mailbox: &str,
        _provider_ids: &[String],
    ) -> Result<Option<Vec<(String, u64)>>> {
        Ok(None)
    }

    /// Gmail: every message of these conversations in `mailbox` (All Mail, where archived
    /// mail is), newest first, without bodies.
    async fn conversation_messages(
        &self,
        _mailbox: &str,
        _conversations: &[u64],
    ) -> Result<Vec<ConversationMessage>> {
        Ok(Vec::new())
    }
}
