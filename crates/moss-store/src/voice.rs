//! What you wrote to someone before, so an AI reply to them sounds like you.

use rusqlite::params;

use crate::codec::parse_id;
use crate::images::literal;
use crate::{text, Result, Store};

impl Store {
    /// The new text of up to `limit` emails you sent `email` from this account, newest first,
    /// leaving out the conversation `except` (it's read already). Emails whose text isn't
    /// on this device are skipped.
    pub fn my_words_to(&self, account_id: &str, email: &str, except: &str, limit: usize) -> Result<Vec<String>> {
        // Recipients are stored as JSON: {"name":…,"email":"alex@example.test",…}.
        let pattern = format!("%\"email\":\"{}\"%", literal(&email.trim().to_lowercase()));
        let mut stmt = self.conn.prepare(
            "SELECT id FROM messages WHERE account_id = ?1 AND from_me = 1 AND thread_id != ?2
               AND (lower(to_json) LIKE ?3 ESCAPE '\\' OR lower(cc_json) LIKE ?3 ESCAPE '\\')
             ORDER BY sent_at DESC, id DESC LIMIT ?4",
        )?;
        // Some may have no text here, so look at a few more.
        let rows = stmt.query_map(params![account_id, parse_id(except), pattern, (limit * 3) as i64], |r| r.get(0))?;
        let ids: Vec<i64> = rows.collect::<rusqlite::Result<_>>()?;
        let mut words = Vec::new();
        for id in ids {
            let Some(body) = self.body(id)? else { continue };
            let text = text::strip_quoted(&body.text).trim().to_string();
            if !text.is_empty() && words.len() < limit {
                words.push(text);
            }
        }
        Ok(words)
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::*;
    use moss_core::{Mailbox, MailboxRole, MessageSource};

    #[test]
    fn your_newest_emails_to_them_in_this_account_without_quotes() {
        let mut store = store_with_work();
        store.upsert_account(&account("home", "maya@home.me")).unwrap();
        store
            .upsert_mailbox(&Mailbox { account_id: "work".into(), name: "Sent".into(), role: Some(MailboxRole::Sent) })
            .unwrap();
        let sent = |store: &mut crate::Store, mail: Mail, id: &str, at: i64, to: &str, body: &str| {
            let raw = mail.id(id).at(at).from("Maya Ross <maya@northwind.co>").to(to).body(body).build();
            store.ingest(&raw, &MessageSource { mailbox: "Sent".into(), ..source(id) }).unwrap()
        };
        let here = ingest(&mut store, "in", &mail().id("in@x").at(NOV_1).body("Can you sign off the budget?").build());
        let other = |subject: &str| mail().subject(subject);
        sent(&mut store, other("Lunch"), "a@x", NOV_1 - 3 * DAY, "Daniel <Daniel@Northwind.co>", "Hey Daniel! Done.\n\n> old words");
        sent(&mut store, other("Plans"), "b@x", NOV_1 - 2 * DAY, "Pat <pat@x.co>, Daniel <daniel@northwind.co>", "Morning both.");
        sent(&mut store, other("Pat"), "c@x", NOV_1 - DAY, "Pat <pat@x.co>", "Only to Pat.");
        let reply = mail().subject("Re: Q4 budget").reply_to(&["in@x"]);
        sent(&mut store, reply, "r@x", NOV_1 + 1, "daniel@northwind.co", "In this thread.");

        let words = store.my_words_to("work", " DANIEL@northwind.co ", &here.thread_id, 5).unwrap();
        assert_eq!(words, ["Morning both.", "Hey Daniel! Done."]);
        assert_eq!(store.my_words_to("work", "daniel@northwind.co", &here.thread_id, 1).unwrap(), ["Morning both."]);
        assert!(store.my_words_to("home", "daniel@northwind.co", "", 5).unwrap().is_empty());
        // Without its text on this device, an email is skipped.
        store.conn.execute_batch("DELETE FROM bodies").unwrap();
        assert!(store.my_words_to("work", "daniel@northwind.co", &here.thread_id, 5).unwrap().is_empty());
        break_schema(&store, "ALTER TABLE messages RENAME COLUMN from_me TO gone");
        assert!(store.my_words_to("work", "daniel@northwind.co", "", 5).is_err());
    }
}
