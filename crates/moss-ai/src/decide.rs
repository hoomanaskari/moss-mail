//! Asks the decision model typed questions about some text, through llama-server's
//! `/v1/systemone`. It answers with a probability per option and writes no text.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::server::Endpoint;
use crate::AiError;

/// One question. Every option is described in words: bare labels route worse.
#[derive(Debug, Clone, PartialEq)]
pub enum Question {
    /// Pick one of these `(label, description)` options.
    Choice {
        instructions: String,
        options: Vec<(String, String)>,
    },
    /// Yes or no.
    YesNo { instructions: String },
}

/// The model's answer to one question.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Answer {
    /// Probability of each option, by label.
    Choice(BTreeMap<String, f64>),
    /// Probability of "yes".
    YesNo(f64),
}

impl Answer {
    /// How likely "yes" is: the yes/no probability, or a choice's "yes" option.
    pub fn yes(&self) -> f64 {
        match self {
            Answer::YesNo(p) => *p,
            Answer::Choice(p) => p.get("yes").copied().unwrap_or(0.0),
        }
    }

    /// A choice's likeliest option and how likely it is. `None` for a yes/no answer.
    pub fn best(&self) -> Option<(String, f64)> {
        match self {
            Answer::YesNo(_) => None,
            Answer::Choice(p) => p.iter().max_by(|a, b| a.1.total_cmp(b.1)).map(|(k, v)| (k.clone(), *v)),
        }
    }
}

/// Asks every question about `state` in one request. Every question gets an answer of its
/// own type, or the whole request is an error.
pub async fn ask(
    client: &reqwest::Client,
    endpoint: &Endpoint,
    state: &str,
    questions: &[(&str, Question)],
) -> Result<BTreeMap<String, Answer>, AiError> {
    let typed: serde_json::Map<String, Value> = questions
        .iter()
        .map(|(id, q)| {
            let q = match q {
                Question::YesNo { instructions } => {
                    json!({"type": "noul", "instructions": instructions})
                }
                Question::Choice {
                    instructions,
                    options,
                } => {
                    let criteria: serde_json::Map<String, Value> = options
                        .iter()
                        .map(|(label, d)| (label.clone(), json!(d)))
                        .collect();
                    json!({"type": "choice", "instructions": instructions, "criteria": criteria})
                }
            };
            (id.to_string(), q)
        })
        .collect();
    let state = fit(client, endpoint, state).await;
    let body = json!({"state": state, "questions": typed});
    let reply = post(client, endpoint, "/v1/systemone", &body).await?;
    questions
        .iter()
        .map(|(id, q)| {
            let a = &reply["answers"][id];
            let answer = match q {
                Question::YesNo { .. } => a["noul"].as_f64().map(Answer::YesNo),
                Question::Choice { .. } => a["probabilities"].as_object().map(|p| {
                    Answer::Choice(
                        p.iter()
                            .filter_map(|(k, v)| Some((k.clone(), v.as_f64()?)))
                            .collect(),
                    )
                }),
            };
            let answer = answer.filter(|a| !matches!(a, Answer::Choice(p) if p.is_empty()));
            Ok((
                id.to_string(),
                answer.ok_or_else(|| AiError::Model(format!("no answer to {id}")))?,
            ))
        })
        .collect()
}

/// How many tokens of email the decision model is shown. It reads the email and the questions
/// in one batch of 2,048 tokens (`-b` and `-ub` in `manager.rs`), and a longer one is refused;
/// this leaves room for the questions.
pub const STATE_TOKENS: usize = 1_700;

/// `state`, cut to [`STATE_TOKENS`] tokens by the model's own tokenizer: long links and some
/// scripts take far more tokens than their letters suggest. A token is at least a byte, so
/// shorter text isn't counted. When counting fails, the text goes as it is.
async fn fit(client: &reqwest::Client, endpoint: &Endpoint, state: &str) -> String {
    if state.len() <= STATE_TOKENS {
        return state.to_string();
    }
    let counted = post(client, endpoint, "/tokenize", &json!({"content": state})).await;
    let tokens = counted.ok().and_then(|c| c["tokens"].as_array().cloned()).unwrap_or_default();
    if tokens.len() <= STATE_TOKENS {
        return state.to_string();
    }
    let cut = post(client, endpoint, "/detokenize", &json!({"tokens": tokens[..STATE_TOKENS]})).await;
    cut.ok().and_then(|c| c["content"].as_str().map(String::from)).unwrap_or_else(|| state.to_string())
}

