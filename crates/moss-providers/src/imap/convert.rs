//! Turning IMAP's wire types into this crate's types, and back.

use async_imap::types::{Fetch, Flag, Name, NameAttribute};
use base64::Engine as _;
use chrono::Utc;

use crate::{
    ConversationMessage, Error, FlagUpdate, Flags, Mailbox, MailboxRole, RawMessage, Result,
};

/// `"<uidvalidity>:<uid>"`.
pub(crate) fn provider_id(uid_validity: u32, uid: u32) -> String {
    format!("{uid_validity}:{uid}")
}

pub(crate) fn parse_provider_id(id: &str) -> Result<(u32, u32)> {
    let parsed = id
        .split_once(':')
        .and_then(|(v, u)| Some((v.parse().ok()?, u.parse().ok()?)));
    match parsed {
        Some((validity, uid)) if uid > 0 => Ok((validity, uid)),
        _ => Err(Error::NotFound(format!("not an IMAP message id: {id:?}"))),
    }
}

/// `1:3,7,9:10`: a sorted, compact UID set.
pub(crate) fn uid_set(uids: &[u32]) -> String {
    let mut sorted = uids.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut ranges: Vec<(u32, u32)> = Vec::new();
    for uid in sorted {
        match ranges.last_mut() {
            Some((_, end)) if *end + 1 == uid => *end = uid,
            _ => ranges.push((uid, uid)),
        }
    }
    let parts: Vec<String> = ranges
        .iter()
        .map(|&(a, b)| {
            if a == b {
                a.to_string()
            } else {
                format!("{a}:{b}")
            }
        })
        .collect();
    parts.join(",")
}

pub(crate) fn flags<'a>(flags: impl Iterator<Item = Flag<'a>>) -> Flags {
    let mut out = Flags::default();
    for flag in flags {
        match flag {
            Flag::Seen => out.seen = true,
            Flag::Answered => out.answered = true,
            Flag::Flagged => out.flagged = true,
            Flag::Deleted => out.deleted = true,
            Flag::Draft => out.draft = true,
            Flag::Custom(keyword) => out.keywords.push(keyword.into_owned()),
            Flag::Recent | Flag::MayCreate => {}
        }
    }
    out
}

fn labels(fetch: &Fetch) -> Vec<String> {
    fetch
        .gmail_labels()
        .map(|l| l.iter().map(|l| l.to_string()).collect())
        .unwrap_or_default()
}

/// A fetched message, or `None` if the response lacks its UID or body (its header block,
/// when `headers_only`).
pub(crate) fn raw_message(
    fetch: &Fetch,
    mailbox: &str,
    uid_validity: u32,
    headers_only: bool,
) -> Option<RawMessage> {
    let raw = if headers_only { fetch.header() } else { fetch.body() }?.to_vec();
    let message = message(fetch, mailbox, uid_validity, raw)?;
    Some(RawMessage { headers_only, ..message })
}

/// A message from its flags and dates in `fetch` and its content fetched separately, or
/// `None` without a UID.
pub(crate) fn message(
    fetch: &Fetch,
    mailbox: &str,
    uid_validity: u32,
    raw: Vec<u8>,
) -> Option<RawMessage> {
    Some(RawMessage {
        provider_id: provider_id(uid_validity, fetch.uid?),
        mailbox: mailbox.to_string(),
        flags: flags(fetch.flags()),
        received_at: fetch.internal_date().map(|d| d.with_timezone(&Utc)),
        raw,
        labels: labels(fetch),
        gmail_msg_id: fetch.gmail_msg_id().copied(),
        headers_only: false,
    })
}

/// A conversation member fetched with its Message-ID header only, or `None` without a UID.
pub(crate) fn conversation_message(
    fetch: &Fetch,
    uid_validity: u32,
) -> Option<ConversationMessage> {
    Some(ConversationMessage {
        provider_id: provider_id(uid_validity, fetch.uid?),
        flags: flags(fetch.flags()),
        labels: labels(fetch),
        headers: fetch.header().unwrap_or_default().to_vec(),
    })
}

