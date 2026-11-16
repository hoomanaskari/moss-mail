//! Who you hear from and write to, counted from headers as mail arrives: how often each
//! way, and when last. The AI and search use it to know who matters.

use moss_core::Person;
use rusqlite::{params, OptionalExtension, Transaction};

use crate::classify::is_robot;
use crate::parse::Parsed;
use crate::{Result, Store};

pub use moss_core::{Contact, RecipientPick};

/// How many recipient picks are kept; older ones go.
pub(crate) const PICKS: usize = 500;

pub(crate) fn bump(tx: &Transaction, account: &str, p: &Person, from: u32, to: u32, at: i64) -> Result<()> {
    if p.email.is_empty() {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO people (account_id, email, name, from_them, to_them, last_at) VALUES (?1, lower(?2), ?3, ?4, ?5, ?6)
         ON CONFLICT (account_id, email) DO UPDATE SET
             name = CASE WHEN excluded.name <> excluded.email THEN excluded.name ELSE name END,
             from_them = from_them + excluded.from_them, to_them = to_them + excluded.to_them,
             last_at = max(last_at, excluded.last_at)",
        params![account, p.email, p.name, from, to, at],
    )?;
    Ok(())
}

/// Counts a new message: one from them, or one you sent to everyone on it.
pub(crate) fn count(tx: &Transaction, account: &str, parsed: &Parsed, from_me: bool, at: i64) -> Result<()> {
    if !from_me {
        return bump(tx, account, &parsed.from, 1, 0, at);
    }
    for p in parsed.to.iter().chain(&parsed.cc).chain(&parsed.bcc) {
        bump(tx, account, p, 0, 1, at)?;
    }
    Ok(())
}

