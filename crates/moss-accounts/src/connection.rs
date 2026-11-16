//! What a signed-in account looks like to the rest of Moss.
//!
//! [`AccountConnection`] is provider-neutral: the provider crate turns it into an IMAP,
//! SMTP or Resend connection, reading the secret from the keychain by its [`SecretRef`].

use moss_core::ProviderKind;
use serde::{Deserialize, Serialize};

use crate::secret::SecretRef;

/// The mail services Moss knows by name. Finer than [`ProviderKind`]: iCloud, Yahoo and
/// Fastmail are all IMAP, but each has its own sign-in screen and server settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MailService {
    /// Outlook.com, Hotmail and Microsoft 365.
    Outlook,
    Gmail,
    #[serde(rename = "icloud")]
    ICloud,
    Yahoo,
    Fastmail,
    /// Any other IMAP server.
    Imap,
    Resend,
}

impl MailService {
    /// How Moss talks to this service.
    pub fn kind(self) -> ProviderKind {
        match self {
            MailService::Outlook => ProviderKind::Microsoft,
            MailService::Gmail => ProviderKind::Gmail,
            MailService::Resend => ProviderKind::Resend,
            MailService::ICloud
            | MailService::Yahoo
            | MailService::Fastmail
            | MailService::Imap => ProviderKind::Imap,
        }
    }

    /// Short lowercase name, used in keychain item names.
    pub fn slug(self) -> &'static str {
        match self {
            MailService::Outlook => "outlook",
            MailService::Gmail => "gmail",
            MailService::ICloud => "icloud",
            MailService::Yahoo => "yahoo",
            MailService::Fastmail => "fastmail",
            MailService::Imap => "imap",
            MailService::Resend => "resend",
        }
    }

    /// The service an address most likely belongs to, from its domain alone.
    /// Custom domains come back as [`MailService::Imap`]; discovery may refine that.
    pub fn for_address(email: &str) -> MailService {
        let domain = domain_of(email).unwrap_or_default();
        match domain.as_str() {
            "gmail.com" | "googlemail.com" => MailService::Gmail,
            "outlook.com" | "hotmail.com" | "live.com" | "msn.com" | "passport.com" => {
                MailService::Outlook
            }
            d if d.starts_with("outlook.")
                || d.starts_with("hotmail.")
                || d.starts_with("live.") =>
            {
                MailService::Outlook
            }
            "icloud.com" | "me.com" | "mac.com" => MailService::ICloud,
            "ymail.com" | "rocketmail.com" => MailService::Yahoo,
            d if d == "yahoo.com" || d.starts_with("yahoo.") => MailService::Yahoo,
            "fastmail.com" | "fastmail.fm" => MailService::Fastmail,
            _ => MailService::Imap,
        }
    }

    /// Where the user makes an app password for this service.
    pub fn app_password_url(self) -> Option<&'static str> {
        match self {
            MailService::Gmail => Some("https://myaccount.google.com/apppasswords"),
            MailService::ICloud => Some("https://account.apple.com/account/manage"),
            MailService::Yahoo => Some("https://login.yahoo.com/account/security"),
            MailService::Fastmail => Some("https://app.fastmail.com/settings/security/apps"),
            _ => None,
        }
    }

    /// Built-in server settings, for services that have fixed ones.
    pub fn known_servers(self, email: &str) -> Option<ServerSettings> {
        let (imap, smtp, smtp_security) = match self {
            MailService::Outlook => (
                "outlook.office365.com",
                ("smtp.office365.com", 587),
                Security::StartTls,
            ),
            MailService::Gmail => ("imap.gmail.com", ("smtp.gmail.com", 465), Security::Tls),
            MailService::ICloud => (
                "imap.mail.me.com",
                ("smtp.mail.me.com", 587),
                Security::StartTls,
            ),
            MailService::Yahoo => (
                "imap.mail.yahoo.com",
                ("smtp.mail.yahoo.com", 465),
                Security::Tls,
            ),
            MailService::Fastmail => (
                "imap.fastmail.com",
                ("smtp.fastmail.com", 465),
                Security::Tls,
            ),
            MailService::Imap | MailService::Resend => return None,
        };
        Some(ServerSettings {
            imap: Server::new(imap, 993, Security::Tls, email),
            smtp: Server::new(smtp.0, smtp.1, smtp_security, email),
        })
    }
}

