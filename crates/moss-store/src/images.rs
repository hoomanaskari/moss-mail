//! Whether an email's remote images load by themselves. Familiar senders' do; anyone else's
//! wait until you say so. See `docs/research/remote-images.md`.

use moss_core::ImageTrust;
use rusqlite::params;

use crate::classify::is_public_domain;
use crate::senders;
use crate::{Result, Store};

fn choice_key(account_id: &str, email: &str) -> String {
    format!("images:{account_id}:{email}")
}

/// `text` for a LIKE pattern, with its wildcards matching only themselves.
pub(crate) fn literal(text: &str) -> String {
    text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

/// SQL: whether you sent mail in account `?1` to the lowercase address `?2` (To, Cc or Bcc).
const WROTE_TO_ADDRESS: &str = "SELECT EXISTS (SELECT 1 FROM sent_to WHERE account_id = ?1 AND email = ?2)";
/// SQL: whether you sent mail in account `?1` to anyone at the lowercase domain `?2`.
const WROTE_TO_DOMAIN: &str = "SELECT EXISTS (SELECT 1 FROM sent_to WHERE account_id = ?1 AND domain = ?2)";

impl Store {
    /// Why images from `email` load by themselves in this account, or [`ImageTrust::Ask`].
    pub fn image_trust(&self, account_id: &str, email: &str) -> Result<ImageTrust> {
        let email = email.trim().to_lowercase();
        // Accepting a sender or their domain (or unblocking them) also shows their images.
        if self.setting(&choice_key(account_id, &email))?.is_some()
            || senders::is_accepted(&self.conn, account_id, &email)?
        {
            return Ok(ImageTrust::YouAllowed);
        }
        if self.wrote_to(WROTE_TO_ADDRESS, account_id, &email)? {
            return Ok(ImageTrust::YouWrote);
        }
        if let Some((_, domain)) = email.rsplit_once('@') {
            if !is_public_domain(domain) && self.wrote_to(WROTE_TO_DOMAIN, account_id, domain)? {
                return Ok(ImageTrust::SameCompany);
            }
        }
        Ok(ImageTrust::Ask)
    }

    /// Remembers that you want images from `email` shown in this account.
    pub fn allow_images(&mut self, account_id: &str, email: &str) -> Result<()> {
        self.set_setting(&choice_key(account_id, &email.trim().to_lowercase()), Some("show"))
    }

    /// Whether you sent mail in this account to `who`, as `sql` looks for them.
    fn wrote_to(&self, sql: &str, account_id: &str, who: &str) -> Result<bool> {
        Ok(self.conn.prepare_cached(sql)?.query_row(params![account_id, who], |r| r.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::*;
    use moss_core::{Account, Mailbox, MailboxRole, MessageSource, ProviderKind};

    /// The work account with a Sent folder holding one email Maya sent to `to`.
    fn sent_to(to: &str) -> Store {
        let mut store = store_with_work();
        store
            .upsert_mailbox(&Mailbox {
                account_id: "work".into(),
                name: "Sent".into(),
                role: Some(MailboxRole::Sent),
            })
            .unwrap();
        let raw = mail().id("sent@x").from("Maya Ross <maya@northwind.co>").to(to).build();
        store.ingest(&raw, &MessageSource { mailbox: "Sent".into(), ..source("s1") }).unwrap();
        store
    }

    fn trust(store: &Store, email: &str) -> ImageTrust {
        store.image_trust("work", email).unwrap()
    }

    #[test]
    fn finding_who_you_wrote_to_searches_an_index_instead_of_reading_every_email_you_sent() {
        let store = store_with_work();
        for sql in [WROTE_TO_ADDRESS, WROTE_TO_DOMAIN] {
            let plan = plan(&store, sql);
            assert!(plan.contains("USING COVERING INDEX sent_to_by_") && !scans(&plan, "messages"), "{plan}");
        }
    }

    #[test]
    fn a_new_sender_is_asked_about() {
        assert_eq!(trust(&store_with_work(), "alex@studio-build.example.test"), ImageTrust::Ask);
    }

    #[test]
    fn someone_you_have_written_to_is_trusted_whatever_the_case() {
        let store = sent_to("Alex Morgan Studio Build <Alex@STUDIO-BUILD.example.test>");
        assert_eq!(trust(&store, "alex@studio-build.example.test"), ImageTrust::YouWrote);
        assert_eq!(trust(&store, " ALEX@studio-build.example.test "), ImageTrust::YouWrote);
    }

    #[test]
    fn writing_to_someone_at_a_company_trusts_their_colleagues_but_not_a_public_domain() {
        let store =
            sent_to("Alex <alex@studio-build.example.test>, Pat <pat@gmail.com>, Lee <lee@yahoo.co.uk>");
        assert_eq!(trust(&store, "office@studio-build.example.test"), ImageTrust::SameCompany);
        assert_eq!(trust(&store, "stranger@gmail.com"), ImageTrust::Ask);
        assert_eq!(trust(&store, "someone@yahoo.co.uk"), ImageTrust::Ask);
        // A domain that only ends the same way is another company.
        assert_eq!(trust(&store, "x@notstudio-build.example.test"), ImageTrust::Ask);
        let store = sent_to("Bo <bo@mail.bigcorp.com>");
        assert_eq!(trust(&store, "y@bigcorp.com"), ImageTrust::Ask);
    }

    #[test]
    fn only_mail_you_sent_counts_not_mail_sent_to_you() {
        let mut store = store_with_work();
        ingest(&mut store, "1", &mail().from("Alex <alex@studio-build.example.test>").build());
        assert_eq!(trust(&store, "alex@studio-build.example.test"), ImageTrust::Ask);
    }

    #[test]
    fn wildcards_in_an_address_match_only_themselves() {
        let store = sent_to("Ann <ann@acme.co>");
        // Unescaped, `_` would match the dot and `%` anything.
        assert_eq!(trust(&store, "ann@acme_co"), ImageTrust::Ask);
        assert_eq!(trust(&store, "ann@%"), ImageTrust::Ask);
        assert_eq!(trust(&store, "x@acme\\.co"), ImageTrust::Ask);
        assert_eq!(trust(&store, "not-an-address"), ImageTrust::Ask);
        assert_eq!(trust(&store, "nobody@"), ImageTrust::Ask);
    }

    #[test]
    fn showing_images_from_a_sender_is_remembered_per_account() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya@fastmail.com")).unwrap();
        store.allow_images("work", "Alex@studio-build.example.test").unwrap();
        assert_eq!(trust(&store, "alex@studio-build.example.test"), ImageTrust::YouAllowed);
        assert_eq!(
            store.image_trust("personal", "alex@studio-build.example.test").unwrap(),
            ImageTrust::Ask
        );
    }

    #[test]
    fn accepting_or_unblocking_a_sender_shows_their_images_in_that_account() {
        let mut store = store_with_work();
        store.upsert_account(&account("personal", "maya@fastmail.com")).unwrap();
        store.accept_sender("work", "Alex@studio-build.example.test", false, NOV_1).unwrap();
        assert_eq!(trust(&store, "alex@studio-build.example.test"), ImageTrust::YouAllowed);
        assert_eq!(
            store.image_trust("personal", "alex@studio-build.example.test").unwrap(),
            ImageTrust::Ask
        );
        // Accepting one person doesn't vouch for their colleagues; accepting the domain does.
        assert_eq!(trust(&store, "office@studio-build.example.test"), ImageTrust::Ask);
        store.accept_sender("work", "alex@studio-build.example.test", true, NOV_1).unwrap();
        assert_eq!(trust(&store, "office@studio-build.example.test"), ImageTrust::YouAllowed);
        assert_eq!(trust(&store, "x@notstudio-build.example.test"), ImageTrust::Ask);

        store.block_sender("work", "jordan@growthleads.example", false, NOV_1).unwrap();
        assert_eq!(trust(&store, "jordan@growthleads.example"), ImageTrust::Ask);
        store.unblock_sender("work", "jordan@growthleads.example", NOV_1).unwrap();
        assert_eq!(trust(&store, "jordan@growthleads.example"), ImageTrust::YouAllowed);
    }

    #[test]
    fn a_thread_says_which_messages_show_images_by_themselves() {
        let mut store = sent_to("Daniel <daniel@northwind.co>");
        let theirs =
            ingest(&mut store, "2", &mail().id("r@x").from("Ines <ines@vendor.io>").build());
        let thread = store.thread(&theirs.thread_id, NOV_1).unwrap().unwrap();
        assert_eq!(thread.messages[0].image_trust, ImageTrust::Ask);
        store.allow_images("work", "ines@vendor.io").unwrap();
        let thread = store.thread(&theirs.thread_id, NOV_1).unwrap().unwrap();
        assert_eq!(thread.messages[0].image_trust, ImageTrust::YouAllowed);

        let mine = store
            .ingest(&mail().id("m@x").from("Maya <maya@northwind.co>").build(), &source("3"))
            .unwrap();
        let thread = store.thread(&mine.thread_id, NOV_1).unwrap().unwrap();
        assert_eq!(thread.messages[0].image_trust, ImageTrust::Yours);
    }

    /// A Gmail account, where Gmail's `Authentication-Results` count, that wrote to Alex.
    fn gmail_writing_to_alex() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        store.upsert_account(&Account { provider: ProviderKind::Gmail, ..account("work", "maya@northwind.co") }).unwrap();
        for (name, role) in [("Sent", MailboxRole::Sent), ("INBOX", MailboxRole::Inbox)] {
            store.upsert_mailbox(&Mailbox { account_id: "work".into(), name: name.into(), role: Some(role) }).unwrap();
        }
        let raw = mail().id("sent@x").from("Maya Ross <maya@northwind.co>").to("Alex <alex@example.com>").build();
        store.ingest(&raw, &MessageSource { mailbox: "Sent".into(), ..source("s1") }).unwrap();
        store
    }

    /// The message stored from `raw`, as the window gets it.
    fn shown(store: &mut Store, mailbox: &str, provider_id: &str, raw: &[u8]) -> moss_core::Message {
        let at = MessageSource { mailbox: mailbox.into(), ..source(provider_id) };
        let stored = store.ingest(raw, &at).unwrap();
        let thread = store.thread(&stored.thread_id, NOV_1).unwrap().unwrap();
        thread.messages.into_iter().find(|m| m.id == stored.message_id).unwrap()
    }

    const PASS: &str = "mx.google.com; dkim=pass header.i=@example.com header.s=s1; dmarc=pass (p=REJECT) header.from=example.com";

    #[test]
    fn images_from_someone_you_trust_load_only_when_gmail_confirmed_who_sent_it() {
        let mut store = gmail_writing_to_alex();
        let alex = || mail().from("Alex <alex@example.com>").received("Received", "from mail.example.com by mx.google.com");
        let confirmed = shown(&mut store, "INBOX", "1", &alex().id("ok@x").received("Authentication-Results", PASS).build());
        assert_eq!((confirmed.image_trust, confirmed.sender_unverified), (ImageTrust::YouWrote, false));

        // A forger's own header saying pass, under Gmail's saying fail.
        let forged = alex()
            .id("forged@x")
            .header("Authentication-Results", PASS)
            .received("Authentication-Results", "mx.google.com; spf=fail smtp.mailfrom=alex@example.com; dmarc=fail header.from=example.com")
            .build();
        let forged = shown(&mut store, "INBOX", "2", &forged);
        assert_eq!((forged.image_trust, forged.sender_unverified), (ImageTrust::YouWrote, true));
        // Nobody vouching for it at all, or a From line naming two people.
        let bare = shown(&mut store, "INBOX", "3", &alex().id("bare@x").build());
        assert!(bare.sender_unverified);
        let two = alex().id("two@x").from("Alex <alex@example.com>, Eve <eve@evil.example>").received("Authentication-Results", PASS).build();
        assert!(shown(&mut store, "INBOX", "4", &two).sender_unverified);
        // The same headers on an account whose server Moss can't tell apart from a sender.
        let mut other = sent_to("Alex <alex@example.com>");
        let elsewhere = shown(&mut other, "INBOX", "1", &alex().id("ok@x").received("Authentication-Results", PASS).build());
        assert_eq!((elsewhere.image_trust, elsewhere.sender_unverified), (ImageTrust::YouWrote, true));
    }

    #[test]
    fn mail_from_your_own_address_is_yours_only_from_your_sent_folder_or_moss_itself() {
        let mut store = gmail_writing_to_alex();
        let mine = || mail().from("Maya Ross <maya@northwind.co>").to("Alex <alex@example.com>");
        let sent = shown(&mut store, "Sent", "s2", &mine().id("a@x").build());
        assert_eq!((sent.image_trust, sent.sender_unverified), (ImageTrust::Yours, false));
        assert!(!shown(&mut store, "INBOX", "local:<b@x>", &mine().id("b@x").build()).sender_unverified);
        // Anyone can write your address on the From line of mail to you.
        let forged = shown(&mut store, "INBOX", "5", &mine().id("c@x").build());
        assert_eq!((forged.image_trust, forged.sender_unverified), (ImageTrust::Yours, true));
        // Mail you sent yourself turns up in your inbox and in Sent: once it's in Sent, it's yours.
        let in_sent = shown(&mut store, "Sent", "s3", &mine().id("c@x").build());
        assert_eq!((in_sent.id, in_sent.sender_unverified), (forged.id, false));
        // Seen again in the inbox, it stays yours.
        assert!(!shown(&mut store, "INBOX", "6", &mine().id("c@x").build()).sender_unverified);
    }

    #[test]
    fn failures_are_errors() {
        let mut store = store_with_work();
        break_schema(&store, "DROP TABLE settings");
        assert!(store.image_trust("work", "a@b.co").is_err());
        assert!(store.allow_images("work", "a@b.co").is_err());
        let store = store_with_work();
        break_schema(&store, "DROP TABLE sender_rules");
        assert!(store.image_trust("work", "a@b.co").is_err());
        let store = store_with_work();
        break_schema(&store, "ALTER TABLE sent_to RENAME COLUMN email TO gone");
        assert!(store.image_trust("work", "a@b.co").is_err());
        let store = store_with_work();
        break_schema(&store, "ALTER TABLE sent_to RENAME COLUMN domain TO gone");
        assert!(store.image_trust("work", "a@b.co").is_err());
    }
}
