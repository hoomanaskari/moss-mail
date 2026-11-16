//! Asks the embedding model (EmbeddingGemma) what texts mean, through llama-server's
//! OpenAI-style embeddings endpoint.

use serde_json::{json, Value};

use crate::server::Endpoint;
use crate::AiError;

/// One vector per text, in the same order.
pub async fn vectors(client: &reqwest::Client, endpoint: &Endpoint, texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
    let body = json!({"input": texts, "model": "embedder", "encoding_format": "float"});
    let reply: Value = crate::decide::post(client, endpoint, "/v1/embeddings", &body).await?;
    let mut rows: Vec<(u64, Vec<f32>)> = reply["data"]
        .as_array()
        .ok_or_else(|| AiError::Model("no vectors came back".into()))?
        .iter()
        .map(|row| {
            let vector = row["embedding"].as_array().map(|v| v.iter().filter_map(Value::as_f64).map(|x| x as f32).collect());
            Some((row["index"].as_u64()?, vector?))
        })
        .collect::<Option<_>>()
        .ok_or_else(|| AiError::Model("a vector came back damaged".into()))?;
    if rows.len() != texts.len() {
        return Err(AiError::Model(format!("{} vectors for {} texts", rows.len(), texts.len())));
    }
    rows.sort_by_key(|(i, _)| *i);
    Ok(rows.into_iter().map(|(_, v)| v).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn server(body: Value) -> (MockServer, Endpoint) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/embeddings"))
            .and(header("authorization", "Bearer k"))
            .and(body_partial_json(json!({"input": ["a", "b"]})))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let endpoint = Endpoint { url: server.uri(), key: "k".into() };
        (server, endpoint)
    }

    fn texts() -> Vec<String> {
        vec!["a".into(), "b".into()]
    }

    #[tokio::test]
    async fn vectors_come_back_in_the_order_asked() {
        let reply = json!({"data": [{"index": 1, "embedding": [0.0, 1.0]}, {"index": 0, "embedding": [1.0, 0.5]}]});
        let (_s, endpoint) = server(reply).await;
        let got = vectors(&crate::http::local(), &endpoint, &texts()).await.unwrap();
        assert_eq!(got, [vec![1.0, 0.5], vec![0.0, 1.0]]);
    }

    #[tokio::test]
    async fn a_short_or_damaged_answer_is_an_error() {
        for reply in [
            json!({"data": [{"index": 0, "embedding": [1.0]}]}),
            json!({"data": [{"index": 0}, {"index": 1, "embedding": [1.0]}]}),
            json!({"error": "no"}),
        ] {
            let (_s, endpoint) = server(reply).await;
            assert!(matches!(vectors(&crate::http::local(), &endpoint, &texts()).await, Err(AiError::Model(_))));
        }
    }
}