/// How the connection to a server is encrypted. Moss never sends passwords unencrypted,
/// so there is no plain-text option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Security {
    /// TLS from the first byte (IMAP 993, SMTP 465).
    Tls,
    /// Plain connection upgraded with STARTTLS (SMTP 587, IMAP 143).
    StartTls,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub username: String,
}

impl Server {
    pub fn new(host: &str, port: u16, security: Security, username: &str) -> Self {
        Self {
            host: host.into(),
            port,
            security,
            username: username.into(),
        }
    }
}

/// Where to read (IMAP) and send (SMTP) mail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSettings {
    pub imap: Server,
    pub smtp: Server,
}

/// Who issued an account's OAuth tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OAuthIssuer {
    Microsoft,
    Google,
}

/// How Moss proves who it is. The secret itself is in the keychain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AuthMethod {
    /// Password or app password, sent with IMAP `LOGIN` / SMTP `AUTH PLAIN`.
    Password,
    /// OAuth 2.0 access token, sent with SASL `XOAUTH2`.
    #[serde(rename = "oauth2", rename_all = "camelCase")]
    OAuth2 {
        issuer: OAuthIssuer,
        client_id: String,
    },
    /// API key in an `Authorization: Bearer` header (Resend).
    ApiKey,
}

/// A signed-in account, ready for the provider crate to connect.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountConnection {
    /// The address mail is sent from.
    pub email: String,
    pub kind: ProviderKind,
    pub service: MailService,
    /// IMAP and SMTP servers. `None` for Resend, which is an HTTP API.
    pub servers: Option<ServerSettings>,
    pub auth: AuthMethod,
    /// Where the password, tokens or API key are in the keychain.
    pub secret: SecretRef,
}