pub(crate) fn flag_update(fetch: &Fetch, uid_validity: u32) -> Option<FlagUpdate> {
    Some(FlagUpdate {
        provider_id: provider_id(uid_validity, fetch.uid?),
        flags: flags(fetch.flags()),
        labels: labels(fetch),
    })
}

/// Mailboxes from LIST, with roles from special-use flags, or failing that
/// from well-known top-level names ("Sent Messages", "Deleted Items"…).
pub(crate) fn mailboxes(names: &[Name]) -> Vec<Mailbox> {
    let mut boxes: Vec<Mailbox> = names
        .iter()
        .map(|n| {
            let id = n.name().to_string();
            let attrs = n.attributes();
            let selectable = !attrs.iter().any(|a| match a {
                NameAttribute::NoSelect => true,
                NameAttribute::Extension(e) => e.eq_ignore_ascii_case("\\NonExistent"),
                _ => false,
            });
            let role = if id.eq_ignore_ascii_case("INBOX") {
                Some(MailboxRole::Inbox)
            } else {
                attrs.iter().find_map(special_use)
            };
            let leaf = match n.delimiter() {
                Some(d) => id.rsplit(d).next().unwrap_or(&id),
                None => &id,
            };
            Mailbox {
                name: decode_mutf7(leaf),
                delimiter: n.delimiter().map(str::to_string),
                id,
                role,
                selectable,
            }
        })
        .collect();

    let mut taken: Vec<MailboxRole> = boxes.iter().filter_map(|b| b.role).collect();
    for b in boxes
        .iter_mut()
        .filter(|b| b.role.is_none() && b.selectable)
    {
        let parent = match &b.delimiter {
            Some(d) => b.id.rsplit_once(d.as_str()).map(|(p, _)| p),
            None => None,
        };
        let top_level = parent.is_none_or(|p| p.eq_ignore_ascii_case("INBOX"));
        if let Some(role) = guess_role(&b.name).filter(|r| top_level && !taken.contains(r)) {
            b.role = Some(role);
            taken.push(role);
        }
    }
    boxes
}

fn special_use(attr: &NameAttribute<'_>) -> Option<MailboxRole> {
    Some(match attr {
        NameAttribute::All => MailboxRole::All,
        NameAttribute::Archive => MailboxRole::Archive,
        NameAttribute::Drafts => MailboxRole::Drafts,
        NameAttribute::Flagged => MailboxRole::Flagged,
        NameAttribute::Junk => MailboxRole::Junk,
        NameAttribute::Sent => MailboxRole::Sent,
        NameAttribute::Trash => MailboxRole::Trash,
        NameAttribute::Extension(e) if e.eq_ignore_ascii_case("\\Important") => {
            MailboxRole::Important
        }
        _ => return None,
    })
}

fn guess_role(name: &str) -> Option<MailboxRole> {
    Some(match name.to_lowercase().as_str() {
        "sent" | "sent items" | "sent messages" | "sent mail" => MailboxRole::Sent,
        "drafts" => MailboxRole::Drafts,
        "trash" | "deleted items" | "deleted messages" | "bin" => MailboxRole::Trash,
        "junk" | "spam" | "junk e-mail" | "junk email" | "bulk mail" => MailboxRole::Junk,
        "archive" | "archives" => MailboxRole::Archive,
        _ => return None,
    })
}

/// Decodes IMAP's modified UTF-7 mailbox names (RFC 3501 5.1.3) for display.
pub(crate) fn decode_mutf7(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        let end = rest.find('-').unwrap_or(rest.len());
        let chunk = &rest[..end];
        rest = rest.get(end + 1..).unwrap_or("");
        if chunk.is_empty() {
            out.push('&');
            continue;
        }
        match decode_utf16_chunk(chunk) {
            Some(text) => out.push_str(&text),
            None => {
                out.push('&');
                out.push_str(chunk);
                out.push('-');
            }
        }
    }
    out.push_str(rest);
    out
}

