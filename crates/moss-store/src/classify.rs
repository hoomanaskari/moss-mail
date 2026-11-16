//! First guess at where a new message belongs. The user can move threads, and smarter
//! (local AI) sorting can replace these rules later.

use moss_core::{Section, SenderStatus, SplitId};

use crate::parse::Parsed;

const RECEIPT_WORDS: &[&str] = &[
    "receipt",
    "invoice",
    "order confirmation",
    "your order",
    "payment received",
    "payment confirmation",
    "order #",
    "order confirmed",
    "has shipped",
    "your purchase",
];
/// Senders whose mail is always a receipt or bill.
const RECEIPT_SENDERS: &[&str] = &["receipt", "billing", "invoice"];
const ROBOT_SENDERS: &[&str] =
    &["noreply", "donotreply", "notification", "mailerdaemon", "postmaster", "bounce"];

/// Whether an address belongs to a machine, e.g. no-reply@ or notifications@.
pub fn is_robot(email: &str) -> bool {
    let local: String = email
        .split('@')
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| !matches!(c, '-' | '_' | '.'))
        .collect::<String>()
        .to_lowercase();
    ROBOT_SENDERS.iter().any(|r| local.starts_with(r))
}

/// Where a new message goes: its split, and whether it needs your reply. Everything that
/// sorts mail goes through here, so a local AI model can replace these rules in one place.
/// `mine` says whether an address is yours: the account's own, or another of its addresses.
pub fn classify(message: &Parsed, mine: impl Fn(&str) -> bool, from_me: bool) -> (SplitId, Section) {
    let split = split_for(message);
    // Only a person can be waiting on your reply; robots and lists are for your information.
    let section = match split {
        SplitId::Important => section_for(message, mine, from_me),
        _ => Section::Info,
    };
    (split, section)
}

/// Free mail services: one person there says nothing about anyone else there, so their
/// domains are never blocked or trusted as a whole.
const PUBLIC_DOMAINS: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "icloud.com",
    "me.com",
    "mac.com",
    "aol.com",
    "proton.me",
    "protonmail.com",
    "pm.me",
    "fastmail.com",
    "gmx.com",
    "gmx.net",
    "mail.com",
    "zoho.com",
    "yandex.com",
    "hey.com",
    "tutanota.com",
    "tuta.io",
    "qq.com",
    "163.com",
    "msn.com",
];
/// Free mail services with a domain per country, such as yahoo.co.uk or hotmail.fr.
const PUBLIC_PREFIXES: &[&str] =
    &["yahoo.", "ymail.", "hotmail.", "outlook.", "live.", "gmx.", "web."];

/// Whether a domain belongs to a free mail service, like gmail.com or yahoo.co.uk.
pub fn is_public_domain(domain: &str) -> bool {
    let domain = domain.to_lowercase();
    PUBLIC_DOMAINS.contains(&domain.as_str())
        || PUBLIC_PREFIXES.iter().any(|p| domain.starts_with(p))
}

/// The lowercase domain of an address, if it has one.
pub fn domain_of(email: &str) -> Option<String> {
    let (_, domain) = email.trim().rsplit_once('@')?;
    (!domain.is_empty()).then(|| domain.to_lowercase())
}

/// What Moss knows locally about a sender, for [`sender_status`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SenderFacts {
    /// You blocked them, or their whole domain.
    pub blocked: bool,
    /// You chose Accept on one of their emails, or unblocked them.
    pub accepted: bool,
    /// You have sent mail to this address.
    pub you_wrote: bool,
    /// They are in a conversation you started.
    pub in_your_thread: bool,
    /// They are at the account's own company domain.
    pub your_domain: bool,
    /// It's your own address.
    pub is_you: bool,
    /// A hint that you asked for this mail (a receipt, a sign-in code). Reserved for the
    /// decision model: it may skip the question, but never blocks or unblocks anyone.
    pub expected: bool,
}

/// Whether to ask "Block this sender?". Rules only, so the answer is instant and certain.
pub fn sender_status(facts: &SenderFacts) -> SenderStatus {
    let f = facts;
    if f.blocked {
        SenderStatus::Blocked
    } else if f.accepted
        || f.you_wrote
        || f.in_your_thread
        || f.your_domain
        || f.is_you
        || f.expected
    {
        SenderStatus::Known
    } else {
        SenderStatus::New
    }
}