/// The lowercase domain of an address, or `None` if it isn't a plausible address.
pub fn domain_of(email: &str) -> Option<String> {
    let (local, domain) = email.trim().rsplit_once('@')?;
    let plausible = !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !email.trim().contains(char::is_whitespace);
    plausible.then(|| domain.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_come_from_plausible_addresses_only() {
        assert_eq!(
            domain_of(" Maya@Northwind.CO "),
            Some("northwind.co".into())
        );
        assert_eq!(domain_of("a@b@example.com"), Some("example.com".into()));
        for bad in [
            "",
            "maya",
            "@northwind.co",
            "maya@",
            "maya@localhost",
            "maya@.co",
            "maya@co.",
            "ma ya@x.co",
        ] {
            assert_eq!(domain_of(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn services_are_recognised_from_the_address() {
        let cases = [
            ("a@gmail.com", MailService::Gmail),
            ("a@googlemail.com", MailService::Gmail),
            ("a@outlook.com", MailService::Outlook),
            ("a@hotmail.co.uk", MailService::Outlook),
            ("a@outlook.de", MailService::Outlook),
            ("a@live.fr", MailService::Outlook),
            ("a@msn.com", MailService::Outlook),
            ("a@icloud.com", MailService::ICloud),
            ("a@me.com", MailService::ICloud),
            ("a@yahoo.com", MailService::Yahoo),
            ("a@yahoo.co.jp", MailService::Yahoo),
            ("a@ymail.com", MailService::Yahoo),
            ("a@fastmail.com", MailService::Fastmail),
            ("a@northwind.co", MailService::Imap),
            ("not an address", MailService::Imap),
        ];
        for (email, service) in cases {
            assert_eq!(MailService::for_address(email), service, "{email}");
        }
    }

    #[test]
    fn services_map_to_provider_kinds() {
        assert_eq!(MailService::Outlook.kind(), ProviderKind::Microsoft);
        assert_eq!(MailService::Gmail.kind(), ProviderKind::Gmail);
        assert_eq!(MailService::Resend.kind(), ProviderKind::Resend);
        for imap in [
            MailService::ICloud,
            MailService::Yahoo,
            MailService::Fastmail,
            MailService::Imap,
        ] {
            assert_eq!(imap.kind(), ProviderKind::Imap);
        }
    }

    #[test]
    fn slugs_are_unique() {
        let all = [
            MailService::Outlook,
            MailService::Gmail,
            MailService::ICloud,
            MailService::Yahoo,
            MailService::Fastmail,
            MailService::Imap,
            MailService::Resend,
        ];
        let slugs: std::collections::HashSet<_> = all.iter().map(|s| s.slug()).collect();
        assert_eq!(slugs.len(), all.len());
        assert_eq!(serde_json::to_value(MailService::ICloud).unwrap(), "icloud");
        assert_eq!(
            serde_json::to_value(MailService::Outlook).unwrap(),
            MailService::Outlook.slug()
        );
    }

    #[test]
    fn app_password_services_link_to_their_settings() {
        for service in [
            MailService::Gmail,
            MailService::ICloud,
            MailService::Yahoo,
            MailService::Fastmail,
        ] {
            assert!(service.app_password_url().unwrap().starts_with("https://"));
        }
        assert_eq!(MailService::Outlook.app_password_url(), None);
    }

    #[test]
    fn known_services_have_fixed_servers() {
        let gmail = MailService::Gmail.known_servers("maya@gmail.com").unwrap();
        assert_eq!(
            gmail.imap,
            Server::new("imap.gmail.com", 993, Security::Tls, "maya@gmail.com")
        );
        assert_eq!(
            gmail.smtp,
            Server::new("smtp.gmail.com", 465, Security::Tls, "maya@gmail.com")
        );
        let icloud = MailService::ICloud.known_servers("m@icloud.com").unwrap();
        assert_eq!(icloud.smtp.port, 587);
        assert_eq!(icloud.smtp.security, Security::StartTls);
        let outlook = MailService::Outlook.known_servers("m@outlook.com").unwrap();
        assert_eq!(outlook.imap.host, "outlook.office365.com");
        assert_eq!(outlook.smtp.host, "smtp.office365.com");
        assert_eq!(
            MailService::Yahoo
                .known_servers("m@yahoo.com")
                .unwrap()
                .imap
                .host,
            "imap.mail.yahoo.com"
        );
        assert_eq!(
            MailService::Fastmail
                .known_servers("m@fastmail.com")
                .unwrap()
                .imap
                .host,
            "imap.fastmail.com"
        );
        assert_eq!(MailService::Imap.known_servers("m@x.io"), None);
        assert_eq!(MailService::Resend.known_servers("m@x.io"), None);
    }

    #[test]
    fn connections_serialise_for_the_ui() {
        let connection = AccountConnection {
            email: "maya@outlook.com".into(),
            kind: ProviderKind::Microsoft,
            service: MailService::Outlook,
            servers: MailService::Outlook.known_servers("maya@outlook.com"),
            auth: AuthMethod::OAuth2 {
                issuer: OAuthIssuer::Microsoft,
                client_id: "abc".into(),
            },
            secret: SecretRef::for_account("maya@outlook.com", "outlook"),
        };
        let json = serde_json::to_value(&connection).unwrap();
        assert_eq!(json["kind"], "microsoft");
        assert_eq!(json["auth"]["type"], "oauth2");
        assert_eq!(json["auth"]["clientId"], "abc");
        assert_eq!(json["servers"]["smtp"]["security"], "startTls");
        assert_eq!(json["secret"]["account"], "maya@outlook.com (outlook)");
        assert_eq!(
            serde_json::from_value::<AccountConnection>(json).unwrap(),
            connection
        );
        assert_eq!(
            serde_json::to_value(AuthMethod::ApiKey).unwrap()["type"],
            "apiKey"
        );
    }
}
