//! Moss domain model.
//!
//! These types mirror `src/data/types.ts` in the UI. They are the contract between
//! the frontend and the Rust engine (sync, storage, search, AI), so field names are
//! serialised in camelCase to match TypeScript.

use serde::{Deserialize, Serialize};

mod files;
mod mail;
mod signatures;
mod summaries;
pub use files::*;
pub use mail::*;
pub use signatures::*;
pub use summaries::*;

/// Build and platform details, shown in About and attached to bug reports.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub os: String,
    pub arch: String,
}

impl AppInfo {
    pub fn current() -> Self {
        Self {
            name: "Moss".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
        }
    }
}

/// Account colour slot. Each maps to a design token of the same name.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AccountColor {
    Work,
    Personal,
    Studio,
    Green,
    Violet,
    Sky,
    Sand,
    Olive,
}

/// How Moss talks to an account's mail server.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ProviderKind {
    Gmail,
    Microsoft,
    Jmap,
    Imap,
    Resend,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub name: String,
    pub letter: String,
    pub address: String,
    pub sender_name: String,
    pub color: AccountColor,
    pub provider: ProviderKind,
    /// Organisation name, used to flag people outside it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_domain: Option<String>,
}

/// Whether you send from an address of an account, or Moss only thinks you could.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AddressState {
    /// You send from it, and mail from it is yours.
    Added,
    /// Mail came to it, so it may be yours: Moss asks.
    Suggested,
    /// You removed it or said no. Kept so it isn't suggested again.
    Dismissed,
}

/// How Moss learned of an address.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AddressSource {
    /// You added it.
    You,
    /// Mail came to it.
    Mail,
    /// Gmail's "Send mail as" list.
    Gmail,
}

/// Another address one login sends from, such as hi@rossstudio.art on an iCloud account.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountAddress {
    /// Lowercase.
    pub email: String,
    /// The sender name on mail from it. `None` uses the account's sender name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub state: AddressState,
    pub source: AddressSource,
    /// Emails from others that came to it.
    pub seen: u32,
    /// Mail sent to it goes to Blocked, like mail from a blocked sender.
    pub blocked: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum SplitId {
    Important,
    Other,
    Calendar,
    Newsletters,
    Receipts,
}

/// Where a thread sits in the Important split.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub enum Section {
    /// Needs your reply.
    Reply,
    /// For your information.
    Info,
    /// Snoozed; comes back later.
    Later,
}

/// Serialises with `initials` and `short` as well, like the UI's `Person` (see `mail.rs`).
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub name: String,
    pub email: String,
}

/// Someone as the user's mail knows them: how often each way, and when last.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Contact {
    pub person: Person,
    /// Messages from them.
    pub from_them: u32,
    /// Messages the user sent them.
    pub to_them: u32,
    pub last_at: i64,
}

/// Someone you chose from the recipient suggestions, and what you had typed when you did,
/// so the same choice comes first next time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecipientPick {
    pub typed: String,
    pub email: String,
}

impl Person {
    /// Someone outside `org_domain`. With no organisation domain nobody is external.
    pub fn is_external(&self, org_domain: Option<&str>) -> bool {
        match org_domain {
            Some(domain) => !self
                .email
                .to_ascii_lowercase()
                .ends_with(&format!("@{}", domain.to_ascii_lowercase())),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_detection_ignores_case() {
        let ines = Person { name: "Ines Duarte".into(), email: "ines@duarte-studio.pt".into() };
        let lena = Person { name: "Lena Vogt".into(), email: "Lena@Northwind.co".into() };
        assert!(ines.is_external(Some("northwind.co")));
        assert!(!lena.is_external(Some("northwind.co")));
        assert!(!ines.is_external(None));
        assert!(!lena.is_external(Some("Northwind.CO")));
    }

    #[test]
    fn look_alike_domain_is_external() {
        let eve = Person { name: "Eve".into(), email: "eve@notnorthwind.co".into() };
        assert!(eve.is_external(Some("northwind.co")));
    }

    #[test]
    fn serialises_like_the_typescript_types() {
        let json = serde_json::to_string(&Section::Reply).unwrap();
        assert_eq!(json, "\"reply\"");
        let info = serde_json::to_value(AppInfo::current()).unwrap();
        assert!(info.get("version").is_some());
    }

    #[test]
    fn an_address_serialises_like_the_typescript_type() {
        let added = AccountAddress {
            email: "hi@rossstudio.art".into(),
            name: Some("Ross Studio".into()),
            state: AddressState::Added,
            source: AddressSource::You,
            seen: 12,
            blocked: false,
        };
        let json = serde_json::to_value(&added).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "email": "hi@rossstudio.art", "name": "Ross Studio", "state": "added",
                "source": "you", "seen": 12, "blocked": false
            })
        );
        assert_eq!(serde_json::from_value::<AccountAddress>(json).unwrap(), added);
        // No name of its own: mail from it uses the account's sender name.
        let leak = AccountAddress {
            email: "bookshop@rossstudio.art".into(),
            name: None,
            state: AddressState::Dismissed,
            source: AddressSource::Mail,
            seen: 3,
            blocked: true,
        };
        let json = serde_json::to_value(&leak).unwrap();
        assert!(json.get("name").is_none());
        assert_eq!((json["state"].as_str(), json["source"].as_str()), (Some("dismissed"), Some("mail")));
        assert_eq!(serde_json::from_value::<AccountAddress>(json).unwrap(), leak);
        assert_eq!(serde_json::to_value(AddressState::Suggested).unwrap(), "suggested");
        assert_eq!(serde_json::to_value(AddressSource::Gmail).unwrap(), "gmail");
    }

    #[test]
    fn account_uses_camel_case_and_skips_missing_org() {
        let account = Account {
            id: "work".into(),
            name: "Work".into(),
            letter: "W".into(),
            address: "maya@northwind.co".into(),
            sender_name: "Maya Ross".into(),
            color: AccountColor::Work,
            provider: ProviderKind::Imap,
            org: None,
            org_domain: Some("northwind.co".into()),
        };
        let json = serde_json::to_value(&account).unwrap();
        assert_eq!(json["senderName"], "Maya Ross");
        assert_eq!(json["orgDomain"], "northwind.co");
        assert_eq!(json["provider"], "imap");
        assert!(json.get("org").is_none());
        let back: Account = serde_json::from_value(json).unwrap();
        assert_eq!(back, account);
    }
}