/// Gmail ignores dots and anything after a + in the part before the @.
fn canonical(email: &str) -> Option<String> {
    let (local, domain) = email
        .trim()
        .to_lowercase()
        .rsplit_once('@')
        .map(|(l, d)| (l.to_string(), d.to_string()))?;
    if domain == "gmail.com" || domain == "googlemail.com" {
        let local = local.split('+').next().unwrap_or_default().replace('.', "");
        Some(format!("{local}@gmail.com"))
    } else {
        Some(format!("{local}@{domain}"))
    }
}

/// Whether two addresses reach the same mailbox.
pub fn same_address(a: &str, b: &str) -> bool {
    matches!((canonical(a), canonical(b)), (Some(a), Some(b)) if a == b)
}

pub fn split_for(message: &Parsed) -> SplitId {
    let subject = message.subject.to_lowercase();
    let sender = message.from.email.to_lowercase();
    if message.has_calendar {
        SplitId::Calendar
    } else if RECEIPT_WORDS.iter().any(|w| subject.contains(w))
        || RECEIPT_SENDERS.iter().any(|r| sender.starts_with(r))
    {
        SplitId::Receipts
    } else if message.is_list {
        SplitId::Newsletters
    } else if message.is_automated || is_robot(&message.from.email) {
        SplitId::Other
    } else {
        SplitId::Important
    }
}

