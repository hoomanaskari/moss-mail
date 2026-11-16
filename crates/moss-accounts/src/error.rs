//! Why a sign-in failed, in terms the UI can explain.
//!
//! Each variant maps to a screen or message in the Add account flow in `design.pen`.
//! Serialised with a `kind` tag so the UI can switch on it. Errors never contain a
//! password, token or key. External response text must not be copied into details.

use serde::Serialize;

use crate::connection::MailService;
use crate::secret::SecretStoreError;

/// What went wrong with a Gmail app password.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum GmailAppPasswordProblem {
    /// What was typed isn't an app password (those are 16 letters).
    NotAnAppPassword,
    /// Gmail refused the normal account password and asked for an app password.
    AppPasswordRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum SignInError {
    #[error("that doesn't look like an email address")]
    InvalidEmail,

    /// Error screen: wrong password.
    #[error("the server didn't accept the email or password")]
    WrongPassword { service: MailService },

    /// Error screen: Gmail app password.
    #[error("Gmail needs an app password")]
    GmailAppPassword { problem: GmailAppPasswordProblem },

    /// An address on a custom domain in iCloud: iCloud only lets the account's own
    /// @icloud.com address log in, so Moss needs that too.
    #[error("iCloud custom domains sign in with the account's @icloud.com address")]
    ICloudAddressNeeded,

    /// Outlook (and Resend) don't take a mail password; use the browser sign-in or API key.
    #[error("{service:?} doesn't sign in with a password")]
    PasswordNotSupported { service: MailService },

    /// Error screen: IMAP settings not found. The user can enter them by hand.
    #[error("couldn't find the mail server settings for {domain}")]
    SettingsNotFound { domain: String },

    /// Error screen: Outlook needs admin approval. Also used when a Google Workspace
    /// admin has blocked the OAuth client.
    #[error("an administrator has to approve Moss for this account")]
    AdminApprovalRequired {
        service: MailService,
        detail: String,
    },

    /// The user cancelled in the browser, or the provider refused access.
    #[error("the sign-in was cancelled or refused")]
    AccessDenied { detail: String },

    /// A Google OAuth client in "Testing" status: Google drops its refresh tokens after
    /// 7 days. Publishing the client ("In production") fixes it.
    #[error("Google ended this sign-in after 7 days because the OAuth app is in Testing")]
    GoogleTestingExpired,

    /// The refresh token was revoked or expired. Signing in again fixes it.
    #[error("this account needs to sign in again")]
    SignInExpired,

    /// The OAuth client ID, secret or redirect URI was rejected.
    #[error("the OAuth client was rejected: {detail}")]
    InvalidOAuthClient { detail: String },

    /// This build has no Microsoft client ID, so the user has to bring their own.
    #[error("this build of Moss has no Microsoft client ID; use your own")]
    ClientIdNotConfigured,

    #[error("Resend didn't accept that API key")]
    InvalidResendKey,

    /// A "Sending access" key. Reading inbound mail needs "Full access".
    #[error("this Resend key can only send; Moss needs a Full access key")]
    ResendKeyRestricted,

    #[error("{domain} isn't a domain in this Resend account")]
    ResendDomainMissing { domain: String },

    #[error("the browser sign-in didn't finish in time")]
    Timeout,

    #[error("couldn't reach {host}: {detail}")]
    Unreachable { host: String, detail: String },

    #[error("{host} sent a response Moss doesn't understand: {detail}")]
    UnexpectedResponse { host: String, detail: String },

    #[error("couldn't use the keychain: {detail}")]
    Keychain { detail: String },
}

impl From<SecretStoreError> for SignInError {
    fn from(e: SecretStoreError) -> Self {
        SignInError::Keychain {
            detail: e.to_string(),
        }
    }
}

/// Keeps the destination host while discarding transport diagnostics and URLs.
pub(crate) fn unreachable(host: &str, _: reqwest::Error) -> SignInError {
    SignInError::Unreachable {
        host: host.into(),
        detail: "The connection could not be completed.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_tagged_for_the_ui() {
        let json = serde_json::to_value(SignInError::GmailAppPassword {
            problem: GmailAppPasswordProblem::AppPasswordRequired,
        })
        .unwrap();
        assert_eq!(json["kind"], "gmailAppPassword");
        assert_eq!(json["problem"], "appPasswordRequired");
        let json = serde_json::to_value(SignInError::SettingsNotFound {
            domain: "x.io".into(),
        })
        .unwrap();
        assert_eq!(json["kind"], "settingsNotFound");
        assert_eq!(json["domain"], "x.io");
        assert_eq!(
            serde_json::to_value(SignInError::Timeout).unwrap()["kind"],
            "timeout"
        );
        assert_eq!(
            serde_json::to_value(SignInError::ICloudAddressNeeded).unwrap()["kind"],
            "iCloudAddressNeeded"
        );
    }

    #[test]
    fn keychain_errors_convert() {
        let err: SignInError = SecretStoreError::Unavailable("locked".into()).into();
        assert_eq!(
            err.to_string(),
            "couldn't use the keychain: the keychain is not available: locked"
        );
    }

    #[tokio::test]
    async fn network_errors_drop_the_url() {
        let e = reqwest::get("http://127.0.0.1:1/?emailaddress=maya@northwind.co")
            .await
            .unwrap_err();
        let err = unreachable("127.0.0.1", e);
        assert!(!err.to_string().contains("maya"), "{err}");
        assert!(matches!(err, SignInError::Unreachable { detail, .. } if detail == "The connection could not be completed."));
    }
}
