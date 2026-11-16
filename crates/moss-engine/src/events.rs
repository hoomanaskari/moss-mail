//! What the engine tells the UI while it works: sync progress, new mail and problems.

use moss_core::ScheduledEmail;
use serde::Serialize;

use crate::AccountInfo;

/// How far an account's sync has got, for the Syncing screen and the account row.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgress {
    pub account_id: String,
    /// Inbox messages stored so far.
    pub done: u32,
    /// Folders found on the server.
    pub folders: u32,
    /// 0 to 1: how much of the last 30 days is in.
    pub recent: f64,
    /// Year of the oldest message stored, once there is one.
    pub oldest_year: Option<i32>,
    /// Older mail is still downloading.
    pub more: bool,
}

/// Why sync stopped for now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Problem {
    /// The server no longer accepts the saved password or token: sign in again.
    SignedOut,
    /// No connection to the server. Moss keeps retrying.
    Offline,
    /// The server refused something. Moss keeps retrying.
    Server,
}

/// Why an account isn't syncing right now. The engine keeps the latest one per account,
/// so a window that opens after the [`Event::SyncError`] still learns of it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncIssue {
    pub account_id: String,
    pub problem: Problem,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Event {
    #[serde(rename_all = "camelCase")]
    SyncProgress(SyncProgress),
    /// Threads changed (new mail, or changes from another app): reload the list.
    #[serde(rename_all = "camelCase")]
    MailChanged { account_id: String },
    #[serde(rename_all = "camelCase")]
    SyncError { account_id: String, problem: Problem, message: String },
    /// Sync is working again after a problem.
    #[serde(rename_all = "camelCase")]
    SyncOk { account_id: String },
    /// Durable server-action status changed independently of incoming sync.
    #[serde(rename_all = "camelCase")]
    ServerChangesChanged { account_id: String },
    /// Something about the account changed, such as the sender name learned from Sent or
    /// addresses Moss found: the account as [`crate::Engine::accounts`] lists it.
    #[serde(rename_all = "camelCase")]
    AccountUpdated { account: AccountInfo },
    /// Drafts came in from the account's Drafts folder, or went there: reload the Drafts list.
    #[serde(rename_all = "camelCase")]
    DraftsChanged { account_id: String },
    /// Moss found emails you sent that it stopped before they went, now back in Drafts: say so
    /// ([`crate::Engine::interrupted_sends`]).
    SendsInterrupted,
    /// A rule wrote a summary of this conversation in the background: reload its summaries.
    #[serde(rename_all = "camelCase")]
    SummaryWritten { thread_id: String },
    /// An email set to send later went: at its time, or `late` because Moss was closed (or
    /// the Mac asleep) then, so it went as soon as it could.
    /// Local repair can emit this again. Consumers must deduplicate by operation ID;
    /// the durable delivery receipt, not event counts, is the authority for sends.
    ScheduledSent { email: ScheduledEmail, late: bool },
    /// An active schedule was created, moved or cancelled by a client. Reload the list.
    ScheduledChanged,
    /// An email set to send later couldn't go. It stays, and is tried again at `next_try`.
    ScheduledFailed { email: ScheduledEmail },
    /// Ownership recovery prevented a send attempt. Counters and deadlines are unchanged.
    ScheduledWaiting { email: ScheduledEmail },
}

/// Where events go. The desktop app forwards them to the window.
pub trait Events: Send + Sync {
    fn emit(&self, event: Event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialise_with_a_type_tag_in_camel_case() {
        let json = serde_json::to_value(Event::MailChanged { account_id: "gmail".into() }).unwrap();
        assert_eq!(json, serde_json::json!({"type": "mailChanged", "accountId": "gmail"}));
        let json = serde_json::to_value(Event::DraftsChanged { account_id: "gmail".into() }).unwrap();
        assert_eq!(json, serde_json::json!({"type": "draftsChanged", "accountId": "gmail"}));
        let json = serde_json::to_value(Event::SummaryWritten { thread_id: "7".into() }).unwrap();
        assert_eq!(json, serde_json::json!({"type": "summaryWritten", "threadId": "7"}));
        let json = serde_json::to_value(Event::SyncError {
            account_id: "gmail".into(),
            problem: Problem::SignedOut,
            message: "no".into(),
        })
        .unwrap();
        assert_eq!(json["problem"], "signedOut");
        let progress = SyncProgress {
            account_id: "gmail".into(),
            done: 3,
            folders: 2,
            recent: 0.5,
            oldest_year: Some(2024),
            more: true,
        };
        let json = serde_json::to_value(Event::SyncProgress(progress)).unwrap();
        assert_eq!(json["type"], "syncProgress");
        assert_eq!(json["oldestYear"], 2024);
        assert_eq!(json["accountId"], "gmail");
    }

    #[test]
    fn send_later_events_carry_the_email() {
        let draft: moss_core::Draft = serde_json::from_value(serde_json::json!({
            "id": "d1", "mode": "new", "threadId": null, "fromAccountId": "gmail", "to": [], "cc": [],
            "bcc": [], "subject": "Lunch?", "body": "", "bodyHtml": "", "remindIfNoReply": false,
            "updatedAt": 1
        }))
        .unwrap();
        let email = ScheduledEmail { revision: 1, operation_id: "schedule-op".into(), draft, send_at: 5, next_try: 65, tries: 1, error: Some("offline".into()) };
        let json = serde_json::to_value(Event::ScheduledSent { email: email.clone(), late: true }).unwrap();
        assert_eq!((json["type"].as_str(), json["late"].as_bool()), (Some("scheduledSent"), Some(true)));
        assert_eq!(json["email"]["draft"]["subject"], "Lunch?");
        assert_eq!(json["email"]["sendAt"], 5);
        let json = serde_json::to_value(Event::ScheduledFailed { email: email.clone() }).unwrap();
        assert_eq!(json["type"], "scheduledFailed");
        assert_eq!((json["email"]["nextTry"].as_i64(), json["email"]["error"].as_str()), (Some(65), Some("offline")));
        let json=serde_json::to_value(Event::ScheduledWaiting {email}).unwrap();
        assert_eq!(json["type"],"scheduledWaiting");
        assert_eq!(json["email"]["tries"],1);
        assert_eq!(json["email"]["nextTry"],65);
    }
}
