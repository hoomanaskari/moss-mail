//! How Moss signs in to mail accounts and keeps their secrets.
//!
//! The app holds one [`Accounts`]. Each sign-in path ends with the secret in the OS
//! keychain and an [`AccountConnection`] for the provider crate to connect with:
//!
//! - **Outlook / Microsoft 365**: [`Accounts::start_outlook`], open the URL, then
//!   [`Accounts::finish_oauth`]. OAuth 2.0 + PKCE, loopback redirect, no client secret.
//! - **Gmail**: an app password through [`Accounts::sign_in_with_password`], or the
//!   user's own Google OAuth client through [`Accounts::start_gmail_oauth`].
//! - **iCloud, Yahoo, Fastmail**: app password, built-in server settings.
//! - **Other IMAP**: password, settings from [`Accounts::discover`] or typed in.
//! - **Resend**: [`Accounts::check_resend_key`], then [`Accounts::sign_in_resend`].
//!
//! Failures are [`SignInError`]s, one per error screen in the design. For OAuth
//! accounts, [`Accounts::access_token`] returns a working token, refreshing as needed.

mod accounts;
mod connection;
mod discovery;
mod error;
mod file_store;
mod keychain;
mod oauth;
mod resend;
mod secret;
mod send_as;

pub use accounts::{
    classify_login_failure, normalize_gmail_app_password, Accounts, Endpoints, LoginCheck,
    LoginFailure, PasswordSignIn,
};
pub use connection::{
    domain_of, AccountConnection, AuthMethod, MailService, OAuthIssuer, Security, Server,
    ServerSettings,
};
pub use discovery::{
    discover, hosting_service, parse_autoconfig, BoxFuture, Discovered, DiscoveryEndpoints, DiscoverySource, DnsMx,
    MxLookup,
};
pub use error::{GmailAppPasswordProblem, SignInError};
pub use file_store::EncryptedFileStore;
pub use keychain::{os_keychain, platform_secret_store, KeyringSecretStore};
pub use oauth::{
    refresh, OAuthConfig, OAuthEndpoints, PendingAuthorization, SignedIn, GOOGLE_SCOPES,
    GOOGLE_TESTING_TOKEN_LIFETIME, MICROSOFT_CLIENT_ID_PLACEHOLDER, MICROSOFT_SCOPES,
    MOSS_MICROSOFT_CLIENT_ID,
};
pub use resend::{ResendDomain, RESEND_API};
pub use send_as::{SendAs, GMAIL_API};
pub use secret::{
    MemorySecretStore, Secret, SecretRef, SecretStore, SecretStoreError, TokenSet, KEYCHAIN_SERVICE,
};