/// POSTs JSON to a running model with its key, and reads the JSON back.
pub(crate) async fn post(
    client: &reqwest::Client,
    endpoint: &Endpoint,
    path: &str,
    body: &Value,
) -> Result<Value, AiError> {
    let failed = |e: reqwest::Error| AiError::Model(e.to_string());
    let response = client
        .post(format!("{}{path}", endpoint.url))
        .bearer_auth(&endpoint.key)
        .json(body)
        .send()
        .await
        .map_err(failed)?;
    if !response.status().is_success() {
        return Err(AiError::Model(format!(
            "{path} answered {}",
            response.status().as_u16()
        )));
    }
    response.json().await.map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn questions() -> Vec<(&'static str, Question)> {
        vec![
            (
                "reply",
                Question::YesNo {
                    instructions: "Does the sender ask the user to do something?".into(),
                },
            ),
            (
                "split",
                Question::Choice {
                    instructions: "Which inbox?".into(),
                    options: vec![
                        ("important".into(), "from a person".into()),
                        ("other".into(), "everything else".into()),
                    ],
                },
            ),
        ]
    }

    async fn server(status: u16, body: Value) -> (MockServer, Endpoint) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .and(header("authorization", "Bearer k1"))
            .and(body_json(json!({
                "state": "From: Dana\n\nCan you send the deck?",
                "questions": {
                    "reply": {"type": "noul", "instructions": "Does the sender ask the user to do something?"},
                    "split": {"type": "choice", "instructions": "Which inbox?",
                              "criteria": {"important": "from a person", "other": "everything else"}}
                }
            })))
            .respond_with(ResponseTemplate::new(status).set_body_json(body))
            .mount(&server)
            .await;
        let endpoint = Endpoint {
            url: server.uri(),
            key: "k1".into(),
        };
        (server, endpoint)
    }

    #[tokio::test]
    async fn sends_typed_questions_and_reads_the_probabilities() {
        let (_s, endpoint) = server(
            200,
            json!({"answers": {
                "reply": {"noul": 0.91},
                "split": {"probabilities": {"important": 0.8, "other": 0.2}}
            }}),
        )
        .await;
        let answers = ask(
            &crate::http::local(),
            &endpoint,
            "From: Dana\n\nCan you send the deck?",
            &questions(),
        )
        .await
        .unwrap();
        assert_eq!(answers["reply"], Answer::YesNo(0.91));
        assert_eq!(
            answers["split"],
            Answer::Choice(BTreeMap::from([
                ("important".into(), 0.8),
                ("other".into(), 0.2)
            ]))
        );
    }

    #[tokio::test]
    async fn a_long_email_is_cut_to_the_tokens_one_batch_holds() {
        let server = MockServer::start().await;
        let long = "x".repeat(STATE_TOKENS + 1);
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .and(body_json(json!({"content": long})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tokens": vec![7; STATE_TOKENS + 5]})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/detokenize"))
            .and(body_json(json!({"tokens": vec![7; STATE_TOKENS]})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"content": "cut"})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .and(wiremock::matchers::body_partial_json(json!({"state": "cut"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answers": {"reply": {"noul": 0.4}}})))
            .mount(&server)
            .await;
        let endpoint = Endpoint { url: server.uri(), key: "k1".into() };
        let reply = [("reply", questions().remove(0).1)];
        let answers = ask(&crate::http::local(), &endpoint, &long, &reply).await.unwrap();
        assert_eq!(answers["reply"], Answer::YesNo(0.4));
    }

    #[tokio::test]
    async fn a_long_email_that_fits_or_can_t_be_counted_is_sent_as_it_is() {
        let server = MockServer::start().await;
        let long = "x".repeat(STATE_TOKENS + 1);
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tokens": vec![7; 10]})))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/systemone"))
            .and(wiremock::matchers::body_partial_json(json!({"state": long})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"answers": {"reply": {"noul": 0.4}}})))
            .mount(&server)
            .await;
        let endpoint = Endpoint { url: server.uri(), key: "k1".into() };
        let reply = [("reply", questions().remove(0).1)];
        // Few tokens after all.
        assert!(ask(&crate::http::local(), &endpoint, &long, &reply).await.is_ok());
        // The count fails (404 now): the model says whether it can read it.
        assert!(ask(&crate::http::local(), &endpoint, &long, &reply).await.is_ok());
        // Too many, but cutting fails (no /detokenize): the same.
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"tokens": vec![7; STATE_TOKENS + 5]})))
            .mount(&server)
            .await;
        assert!(ask(&crate::http::local(), &endpoint, &long, &reply).await.is_ok());
    }

    #[test]
    fn reads_yes_from_either_kind_of_answer() {
        assert_eq!(Answer::YesNo(0.7).yes(), 0.7);
        assert_eq!(
            Answer::Choice(BTreeMap::from([("yes".into(), 0.6)])).yes(),
            0.6
        );
        assert_eq!(
            Answer::Choice(BTreeMap::from([("no".into(), 1.0)])).yes(),
            0.0
        );
    }

    #[tokio::test]
    async fn a_missing_or_odd_answer_is_an_error() {
        let client = crate::http::local();
        let state = "From: Dana\n\nCan you send the deck?";
        let (_s, endpoint) = server(200, json!({"answers": {"reply": {"noul": 0.5}}})).await;
        assert!(matches!(
            ask(&client, &endpoint, state, &questions()).await,
            Err(AiError::Model(_))
        ));
        let (_s, endpoint) = server(
            200,
            json!({"answers": {"reply": {"noul": "yes"}, "split": {}}}),
        )
        .await;
        assert!(matches!(
            ask(&client, &endpoint, state, &questions()).await,
            Err(AiError::Model(_))
        ));
        // A choice with no labels in it is no answer either.
        let (_s, endpoint) = server(
            200,
            json!({"answers": {"reply": {"noul": 0.5}, "split": {"probabilities": {}}}}),
        )
        .await;
        assert!(matches!(
            ask(&client, &endpoint, state, &questions()).await,
            Err(AiError::Model(_))
        ));
        let (_s, endpoint) = server(500, json!({"error": "boom"})).await;
        assert!(matches!(
            ask(&client, &endpoint, state, &questions()).await,
            Err(AiError::Model(_))
        ));
        let gone = Endpoint {
            url: "http://127.0.0.1:9".into(),
            key: "k".into(),
        };
        assert!(matches!(
            ask(&client, &gone, state, &questions()).await,
            Err(AiError::Model(_))
        ));
    }
}