impl Store {
    /// People to suggest as recipients, across accounts: the ones you write to most first,
    /// then the ones who write to you. Leaves out your own addresses and machines.
    pub fn contacts(&self, limit: usize) -> Result<Vec<Contact>> {
        let mut stmt = self.conn.prepare(
            "SELECT email, max(CASE WHEN name <> email THEN name END), sum(to_them) AS t, sum(from_them) AS f, max(last_at) AS l
             FROM people WHERE email NOT IN (SELECT lower(address) FROM accounts)
             GROUP BY email ORDER BY t DESC, f DESC, l DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            let email: String = r.get(0)?;
            let name = r.get::<_, Option<String>>(1)?.unwrap_or_else(|| email.clone());
            Ok(Contact { person: Person { name, email }, to_them: r.get(2)?, from_them: r.get(3)?, last_at: r.get(4)? })
        })?;
        let mut people = Vec::new();
        for p in rows {
            let p = p?;
            if is_robot(&p.person.email) {
                continue;
            }
            people.push(p);
            if people.len() == limit {
                break;
            }
        }
        Ok(people)
    }

    /// The account you last wrote `email` from (to or cc), with your address you wrote from;
    /// if you never wrote them, the account their latest email came to, with no address.
    pub fn writes_from(&self, email: &str) -> Result<Option<(String, Option<String>)>> {
        Ok(self
            .conn
            .query_row(
                "SELECT account_id, CASE WHEN from_me THEN lower(from_email) END FROM messages m
                 WHERE (from_me AND EXISTS (
                     SELECT 1 FROM json_each(m.to_json) j WHERE lower(json_extract(j.value, '$.email')) = lower(?1)
                     UNION ALL SELECT 1 FROM json_each(m.cc_json) j WHERE lower(json_extract(j.value, '$.email')) = lower(?1)))
                    OR (NOT from_me AND lower(from_email) = lower(?1))
                 ORDER BY from_me DESC, sent_at DESC LIMIT 1",
                [email],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    /// Remembers that you chose `email` from the suggestions after typing `typed`.
    pub fn pick_recipient(&mut self, typed: &str, email: &str, at: i64) -> Result<()> {
        let typed = typed.trim().to_lowercase();
        if typed.is_empty() {
            return Ok(());
        }
        let tx = self.conn.transaction()?;
        // Taken out and put back, so a pick made in the same instant as another still counts as newer.
        tx.execute("DELETE FROM recipient_picks WHERE typed = ?1 AND email = lower(?2)", params![typed, email])?;
        tx.execute("INSERT INTO recipient_picks (typed, email, at) VALUES (?1, lower(?2), ?3)", params![typed, email, at])?;
        tx.execute(
            "DELETE FROM recipient_picks WHERE rowid NOT IN (SELECT rowid FROM recipient_picks ORDER BY at DESC, rowid DESC LIMIT ?1)",
            [PICKS as i64],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Who you chose from the suggestions for what you typed, newest first.
    pub fn recipient_picks(&self) -> Result<Vec<RecipientPick>> {
        let mut stmt = self.conn.prepare("SELECT typed, email FROM recipient_picks ORDER BY at DESC, rowid DESC")?;
        let picks = stmt.query_map([], |r| Ok(RecipientPick { typed: r.get(0)?, email: r.get(1)? }))?;
        Ok(picks.collect::<rusqlite::Result<_>>()?)
    }

    /// What an account knows of someone, by address.
    #[cfg(test)]
    pub fn contact(&self, account: &str, email: &str) -> Result<Option<Contact>> {
        Ok(self
            .conn
            .query_row(
                "SELECT email, name, from_them, to_them, last_at FROM people WHERE account_id = ?1 AND email = lower(?2)",
                params![account, email],
                |r| {
                    Ok(Contact {
                        person: Person { email: r.get(0)?, name: r.get(1)? },
                        from_them: r.get(2)?,
                        to_them: r.get(3)?,
                        last_at: r.get(4)?,
                    })
                },
            )
            .optional()?)
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::*;
    use crate::MessageSource;
    use moss_core::{Contact, Person, RecipientPick};

    #[test]
    fn counts_mail_from_people_and_mail_you_send_them() {
        let mut store = store_with_work();
        ingest(&mut store, "1:1", &mail().id("a@x").at(NOV_1).build());
        ingest(&mut store, "1:2", &mail().id("b@x").at(NOV_1 + DAY).from("daniel@northwind.co").build());
        let mine = mail()
            .id("c@x")
            .at(NOV_1 - DAY)
            .from("Maya Ross <maya@northwind.co>")
            .to("Daniel Okafor <Daniel@northwind.co>, Priya <priya@acme.co>")
            .build();
        store.ingest(&mine, &MessageSource { mailbox: "Sent".into(), ..source("2:1") }).unwrap();
        let daniel = store.contact("work", "DANIEL@northwind.co").unwrap().unwrap();
        assert_eq!((daniel.from_them, daniel.to_them, daniel.last_at), (2, 1, NOV_1 + DAY));
        // A name beats a bare address.
        assert_eq!(daniel.person.name, "Daniel Okafor");
        let priya = store.contact("work", "priya@acme.co").unwrap().unwrap();
        assert_eq!((priya.from_them, priya.to_them), (0, 1));
        assert_eq!(store.contact("work", "nobody@x").unwrap(), None);
        assert_eq!(store.contact("home", "daniel@northwind.co").unwrap(), None);
    }

    #[test]
    fn suggests_people_you_write_to_first_and_leaves_out_you_and_machines() {
        let mut store = store_with_work();
        store.upsert_account(&account("home", "maya@home.me")).unwrap();
        // Lena writes often; you never write back.
        ingest(&mut store, "1:1", &mail().id("l1@x").at(NOV_1).from("Lena <lena@x.io>").build());
        ingest(&mut store, "1:2", &mail().id("l2@x").at(NOV_1 + DAY).from("Lena <lena@x.io>").build());
        ingest(&mut store, "1:3", &mail().id("gh@x").from("CodeHost <noreply@codehost.example>").build());
        // Known only by address.
        ingest(&mut store, "1:4", &mail().id("t1@x").from("tom@x.io").build());
        let sent = |id: &str, to: &str| mail().id(id).from("Maya Ross <maya@northwind.co>").to(to).build();
        let to_sent = |id: &str| MessageSource { mailbox: "Sent".into(), ..source(id) };
        store.ingest(&sent("s1@x", "Priya <priya@acme.co>, Maya <maya@home.me>"), &to_sent("2:1")).unwrap();
        store.ingest(&sent("s2@x", "priya@acme.co, Daniel Okafor <daniel@northwind.co>"), &to_sent("2:2")).unwrap();
        // Someone known to two accounts is suggested once.
        let home = MessageSource { account_id: "home".into(), ..source("3:1") };
        store.ingest(&mail().id("h1@x").from("Priya <priya@acme.co>").to("maya@home.me").build(), &home).unwrap();

        let emails: Vec<_> = store.contacts(10).unwrap().into_iter().map(|c| c.person.email).collect();
        assert_eq!(emails, ["priya@acme.co", "daniel@northwind.co", "lena@x.io", "tom@x.io"]);
        assert_eq!(store.contacts(10).unwrap()[3].person.name, "tom@x.io");
        // How often each way, and when last, summed across accounts.
        let priya = Contact { person: Person { name: "Priya".into(), email: "priya@acme.co".into() }, from_them: 1, to_them: 2, last_at: NOV_1 };
        assert_eq!(store.contacts(1).unwrap(), [priya]);
    }

    #[test]
    fn knows_the_account_and_address_you_write_someone_from() {
        let mut store = store_with_work();
        store.upsert_account(&account("home", "maya@home.me")).unwrap();
        let sent = |id: &str, from: &str, to: &str, at: i64| mail().id(id).at(at).from(from).to(to).build();
        let sent_from = |account: &str, id: &str| MessageSource { account_id: account.into(), ..source(id) };
        // Mail from an address you added is yours.
        store.add_address("work", "hello@northwind.co", None).unwrap();
        // Only heard from: the account their mail came to, no address of yours.
        let home = MessageSource { account_id: "home".into(), ..source("1:1") };
        store.ingest(&mail().id("r1@x").at(NOV_1).from("Simon Lark <ME@simon.example>").to("maya@home.me").build(), &home).unwrap();
        assert_eq!(store.writes_from("me@simon.example").unwrap(), Some(("home".into(), None)));
        // Once you write them, where you last wrote from, though they wrote to home later. Cc counts.
        store.ingest(&sent("s1@x", "Maya <hello@northwind.co>", "Ana <ana@x.io>", NOV_1 - DAY), &sent_from("work", "2:1")).unwrap();
        let cc = mail().id("s2@x").at(NOV_1 - 2 * DAY).from("maya@northwind.co").to("ana@x.io").header("Cc", "Simon <me@simon.example>").build();
        store.ingest(&cc, &sent_from("work", "2:2")).unwrap();
        assert_eq!(store.writes_from("Me@Simon.example").unwrap(), Some(("work".into(), Some("maya@northwind.co".into()))));
        assert_eq!(store.writes_from("ana@x.io").unwrap(), Some(("work".into(), Some("hello@northwind.co".into()))), "the newest");
        assert_eq!(store.writes_from("nobody@x.io").unwrap(), None);
    }

    #[test]
    fn a_broken_people_table_is_an_error() {
        let store = store_with_work();
        store.conn.execute("INSERT INTO people (account_id, email, name, last_at) VALUES ('work', x'07', 'x', 0)", []).unwrap();
        assert!(store.contacts(10).is_err(), "an address that isn't text");
        store.conn.execute("DROP TABLE people", []).unwrap();
        assert!(store.contacts(10).is_err());
    }

    #[test]
    fn mail_without_a_sender_counts_no_one() {
        let mut store = store_with_work();
        ingest(&mut store, "1:1", &mail().without("From").build());
        let n: i64 = store.conn.query_row("SELECT count(*) FROM people", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn remembers_who_you_picked_for_what_you_typed_newest_first() {
        let mut store = store_with_work();
        assert_eq!(store.recipient_picks().unwrap(), []);
        store.pick_recipient(" Ross ", "RossMaya@home.example", NOV_1).unwrap();
        store.pick_recipient("ro", "kindle@x.io", NOV_1 + 1).unwrap();
        // Picking the same person for the same text again only makes it newer.
        store.pick_recipient("ross", "rossmaya@home.example", NOV_1 + 2).unwrap();
        // Nothing typed is no pick.
        store.pick_recipient("  ", "tom@x.io", NOV_1 + 3).unwrap();
        let pick = |typed: &str, email: &str| RecipientPick { typed: typed.into(), email: email.into() };
        assert_eq!(store.recipient_picks().unwrap(), [pick("ross", "rossmaya@home.example"), pick("ro", "kindle@x.io")]);
    }

    #[test]
    fn keeps_only_the_latest_picks() {
        let mut store = store_with_work();
        for i in 0..super::PICKS as i64 + 2 {
            store.pick_recipient(&format!("p{i}"), "tom@x.io", NOV_1 + i).unwrap();
        }
        let picks = store.recipient_picks().unwrap();
        assert_eq!(picks.len(), super::PICKS);
        assert_eq!(picks[0].typed, format!("p{}", super::PICKS + 1));
        assert_eq!(picks.last().unwrap().typed, "p2");
    }
}