fn decode_utf16_chunk(chunk: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(chunk.replace(',', "/"))
        .ok()?;
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| u16::from_be_bytes(*p))
        .collect();
    String::from_utf16(&units).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::parse_names;

    #[test]
    fn provider_ids_round_trip() {
        assert_eq!(provider_id(7, 42), "7:42");
        assert_eq!(parse_provider_id("7:42").unwrap(), (7, 42));
        for bad in ["42", "a:1", "1:b", "1:0", ""] {
            assert!(
                matches!(parse_provider_id(bad), Err(Error::NotFound(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn uid_sets_are_compact() {
        assert_eq!(uid_set(&[9, 1, 2, 3, 7, 10]), "1:3,7,9:10");
        assert_eq!(uid_set(&[5]), "5");
        assert_eq!(uid_set(&[5, 5]), "5");
    }

    #[test]
    fn flags_split_system_flags_from_keywords() {
        let all = [
            Flag::Seen,
            Flag::Answered,
            Flag::Flagged,
            Flag::Deleted,
            Flag::Draft,
            Flag::Recent,
            Flag::MayCreate,
            Flag::Custom("$Forwarded".into()),
        ];
        assert_eq!(
            flags(all.into_iter()),
            Flags {
                seen: true,
                flagged: true,
                answered: true,
                draft: true,
                deleted: true,
                keywords: vec!["$Forwarded".into()]
            }
        );
        assert_eq!(flags(std::iter::empty()), Flags::default());
    }

    #[test]
    fn decodes_modified_utf7() {
        assert_eq!(decode_mutf7("INBOX"), "INBOX");
        assert_eq!(decode_mutf7("Entw&APw-rfe"), "Entwürfe");
        assert_eq!(decode_mutf7("&ZeVnLIqe-"), "日本語");
        assert_eq!(decode_mutf7("Tom &- Jerry"), "Tom & Jerry");
        assert_eq!(decode_mutf7("&U,BTFw-"), "台北");
        assert_eq!(
            decode_mutf7("bad&!!-x"),
            "bad&!!-x",
            "keeps what it can't decode"
        );
        assert_eq!(
            decode_mutf7("odd&AA-"),
            "odd&AA-",
            "an odd number of bytes isn't UTF-16"
        );
        assert_eq!(decode_mutf7("&2D3eAA-"), "😀");
        assert_eq!(
            decode_mutf7("&2D0-"),
            "&2D0-",
            "a lone surrogate isn't text"
        );
        assert_eq!(
            decode_mutf7("open&APw"),
            "openü",
            "a missing '-' ends at the end"
        );
    }

    #[test]
    fn mailbox_roles_come_from_special_use_flags() {
        let names = parse_names(&[
            r#"* LIST (\HasNoChildren) "/" "INBOX""#,
            r#"* LIST (\HasChildren \Noselect) "/" "[Gmail]""#,
            r#"* LIST (\All \HasNoChildren) "/" "[Gmail]/All Mail""#,
            r#"* LIST (\HasNoChildren \Sent) "/" "[Gmail]/Sent Mail""#,
            r#"* LIST (\HasNoChildren \Trash) "/" "[Gmail]/Bin""#,
            r#"* LIST (\Drafts \HasNoChildren) "/" "[Gmail]/Drafts""#,
            r#"* LIST (\HasNoChildren \Junk) "/" "[Gmail]/Spam""#,
            r#"* LIST (\Flagged \HasNoChildren) "/" "[Gmail]/Starred""#,
            r#"* LIST (\HasNoChildren \Important) "/" "[Gmail]/Important""#,
            r#"* LIST (\Archive) "/" "Archive""#,
            r#"* LIST (\HasNoChildren) "/" "Work/Entw&APw-rfe""#,
            r#"* LIST (\NonExistent) "/" "Gone""#,
        ]);
        let boxes = mailboxes(&names);
        let roles: Vec<(&str, Option<MailboxRole>, bool)> = boxes
            .iter()
            .map(|b| (b.name.as_str(), b.role, b.selectable))
            .collect();
        assert_eq!(
            roles,
            [
                ("INBOX", Some(MailboxRole::Inbox), true),
                ("[Gmail]", None, false),
                ("All Mail", Some(MailboxRole::All), true),
                ("Sent Mail", Some(MailboxRole::Sent), true),
                ("Bin", Some(MailboxRole::Trash), true),
                ("Drafts", Some(MailboxRole::Drafts), true),
                ("Spam", Some(MailboxRole::Junk), true),
                ("Starred", Some(MailboxRole::Flagged), true),
                ("Important", Some(MailboxRole::Important), true),
                ("Archive", Some(MailboxRole::Archive), true),
                ("Entwürfe", None, true),
                ("Gone", None, false),
            ]
        );
        assert_eq!(boxes[2].id, "[Gmail]/All Mail");
        assert_eq!(boxes[2].delimiter.as_deref(), Some("/"));
        assert_eq!(
            boxes[10].id, "Work/Entw&APw-rfe",
            "the id stays as the server spells it"
        );
    }

    #[test]
    fn well_known_names_get_a_role_when_the_server_gives_none() {
        let names = parse_names(&[
            r#"* LIST () "." "INBOX""#,
            r#"* LIST () "." "INBOX.Sent Messages""#,
            r#"* LIST () "." "Deleted Messages""#,
            r#"* LIST () "." "Bulk Mail""#,
            r#"* LIST () "." "Drafts""#,
            r#"* LIST () "." "Archive""#,
            r#"* LIST () "." "Trash""#,
            r#"* LIST () "." "Projects.Archive""#,
            r#"* LIST () NIL "Notes""#,
        ]);
        let roles: Vec<Option<MailboxRole>> = mailboxes(&names).iter().map(|b| b.role).collect();
        assert_eq!(
            roles,
            [
                Some(MailboxRole::Inbox),
                Some(MailboxRole::Sent),
                Some(MailboxRole::Trash),
                Some(MailboxRole::Junk),
                Some(MailboxRole::Drafts),
                Some(MailboxRole::Archive),
                None, // "Deleted Messages" already took Trash
                None, // not top level
                None,
            ]
        );
    }

    #[test]
    fn a_guess_never_overrides_a_special_use_flag() {
        let names = parse_names(&[r#"* LIST () "/" "Trash""#, r#"* LIST (\Trash) "/" "Bin""#]);
        let roles: Vec<Option<MailboxRole>> = mailboxes(&names).iter().map(|b| b.role).collect();
        assert_eq!(roles, [None, Some(MailboxRole::Trash)]);
    }

    #[test]
    fn fetched_messages_carry_gmail_ids_and_labels() {
        use crate::testing::parse_fetch;
        let fetch = parse_fetch(
            "* 1 FETCH (UID 42 FLAGS (\\Seen) INTERNALDATE \"01-Nov-2026 09:30:00 +0100\" \
             X-GM-MSGID 1278455344230334865 X-GM-LABELS (\\Inbox Work) BODY[] {5}\r\nhello)",
        );
        let m = raw_message(&fetch, "INBOX", 7, false).unwrap();
        assert_eq!(m.provider_id, "7:42");
        assert_eq!(m.mailbox, "INBOX");
        assert!(m.flags.seen);
        assert_eq!(
            m.received_at.unwrap().to_rfc3339(),
            "2026-11-01T08:30:00+00:00"
        );
        assert_eq!(m.raw, b"hello");
        assert_eq!(m.gmail_msg_id, Some(1278455344230334865));
        assert_eq!(m.labels, ["\\Inbox", "Work"]);

        let update = flag_update(&fetch, 7).unwrap();
        assert_eq!(
            (
                update.provider_id.as_str(),
                update.flags.seen,
                update.labels.len()
            ),
            ("7:42", true, 2)
        );
    }

    #[test]
    fn fetches_without_a_uid_or_body_are_skipped() {
        use crate::testing::parse_fetch;
        let no_body = parse_fetch("* 1 FETCH (UID 42 FLAGS ())");
        assert!(raw_message(&no_body, "INBOX", 7, false).is_none());
        let plain = flag_update(&no_body, 7).unwrap();
        assert!(plain.labels.is_empty() && plain.flags == Flags::default());
        let no_uid = parse_fetch("* 1 FETCH (FLAGS (\\Seen))");
        assert!(flag_update(&no_uid, 7).is_none());
        let no_date = parse_fetch("* 1 FETCH (UID 1 BODY[] {1}\r\nx)");
        assert_eq!(raw_message(&no_date, "INBOX", 7, false).unwrap().received_at, None);
    }
}
