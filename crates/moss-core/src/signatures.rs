//! Signatures and which one each account (or address) adds. Mirrors `src/state/signatures.ts`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Signature {
    pub id: String,
    pub name: String,
    /// Plain text, one entry per line. Web addresses become links.
    pub lines: Vec<String>,
}

/// Which signature an account adds; `None` means none.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountSignatures {
    pub new_emails: Option<String>,
    pub replies: Option<String>,
}

/// Which kind of email an account's choice is for.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SignatureUse {
    NewEmails,
    Replies,
}

/// Every signature and how Moss picks one: the UI's signatures state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignatureSettings {
    pub signatures: Vec<Signature>,
    /// Keyed by account id; accounts that never chose are missing.
    pub by_account: BTreeMap<String, AccountSignatures>,
    /// Keyed by address, for addresses with a choice of their own; the rest use their
    /// account's.
    pub by_address: BTreeMap<String, AccountSignatures>,
    /// Moss decides per email whether to add one, starting from each account's choice.
    pub let_moss_choose: bool,
    /// Every signature id, most recently used first; ones never used follow in list order.
    pub recent: Vec<String>,
}

/// What an email is, for picking its signature.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EmailKind {
    New,
    Reply,
    Forward,
}

/// The email a signature is picked for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignatureEmail {
    pub account_id: String,
    /// The From address, when it isn't the account's own.
    #[serde(default)]
    pub address: Option<String>,
    pub kind: EmailKind,
    /// How many messages in the thread you already sent: a back-and-forth once there are some.
    pub signed_before: u32,
}

/// Why an email got the signature it got. The UI says it in words.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum PickWhy {
    /// The address's choice for new emails.
    New,
    /// The address's choice for replies.
    Reply,
    /// A forward goes to someone new, so it gets the one for new emails.
    Forward,
    /// You signed earlier in the thread: a one-line one.
    Short,
    /// You signed earlier in the thread and there's no one-line one: none.
    Quick,
    /// The address has none for this kind of email.
    NoneSet,
}

/// The signature an email gets (`None`: none) and why.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SignaturePick {
    pub id: Option<String>,
    pub why: PickWhy,
}

/// Picks the signature for an email. Mirrors `pickSignature` in `src/state/signatures.ts`.
///
/// Starts from the From address's choice (or its account's) for new emails or replies. With
/// "Let Moss choose" on, a forward gets the one for new emails, and once you've signed earlier
/// in the thread a reply gets a one-line one or none.
pub fn pick_signature(settings: &SignatureSettings, email: &SignatureEmail) -> SignaturePick {
    let address = email.address.as_deref().map(str::to_lowercase);
    let choice = address
        .and_then(|a| settings.by_address.get(&a))
        .or_else(|| settings.by_account.get(&email.account_id))
        .cloned()
        .unwrap_or_default();
    let auto = settings.let_moss_choose;
    let find = |id: &Option<String>| settings.signatures.iter().find(|s| Some(&s.id) == id.as_ref());
    let (slot, why) = match email.kind {
        EmailKind::New => (&choice.new_emails, PickWhy::New),
        EmailKind::Forward if auto => (&choice.new_emails, PickWhy::Forward),
        _ => (&choice.replies, PickWhy::Reply),
    };
    let pick = |s: &Signature, why| SignaturePick { id: Some(s.id.clone()), why };
    let Some(sig) = find(slot) else {
        return SignaturePick { id: None, why: PickWhy::NoneSet };
    };
    if !(auto && email.kind == EmailKind::Reply && email.signed_before > 0) {
        return pick(sig, why);
    }
    match [Some(sig), find(&choice.new_emails)].into_iter().flatten().find(|s| one_line(s)) {
        Some(short) => pick(short, PickWhy::Short),
        None => SignaturePick { id: None, why: PickWhy::Quick },
    }
}

/// Just your name, say: blank lines don't count.
fn one_line(sig: &Signature) -> bool {
    sig.lines.iter().filter(|l| !l.trim().is_empty()).count() <= 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serialises_like_the_typescript_types() {
        let settings = SignatureSettings {
            signatures: vec![Signature {
                id: "s".into(),
                name: "Work".into(),
                lines: vec!["Maya".into()],
            }],
            by_account: BTreeMap::from([(
                "work".into(),
                AccountSignatures { new_emails: Some("s".into()), replies: None },
            )]),
            by_address: BTreeMap::from([(
                "hi@rossstudio.art".into(),
                AccountSignatures { new_emails: None, replies: Some("s".into()) },
            )]),
            let_moss_choose: true,
            recent: vec!["s".into()],
        };
        assert_eq!(
            serde_json::to_value(&settings).unwrap(),
            json!({
                "signatures": [{"id": "s", "name": "Work", "lines": ["Maya"]}],
                "byAccount": {"work": {"newEmails": "s", "replies": null}},
                "byAddress": {"hi@rossstudio.art": {"newEmails": null, "replies": "s"}},
                "letMossChoose": true,
                "recent": ["s"],
            })
        );
        assert_eq!(serde_json::to_value(SignatureUse::NewEmails).unwrap(), json!("newEmails"));
        assert_eq!(
            serde_json::from_value::<SignatureUse>(json!("replies")).unwrap(),
            SignatureUse::Replies
        );
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Case {
        name: String,
        let_moss_choose: Option<bool>,
        email: SignatureEmail,
        pick: SignaturePick,
    }

    #[derive(Deserialize)]
    struct Table {
        settings: SignatureSettings,
        cases: Vec<Case>,
    }

    /// The same table `pickSignature` in the UI is tested with.
    #[test]
    fn picks_the_signature_like_the_ui() {
        let table: Table = serde_json::from_str(include_str!("../tests/signature-picks.json")).unwrap();
        assert!(!table.cases.is_empty());
        for case in table.cases {
            let mut settings = table.settings.clone();
            settings.let_moss_choose = case.let_moss_choose.unwrap_or(settings.let_moss_choose);
            assert_eq!(pick_signature(&settings, &case.email), case.pick, "{}", case.name);
        }
    }
}