/// Needs your reply when it was written to you directly, at any of your addresses; otherwise
/// it's for your information.
pub fn section_for(message: &Parsed, mine: impl Fn(&str) -> bool, from_me: bool) -> Section {
    let to_me = message.to.iter().any(|p| mine(&p.email));
    if to_me && !from_me {
        Section::Reply
    } else {
        Section::Info
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse;
    use crate::test_support::*;

    fn split(mail: Mail) -> SplitId {
        split_for(&parse(&mail.build()).unwrap())
    }

    #[test]
    fn robots_are_recognised_by_their_address() {
        assert!(is_robot("no-reply@codehost.example"));
        assert!(is_robot("No_Reply@x.co"));
        assert!(is_robot("do.not.reply@x.co"));
        assert!(is_robot("notifications@codehost.example"));
        assert!(is_robot("MAILER-DAEMON@x.co"));
        assert!(!is_robot("daniel@northwind.co"));
        assert!(!is_robot(""));
    }

    #[test]
    fn sorts_into_splits() {
        assert_eq!(split(mail()), SplitId::Important);
        assert_eq!(
            split(mail().attach("invite.ics", "text/calendar", "BEGIN:VCALENDAR")),
            SplitId::Calendar
        );
        assert_eq!(split(mail().subject("Your receipt from Framely")), SplitId::Receipts);
        assert_eq!(
            split(mail().subject("Invoice #42").header("List-Id", "<x>")),
            SplitId::Receipts
        );
        assert_eq!(
            split(mail().header("List-Unsubscribe", "<mailto:u@x.co>")),
            SplitId::Newsletters
        );
        assert_eq!(split(mail().header("Auto-Submitted", "auto-replied")), SplitId::Other);
        assert_eq!(split(mail().from("CodeHost <noreply@codehost.example>")), SplitId::Other);
    }

    #[test]
    fn more_receipts_are_recognised() {
        assert_eq!(split(mail().subject("Order #1234 confirmed")), SplitId::Receipts);
        assert_eq!(split(mail().subject("Your package has shipped")), SplitId::Receipts);
        assert_eq!(split(mail().subject("Thanks for your purchase")), SplitId::Receipts);
        assert_eq!(
            split(mail().subject("Hello").from("Shop <billing@shop.io>")),
            SplitId::Receipts
        );
        assert_eq!(split(mail().subject("Hello").from("receipts@shop.io")), SplitId::Receipts);
        assert_eq!(split(mail().subject("Ordering lunch?")), SplitId::Important);
    }

    #[test]
    fn one_function_sorts_mail_into_a_split_and_section() {
        let direct = parse(&mail().build()).unwrap();
        assert_eq!(
            classify(&direct, |e| same_address(e, "maya@northwind.co"), false),
            (SplitId::Important, Section::Reply)
        );
        let list = parse(&mail().header("List-Id", "<x>").build()).unwrap();
        assert_eq!(
            classify(&list, |e| same_address(e, "maya@northwind.co"), false),
            (SplitId::Newsletters, Section::Info)
        );
    }

    #[test]
    fn gmail_addresses_match_without_dots_or_plus_tags() {
        assert!(same_address("Maya.Ross@gmail.com", "mayaross@gmail.com"));
        assert!(same_address("mayaross+news@googlemail.com", "maya.ross@gmail.com"));
        assert!(!same_address("maya.ross@northwind.co", "mayaross@northwind.co"));
        assert!(same_address("MAYA@northwind.co", "maya@northwind.co"));
        assert!(!same_address("maya", "maya@gmail.com"));
        let dotted = parse(&mail().to("Maya <m.a.y.a@gmail.com>").build()).unwrap();
        assert_eq!(section_for(&dotted, |e| same_address(e, "maya@gmail.com"), false), Section::Reply);
    }

    fn facts() -> SenderFacts {
        SenderFacts::default()
    }

    #[test]
    fn someone_you_never_wrote_to_is_a_new_sender() {
        assert_eq!(sender_status(&facts()), SenderStatus::New);
    }

    #[test]
    fn writing_to_them_accepting_them_or_sharing_a_company_makes_a_sender_known() {
        for known in [
            SenderFacts { you_wrote: true, ..facts() },
            SenderFacts { in_your_thread: true, ..facts() },
            SenderFacts { accepted: true, ..facts() },
            SenderFacts { your_domain: true, ..facts() },
            SenderFacts { is_you: true, ..facts() },
        ] {
            assert_eq!(sender_status(&known), SenderStatus::Known, "{known:?}");
        }
    }

    #[test]
    fn blocking_wins_over_everything() {
        let blocked = SenderFacts { blocked: true, you_wrote: true, accepted: true, ..facts() };
        assert_eq!(sender_status(&blocked), SenderStatus::Blocked);
    }

    #[test]
    fn mail_you_asked_for_skips_the_question_but_never_unblocks() {
        // The hint from the decision model: a receipt or sign-in code you expected.
        assert_eq!(sender_status(&SenderFacts { expected: true, ..facts() }), SenderStatus::Known);
        let blocked = SenderFacts { expected: true, blocked: true, ..facts() };
        assert_eq!(sender_status(&blocked), SenderStatus::Blocked);
    }

    #[test]
    fn free_mail_domains_are_public() {
        for public in [
            "gmail.com",
            "GMAIL.com",
            "icloud.com",
            "yahoo.co.uk",
            "hotmail.fr",
            "outlook.com",
            "proton.me",
        ] {
            assert!(is_public_domain(public), "{public}");
        }
        for company in ["growthleads.example", "northwind.co", "notgmail.com", "mail.bigcorp.com", ""] {
            assert!(!is_public_domain(company), "{company}");
        }
    }

    #[test]
    fn the_domain_of_an_address_is_lowercase_and_after_the_last_at() {
        assert_eq!(domain_of("Jordan@GrowthLeads.example").as_deref(), Some("growthleads.example"));
        assert_eq!(domain_of(" a@b@c.co ").as_deref(), Some("c.co"));
        assert_eq!(domain_of("nobody"), None);
        assert_eq!(domain_of("x@"), None);
    }

    #[test]
    fn direct_mail_needs_a_reply() {
        let direct = parse(&mail().build()).unwrap();
        assert_eq!(section_for(&direct, |e| same_address(e, "Maya@northwind.co"), false), Section::Reply);
        assert_eq!(section_for(&direct, |e| same_address(e, "maya@northwind.co"), true), Section::Info);
        let cc = parse(&mail().to("lena@northwind.co").header("Cc", "maya@northwind.co").build())
            .unwrap();
        assert_eq!(section_for(&cc, |e| same_address(e, "maya@northwind.co"), false), Section::Info);
    }
}
