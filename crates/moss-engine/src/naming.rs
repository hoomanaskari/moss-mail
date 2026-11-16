//! What a newly connected account is called and coloured, like `makeAccount` in the UI.

use moss_accounts::MailService;
use moss_core::{Account, AccountColor};

const COLORS: [AccountColor; 8] = [
    AccountColor::Work,
    AccountColor::Personal,
    AccountColor::Studio,
    AccountColor::Green,
    AccountColor::Violet,
    AccountColor::Sky,
    AccountColor::Sand,
    AccountColor::Olive,
];

fn capitalise(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A first guess at your name from the address: "maya.ross@…" → "Maya Ross". Sync replaces
/// it with the name on mail you sent.
pub fn name_from_address(email: &str) -> String {
    let local = email.split('@').next().unwrap_or_default();
    let local = local.split('+').next().unwrap_or_default();
    local
        .split(['.', '_', '-'])
        .filter(|w| !w.is_empty())
        .map(capitalise)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Named after the provider ("Gmail"), or the domain for other servers ("Northwind"), in
/// the first colour no account uses yet. A second Gmail becomes "Gmail 2".
pub fn new_account(service: MailService, email: &str, existing: &[Account]) -> Account {
    let email = email.trim().to_string();
    let domain = email.rsplit('@').next().unwrap_or_default();
    let base = match service {
        MailService::Gmail => "Gmail".to_string(),
        MailService::Outlook => "Outlook".into(),
        MailService::ICloud => "iCloud".into(),
        MailService::Yahoo => "Yahoo".into(),
        MailService::Fastmail => "Fastmail".into(),
        MailService::Resend | MailService::Imap => {
            capitalise(domain.split('.').next().unwrap_or_default())
        }
    };
    let mut name = base.clone();
    let mut id = base.to_lowercase();
    for n in 2.. {
        if !existing.iter().any(|a| a.id == id || a.name == name) {
            break;
        }
        name = format!("{base} {n}");
        id = format!("{}-{n}", base.to_lowercase());
    }
    let color = COLORS
        .into_iter()
        .find(|c| !existing.iter().any(|a| a.color == *c))
        .unwrap_or(COLORS[existing.len() % COLORS.len()]);
    Account {
        id,
        letter: name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default(),
        name,
        sender_name: name_from_address(&email),
        address: email,
        color,
        provider: service.kind(),
        org: None,
        org_domain: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moss_core::ProviderKind;

    #[test]
    fn guesses_a_name_from_the_address() {
        assert_eq!(name_from_address("maya.ross@gmail.com"), "Maya Ross");
        assert_eq!(name_from_address("jo_lindqvist+moss@x.io"), "Jo Lindqvist");
        assert_eq!(name_from_address("dan--ok@x.io"), "Dan Ok");
        assert_eq!(name_from_address(""), "");
    }

    #[test]
    fn names_accounts_after_the_provider_or_domain() {
        let gmail = new_account(MailService::Gmail, " maya.ross@gmail.com ", &[]);
        assert_eq!(
            (gmail.id.as_str(), gmail.name.as_str(), gmail.letter.as_str()),
            ("gmail", "Gmail", "G")
        );
        assert_eq!(gmail.address, "maya.ross@gmail.com");
        assert_eq!(gmail.sender_name, "Maya Ross");
        assert_eq!(gmail.color, AccountColor::Work);
        assert_eq!(gmail.provider, ProviderKind::Gmail);
        let other = new_account(MailService::Imap, "maya@northwind.co", &[]);
        assert_eq!((other.id.as_str(), other.name.as_str()), ("northwind", "Northwind"));
        let resend = new_account(MailService::Resend, "hello@rossstudio.art", &[]);
        assert_eq!(resend.name, "Rossstudio");
        for (service, name) in [
            (MailService::Outlook, "Outlook"),
            (MailService::ICloud, "iCloud"),
            (MailService::Yahoo, "Yahoo"),
            (MailService::Fastmail, "Fastmail"),
        ] {
            assert_eq!(new_account(service, "a@b.co", &[]).name, name);
        }
        assert_eq!(new_account(MailService::Imap, "a@", &[]).letter, "");
    }

    #[test]
    fn a_second_account_gets_its_own_id_name_and_colour() {
        let first = new_account(MailService::Gmail, "a@gmail.com", &[]);
        let second = new_account(MailService::Gmail, "b@gmail.com", &[first.clone()]);
        assert_eq!((second.id.as_str(), second.name.as_str()), ("gmail-2", "Gmail 2"));
        assert_eq!(second.color, AccountColor::Personal);
        let third = new_account(MailService::Gmail, "c@gmail.com", &[first.clone(), second]);
        assert_eq!(third.id, "gmail-3");
    }

    #[test]
    fn colours_repeat_once_all_are_used() {
        let mut existing = vec![];
        for (i, color) in COLORS.into_iter().enumerate() {
            let mut a = new_account(MailService::Imap, &format!("a@d{i}.co"), &existing);
            a.color = color;
            existing.push(a);
        }
        let ninth = new_account(MailService::Imap, "a@z.co", &existing);
        assert_eq!(ninth.color, COLORS[0]);
    }
}
