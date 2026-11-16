//! Finding the other addresses an account sends from. Gmail lists them ("Send mail as");
//! for the rest, Moss looks at the addresses mail came to: at the account's own domains,
//! and, for iCloud and Fastmail, at custom domains whose mail goes to that service. Apple
//! has no way to list them.

use std::sync::Arc;

use moss_accounts::{AuthMethod, MailService};

use crate::events::Event;
use crate::{Result, Shared};

/// How often sync looks again, at most.
pub(crate) const EVERY: i64 = 6 * 3_600_000;

/// Domains asked of the DNS per look; the rest wait for the next one.
pub(crate) const LOOKUPS: usize = 20;

impl Shared {
    /// After a sync pass: whether it's time to look for the account's addresses again.
    /// Marks the look as started.
    pub(crate) fn discovery_due(&self, id: &str) -> bool {
        let mut last = crate::lock(&self.discovered);
        if last.get(id).is_some_and(|at| self.now() - at < EVERY) {
            return false;
        }
        last.insert(id.into(), self.now());
        true
    }

    /// Looks in the background; a failure waits for the next look.
    pub(crate) fn discover_in_background(self: &Arc<Self>, id: &str) {
        let shared = self.clone();
        let id = id.to_string();
        tokio::spawn(async move {
            let _ = shared.discover_addresses(&id).await;
        });
    }

    /// Looks for the account's addresses now, and says so with [`Event::AccountUpdated`]
    /// when they changed.
    pub(crate) async fn discover_addresses(&self, id: &str) -> Result<()> {
        let _credentials = self.credential_lifecycle.read().await;
        self.discovered
            .lock()
            .unwrap()
            .insert(id.into(), self.now());
        let before = self.store().every_address(id)?;
        let connection = self.connection(id).ok();
        if let Some(c) = connection.as_ref().filter(|c| {
            c.service == MailService::Gmail && matches!(c.auth, AuthMethod::OAuth2 { .. })
        }) {
            let token = self.accounts.access_token(c).await?;
            for found in self.accounts.gmail_addresses(&token).await? {
                match self
                    .store()
                    .add_gmail_address(id, &found.email, found.name.as_deref())
                {
                    // Gmail's list may hold the account's own address, or something odd.
                    Ok(())
                    | Err(
                        moss_store::Error::PrimaryAddress(_) | moss_store::Error::InvalidAddress(_),
                    ) => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        let service = connection.map(|c| c.service);
        let mut hosted = Vec::new();
        if let Some(service) =
            service.filter(|s| matches!(s, MailService::ICloud | MailService::Fastmail))
        {
            let mut asked = 0;
            let domains = self.store().address_domains_to_check(id)?;
            for domain in domains {
                let known = crate::lock(&self.hosts).get(&domain).copied();
                let host = match known {
                    Some(host) => host,
                    None if asked < LOOKUPS => {
                        asked += 1;
                        let host = self.accounts.domain_service(&domain).await;
                        crate::lock(&self.hosts).insert(domain.clone(), host);
                        host
                    }
                    None => continue,
                };
                if host == Some(service) {
                    hosted.push(domain);
                }
            }
        }
        self.store().scan_addresses(id, &hosted)?;
        if self.store().every_address(id)? != before {
            let account = self.info(self.account(id)?)?;
            self.events.emit(Event::AccountUpdated { account });
        }
        Ok(())
    }
}
