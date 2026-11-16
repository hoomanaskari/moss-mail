//! Checking a Resend API key with one cheap call: listing the account's domains.

use serde::{Deserialize, Serialize};

use crate::error::{unreachable, SignInError};

pub const RESEND_API: &str = "https://api.resend.com";

/// A sending domain in the Resend account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResendDomain {
    pub name: String,
    /// `verified`, `pending`, `failed`, … as Resend reports it.
    pub status: String,
}

#[derive(Deserialize)]
struct DomainList {
    data: Vec<ResendDomain>,
}

#[derive(Deserialize)]
struct ErrorBody {
    #[serde(default)]
    name: String,
}

/// Lists the account's domains, which proves the key works and has Full access.
pub async fn check_key(
    http: &reqwest::Client,
    api: &str,
    key: &str,
) -> Result<Vec<ResendDomain>, SignInError> {
    let host = url::Url::parse(api)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .unwrap_or_default();
    let key = key.trim();
    if key.is_empty() {
        return Err(SignInError::InvalidResendKey);
    }
    let response = http
        .get(format!("{}/domains", api.trim_end_matches('/')))
        .bearer_auth(key)
        .send()
        .await
        .map_err(|e| unreachable(&host, e))?;
    let status = response.status();
    let body = response.text().await.map_err(|e| unreachable(&host, e))?;
    if status.is_success() {
        return serde_json::from_str::<DomainList>(&body)
            .map(|list| list.data)
            .map_err(|_| SignInError::UnexpectedResponse {
                host,
                detail: "The domain list could not be read.".into(),
            });
    }
    let error: ErrorBody = serde_json::from_str(&body).unwrap_or(ErrorBody {
        name: String::new(),
    });
    match (error.name.as_str(), status.as_u16()) {
        ("restricted_api_key", _) => Err(SignInError::ResendKeyRestricted),
        // Listing domains takes no input, so a 400 is about the key: Resend answers a key it
        // doesn't know with 400 `validation_error`.
        ("invalid_api_key" | "missing_api_key", _) | (_, 400 | 401 | 403) => {
            Err(SignInError::InvalidResendKey)
        }
        _ => Err(SignInError::UnexpectedResponse {
            host,
            detail: format!("HTTP {status}"),
        }),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    pub fn domains_json() -> serde_json::Value {
        serde_json::json!({
            "object": "list",
            "has_more": false,
            "data": [
                { "id": "d1", "name": "northwind.co", "status": "verified", "region": "us-east-1" },
                { "id": "d2", "name": "studio.northwind.co", "status": "pending", "region": "eu-west-1" }
            ]
        })
    }

    async fn reply(status: u16, body: serde_json::Value) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/domains"))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn a_full_access_key_lists_domains() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/domains"))
            .and(header("authorization", "Bearer re_key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(domains_json()))
            .expect(1)
            .mount(&server)
            .await;
        let domains = check_key(
            &reqwest::Client::builder().no_proxy().build().unwrap(),
            &format!("{}/", server.uri()),
            " re_key ",
        )
        .await
        .unwrap();
        assert_eq!(
            domains,
            vec![
                ResendDomain {
                    name: "northwind.co".into(),
                    status: "verified".into()
                },
                ResendDomain {
                    name: "studio.northwind.co".into(),
                    status: "pending".into()
                },
            ]
        );
    }

    #[tokio::test]
    async fn domain_errors_do_not_expose_external_values() {
        let marker = "synthetic-private-domain-marker";
        let mut leaks = Vec::new();
        for (status, body) in [
            (500, serde_json::json!({"message":marker,"name":marker})),
            (200, serde_json::json!({"data":marker})),
        ] {
            let server = reply(status, body).await;
            let error = check_key(&reqwest::Client::builder().no_proxy().build().unwrap(), &server.uri(), "synthetic-key").await.unwrap_err();
            assert!(matches!(error, SignInError::UnexpectedResponse { .. }));
            if format!("{error:?} {error} {}", serde_json::to_string(&error).unwrap()).contains(marker) { leaks.push(status); }
        }
        assert!(leaks.is_empty(), "domain details exposed at statuses {leaks:?}");
    }

    #[tokio::test]
    async fn key_problems_are_typed() {
        let http = reqwest::Client::builder().no_proxy().build().unwrap();
        let cases = [
            (
                401,
                serde_json::json!({"statusCode": 401, "name": "restricted_api_key", "message": "only send"}),
                SignInError::ResendKeyRestricted,
            ),
            (
                400,
                serde_json::json!({"statusCode": 400, "name": "invalid_api_key", "message": "API key is invalid"}),
                SignInError::InvalidResendKey,
            ),
            (
                // What Resend answers for a key it doesn't know, as of November 2026.
                400,
                serde_json::json!({"statusCode": 400, "name": "validation_error", "message": "API key is invalid"}),
                SignInError::InvalidResendKey,
            ),
            (
                401,
                serde_json::json!({"name": "missing_api_key"}),
                SignInError::InvalidResendKey,
            ),
            (
                403,
                serde_json::json!("nope"),
                SignInError::InvalidResendKey,
            ),
            (
                500,
                serde_json::json!({"name": "internal_server_error", "message": "down"}),
                SignInError::UnexpectedResponse {
                    host: "127.0.0.1".into(),
                    detail: "HTTP 500 Internal Server Error".into(),
                },
            ),
        ];
        for (status, body, expected) in cases {
            let server = reply(status, body).await;
            assert_eq!(
                check_key(&http, &server.uri(), "re_key").await,
                Err(expected)
            );
        }
        let server = reply(200, serde_json::json!({"unexpected": true})).await;
        let err = check_key(&http, &server.uri(), "re_key").await.unwrap_err();
        assert!(
            matches!(err, SignInError::UnexpectedResponse { ref detail, .. } if detail == "The domain list could not be read.")
        );
        assert_eq!(
            check_key(&http, "http://127.0.0.1:1", "  ").await,
            Err(SignInError::InvalidResendKey)
        );
    }

    #[tokio::test]
    async fn an_unreachable_api_is_reported() {
        let err = check_key(&reqwest::Client::builder().no_proxy().build().unwrap(), "http://127.0.0.1:1", "re_key")
            .await
            .unwrap_err();
        assert!(matches!(err, SignInError::Unreachable { ref host, .. } if host == "127.0.0.1"));
    }

    #[tokio::test]
    async fn a_body_that_breaks_off_is_unreachable() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let _ = s.read(&mut buf).await;
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n{")
                .await
                .unwrap();
        });
        let err = check_key(
            &reqwest::Client::builder().no_proxy().build().unwrap(),
            &format!("http://127.0.0.1:{port}"),
            "re_key",
        )
        .await
        .unwrap_err();
        assert!(matches!(err, SignInError::Unreachable { .. }));
    }
}
