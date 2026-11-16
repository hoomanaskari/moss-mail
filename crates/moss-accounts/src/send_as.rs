//! Gmail's "Send mail as" list: the other addresses a Gmail account sends from.

use serde::{Deserialize, Serialize};

use crate::error::{unreachable, SignInError};

pub const GMAIL_API: &str = "https://gmail.googleapis.com";

/// An address Gmail lets the account send from, besides its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendAs {
    pub email: String,
    /// The name Gmail puts on mail from it, when it has one.
    pub name: Option<String>,
}

/// The account's other addresses, leaving out its own and those still waiting to be
/// verified.
pub async fn send_as(
    http: &reqwest::Client,
    api: &str,
    access_token: &str,
) -> Result<Vec<SendAs>, SignInError> {
    let host = url::Url::parse(api)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_default();
    let response = http
        .get(format!(
            "{}/gmail/v1/users/me/settings/sendAs",
            api.trim_end_matches('/')
        ))
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| unreachable(&host, e))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| unreachable(&host, e))?;
    if status.as_u16() == 401 {
        return Err(SignInError::SignInExpired);
    }
    let unexpected = |detail: String| SignInError::UnexpectedResponse {
        host: host.clone(),
        detail,
    };
    if !status.is_success() {
        return Err(unexpected(format!("HTTP {status}")));
    }
    let list: SendAsList =
        serde_json::from_str(&body).map_err(|_| unexpected("The sender alias list could not be read.".into()))?;
    Ok(list
        .send_as
        .into_iter()
        .filter(|a| !a.is_primary && a.verification_status.as_deref() != Some("pending"))
        .map(|a| SendAs {
            email: a.send_as_email.trim().to_lowercase(),
            name: a
                .display_name
                .map(|n| n.trim().to_string())
                .filter(|n| !n.is_empty()),
        })
        .collect())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SendAsList {
    #[serde(default)]
    send_as: Vec<GmailSendAs>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GmailSendAs {
    send_as_email: String,
    display_name: Option<String>,
    #[serde(default)]
    is_primary: bool,
    verification_status: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const PATH: &str = "/gmail/v1/users/me/settings/sendAs";

    async fn reply(status: u16, body: serde_json::Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(PATH))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&server)
            .await;
        server
    }

    async fn ask(server: &MockServer) -> Result<Vec<SendAs>, SignInError> {
        send_as(
            &reqwest::Client::builder().no_proxy().build().unwrap(),
            &format!("{}/", server.uri()),
            "tok",
        )
        .await
    }

    #[tokio::test]
    async fn lists_the_other_verified_addresses_with_their_names() {
        let server = MockServer::start().await;
        let body = serde_json::json!({"sendAs": [
            {"sendAsEmail": "maya.ross@gmail.com", "displayName": "", "isPrimary": true, "isDefault": true},
            {"sendAsEmail": "Hi@RossStudio.art", "displayName": "Ross Studio", "verificationStatus": "accepted"},
            {"sendAsEmail": "shop@rossstudio.art", "displayName": " ", "verificationStatus": "accepted", "isPrimary": false},
            {"sendAsEmail": "new@rossstudio.art", "displayName": "New", "verificationStatus": "pending"},
            {"sendAsEmail": "old@rossstudio.art"}
        ]});
        Mock::given(method("GET"))
            .and(path(PATH))
            .and(header("authorization", "Bearer tok"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let named = |email: &str, name: Option<&str>| SendAs {
            email: email.into(),
            name: name.map(String::from),
        };
        assert_eq!(
            ask(&server).await.unwrap(),
            [
                named("hi@rossstudio.art", Some("Ross Studio")),
                named("shop@rossstudio.art", None),
                named("old@rossstudio.art", None),
            ]
        );
    }

    #[tokio::test]
    async fn an_account_with_none_lists_none() {
        assert!(ask(&reply(200, serde_json::json!({})).await)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn a_refused_token_means_signing_in_again() {
        let server = reply(401, serde_json::json!({"error": {"code": 401}})).await;
        assert_eq!(ask(&server).await.unwrap_err(), SignInError::SignInExpired);
    }

    #[tokio::test]
    async fn malformed_alias_values_do_not_enter_errors() {
        let marker = "synthetic-private-alias-marker";
        let server = reply(200, serde_json::json!({"sendAs":marker})).await;
        let error = ask(&server).await.unwrap_err();
        assert!(matches!(error, SignInError::UnexpectedResponse { .. }));
        assert!(!format!("{error:?} {error} {}", serde_json::to_string(&error).unwrap()).contains(marker));
    }

    #[tokio::test]
    async fn other_failures_say_what_happened() {
        let server = reply(500, serde_json::json!({"error": {"message": "backend"}})).await;
        let err = ask(&server).await.unwrap_err();
        assert!(
            matches!(&err, SignInError::UnexpectedResponse { detail, .. } if detail.contains("500")),
            "{err}"
        );
        let server = reply(200, serde_json::json!({"sendAs": "nope"})).await;
        assert!(matches!(
            ask(&server).await,
            Err(SignInError::UnexpectedResponse { .. })
        ));
        let err = send_as(&reqwest::Client::builder().no_proxy().build().unwrap(), "http://127.0.0.1:9", "tok")
            .await
            .unwrap_err();
        assert!(
            matches!(err, SignInError::Unreachable { ref host, .. } if host == "127.0.0.1"),
            "{err}"
        );
    }
}
