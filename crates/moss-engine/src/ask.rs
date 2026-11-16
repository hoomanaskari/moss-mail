//! What Ask Moss looks up beyond search: files people sent, and people.

use moss_core::{Contact, FileHit, FileQuery};

use crate::{Engine, Result};

/// How many contacts a people search looks through.
const PEOPLE: usize = 2_000;

impl Engine {
    /// Emails on this device, for "2,431 emails on this Mac".
    pub fn mail_count(&self) -> Result<u64> {
        Ok(self.shared.store().mail_count()?)
    }

    /// Files people sent, newest first (see [`moss_store::Store::find_files`]).
    pub fn find_files(&self, query: &FileQuery) -> Result<Vec<FileHit>> {
        Ok(self.shared.store().find_files(query)?)
    }

    /// Attachment `index` of a message, with where it came from.
    pub fn file(&self, message_id: &str, index: u32) -> Result<FileHit> {
        let file = self.shared.store().file(message_id, index)?;
        file.ok_or_else(|| crate::Error::Unavailable(format!("attachment {index} of message {message_id}")))
    }

    /// People whose name or address has `query` in it, the ones you write to most first, with
    /// how often each way and when last.
    pub fn find_people(&self, query: &str, limit: usize) -> Result<Vec<Contact>> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let found = self.shared.store().contacts(PEOPLE)?.into_iter().filter(|c| {
            let who = format!("{} {}", c.person.name, c.person.email).to_lowercase();
            words.iter().all(|w| who.contains(w))
        });
        Ok(found.take(limit).collect())
    }
}
