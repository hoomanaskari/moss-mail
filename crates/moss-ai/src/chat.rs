//! A chat with a model, streamed, with tools it may call: what the agent and the thread chat
//! talk to.
//!
//! [`ChatModel`] is the one writer interface: every job that writes text (summaries, replies,
//! rewrites, Ask Moss, the one-liners) asks [`Ai`] through it. [`Ai`] sends it to the local
//! writing model in llama-server, or to Apple Intelligence through its helper. Each job fits
//! what it sends to the model's context with [`room`].

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::server::Endpoint;
use crate::{Ai, AiError};

/// One message in a chat.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    System(String),
    User(String),
    /// What the model said, and the tools it asked for.
    Assistant { text: String, calls: Vec<ToolCall> },
    /// A tool's result, for the call with this id.
    Tool { id: String, text: String },
}

impl Msg {
    /// The message as the chat endpoint takes it.
    pub fn to_json(&self) -> Value {
        match self {
            Msg::System(text) => json!({"role": "system", "content": text}),
            Msg::User(text) => json!({"role": "user", "content": text}),
            Msg::Assistant { text, calls } if calls.is_empty() => json!({"role": "assistant", "content": text}),
            Msg::Assistant { text, calls } => {
                let calls: Vec<Value> = calls
                    .iter()
                    .map(|c| json!({"id": c.id, "type": "function", "function": {"name": c.name, "arguments": c.arguments}}))
                    .collect();
                json!({"role": "assistant", "content": text, "tool_calls": calls})
            }
            Msg::Tool { id, text } => json!({"role": "tool", "tool_call_id": id, "content": text}),
        }
    }

    /// The characters it holds, to keep a chat inside the model's context.
    pub fn chars(&self) -> usize {
        match self {
            Msg::System(text) | Msg::User(text) | Msg::Tool { text, .. } => text.chars().count(),
            Msg::Assistant { text, calls } => {
                text.chars().count() + calls.iter().map(|c| c.name.len() + c.arguments.chars().count()).sum::<usize>()
            }
        }
    }
}

/// A tool the model asked for, with its arguments as the JSON text it wrote.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// What to send: the chat so far, the tools on offer (OpenAI `tools` entries), how long the
/// answer may be, how freely to write, and the JSON shape the answer must have, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub messages: Vec<Msg>,
    pub tools: Vec<Value>,
    pub max_tokens: u32,
    pub temperature: f64,
    /// A JSON schema the answer must follow. A writer that can't hold the model to it relies
    /// on the prompt asking for JSON.
    pub format: Option<Value>,
    /// The model has to call one of `tools`, not answer in words.
    pub must_call: bool,
}

impl ChatRequest {
    /// `messages`, with no tools, written at temperature 0.2.
    pub fn new(messages: Vec<Msg>, max_tokens: u32) -> ChatRequest {
        ChatRequest { messages, tools: vec![], max_tokens, temperature: 0.2, format: None, must_call: false }
    }

    /// A system instruction and one user message.
    pub fn ask(system: &str, user: &str, max_tokens: u32) -> ChatRequest {
        ChatRequest::new(vec![Msg::System(system.into()), Msg::User(user.into())], max_tokens)
    }

    pub fn temperature(mut self, temperature: f64) -> ChatRequest {
        self.temperature = temperature;
        self
    }

    pub fn format(mut self, schema: Value) -> ChatRequest {
        self.format = Some(schema);
        self
    }
}

/// What came back: text, tool calls, or both. `stopped` when `on_text` asked to stop.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Reply {
    pub text: String,
    pub calls: Vec<ToolCall>,
    pub stopped: bool,
}

/// Hears each piece of text as it's written; returns `false` to stop. (Spelled out so
/// `async_trait` keeps it working for text of any lifetime.)
pub type OnText<'a> = dyn for<'s> FnMut(&'s str) -> bool + Send + 'a;

/// A model to chat with. `on_text` hears each piece of text as it's written, and returns
/// `false` to stop.
#[async_trait]
pub trait ChatModel: Send + Sync {
    async fn chat(&self, request: &ChatRequest, on_text: &mut OnText) -> Result<Reply, AiError>;

    /// Tokens it reads and writes at once: the Gemma writer's 32k unless it says otherwise.
    fn context_tokens(&self) -> usize {
        crate::agent::CONTEXT_TOKENS
    }
}

/// `chars`, a budget set for the Gemma writer's context, scaled to `model`'s: a quarter of it
/// for Apple Intelligence's 8k.
pub fn room(chars: usize, model: &dyn ChatModel) -> usize {
    chars * model.context_tokens() / crate::agent::CONTEXT_TOKENS
}

/// The request body for llama-server's chat endpoint: streamed, thinking off.
pub fn body(request: &ChatRequest) -> Value {
    let mut body = json!({
        "messages": request.messages.iter().map(Msg::to_json).collect::<Vec<_>>(),
        "max_tokens": request.max_tokens,
        "temperature": request.temperature,
        "stream": true,
        "chat_template_kwargs": {"enable_thinking": false},
    });
    if !request.tools.is_empty() {
        body["tools"] = Value::Array(request.tools.clone());
        // Done on five emails is five calls in one step, not five steps.
        body["parallel_tool_calls"] = json!(true);
        if request.must_call {
            body["tool_choice"] = json!("required");
        }
    }
    if let Some(schema) = &request.format {
        body["response_format"] = json!({"type": "json_schema", "json_schema": {"name": "answer", "schema": schema}});
    }
    body
}

/// The model's answer to a question of its own, in a shape: the arguments of the one tool it has
/// to call (`name`, taking `parameters`). Gemma, asked for JSON instead, thinks aloud and runs out
/// of room before it answers. None when it doesn't call it.
pub async fn decided(model: &dyn ChatModel, system: &str, ask: String, (name, description, parameters): (&str, &str, Value), tokens: u32) -> Option<Value> {
    let tool = json!({"type": "function", "function": {"name": name, "description": description, "parameters": parameters}});
    let messages = vec![Msg::System(system.into()), Msg::User(ask)];
    let request = ChatRequest { tools: vec![tool], must_call: true, temperature: 0.0, ..ChatRequest::new(messages, tokens) };
    let reply = model.chat(&request, &mut |_| true).await.ok()?;
    let call = reply.calls.into_iter().find(|c| c.name == name)?;
    serde_json::from_str(&call.arguments).ok()
}

/// Why a streamed chat failed, before it's put in words for whoever asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The server answered with this status and body.
    Status(u16, String),
    /// The server couldn't be reached, or the connection dropped.
    Connection(String),
    /// The answer came, but said it failed or couldn't be read.
    Answer(String),
}

/// Sends a request for an OpenAI-style streamed chat and reads the answer as it comes.
pub(crate) async fn send_streamed<F: FnMut(&str) -> bool + Send + ?Sized>(
    request: reqwest::RequestBuilder,
    on_text: &mut F,
) -> Result<Reply, Failure> {
    let lost = |e: reqwest::Error| Failure::Connection(e.to_string());
    let mut response = request.send().await.map_err(lost)?;
    let status = response.status();
    if !status.is_success() {
        let why = response.text().await.unwrap_or_default();
        return Err(Failure::Status(status.as_u16(), why.trim().to_string()));
    }
    let mut events = Events::default();
    let unreadable = |e: AiError| Failure::Answer(e.to_string());
    while let Some(bytes) = response.chunk().await.map_err(lost)? {
        if !events.push(&bytes, on_text).map_err(unreadable)? {
            events.reply.stopped = true;
            return Ok(events.reply);
        }
    }
    events.finish(on_text).map_err(unreadable)
}

/// Reads the server-sent events of a streamed answer, in whatever pieces they arrive.
#[derive(Default)]
struct Events {
    /// Bytes of a line not yet ended.
    partial: Vec<u8>,
    reply: Reply,
    done: bool,
}

impl Events {
    /// Takes in some bytes. Returns `false` once `on_text` asks to stop.
    fn push<F: FnMut(&str) -> bool + ?Sized>(&mut self, bytes: &[u8], on_text: &mut F) -> Result<bool, AiError> {
        self.partial.extend_from_slice(bytes);
        while let Some(end) = self.partial.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=end).collect();
            if !self.line(&String::from_utf8_lossy(&line), on_text)? {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// The end of the stream: reads a last line that had no line break.
    fn finish<F: FnMut(&str) -> bool + ?Sized>(mut self, on_text: &mut F) -> Result<Reply, AiError> {
        let rest = String::from_utf8_lossy(&std::mem::take(&mut self.partial)).into_owned();
        self.line(&rest, on_text)?;
        Ok(self.reply)
    }

    /// One line of the stream. Only `data:` lines carry anything.
    fn line<F: FnMut(&str) -> bool + ?Sized>(&mut self, line: &str, on_text: &mut F) -> Result<bool, AiError> {
        let Some(data) = line.trim().strip_prefix("data:").map(str::trim) else {
            return Ok(true);
        };
        if self.done || data == "[DONE]" {
            self.done = true;
            return Ok(true);
        }
        let event: Value = serde_json::from_str(data).map_err(|e| AiError::Model(format!("can't read the answer: {e}")))?;
        if let Some(error) = event.get("error") {
            let message = error["message"].as_str().map_or_else(|| error.to_string(), str::to_string);
            return Err(AiError::Model(message));
        }
        let delta = &event["choices"][0]["delta"];
        for piece in delta["tool_calls"].as_array().into_iter().flatten() {
            let index = piece["index"].as_u64().unwrap_or(0) as usize;
            if self.reply.calls.len() <= index {
                self.reply.calls.resize(index + 1, ToolCall::default());
            }
            let call = &mut self.reply.calls[index];
            if let Some(id) = piece["id"].as_str() {
                call.id = id.to_string();
            }
            if let Some(name) = piece["function"]["name"].as_str() {
                call.name.push_str(name);
            }
            if let Some(arguments) = piece["function"]["arguments"].as_str() {
                call.arguments.push_str(arguments);
            }
        }
        match delta["content"].as_str().filter(|t| !t.is_empty()) {
            Some(text) => {
                self.reply.text.push_str(text);
                Ok(on_text(text))
            }
            None => Ok(true),
        }
    }
}

/// One streamed chat with llama-server.
pub async fn stream<F: FnMut(&str) -> bool + Send + ?Sized>(
    client: &reqwest::Client,
    endpoint: &Endpoint,
    request: &ChatRequest,
    on_text: &mut F,
) -> Result<Reply, AiError> {
    let post = client
        .post(format!("{}/v1/chat/completions", endpoint.url))
        .bearer_auth(&endpoint.key)
        .json(&body(request));
    send_streamed(post, on_text).await.map_err(|failure| match failure {
        Failure::Status(status, why) => AiError::Model(format!("the model answered {status}: {why}")),
        Failure::Connection(why) => AiError::Model(why),
        Failure::Answer(why) => AiError::Model(why.trim_start_matches("the AI model failed: ").to_string()),
    })
}

/// The writing model on this Mac. A failed chat restarts it next time.
#[async_trait]
impl ChatModel for Ai {
    async fn chat(&self, request: &ChatRequest, on_text: &mut OnText) -> Result<Reply, AiError> {
        let _busy = self.writing();
        let endpoint = self.writer().await?;
        let reply = stream(self.http(), &endpoint, request, on_text).await;
        if reply.is_err() {
            self.writer_failed().await;
        }
        reply
    }

    fn context_tokens(&self) -> usize {
        Ai::context_tokens(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn call(id: &str, name: &str, arguments: &str) -> ToolCall {
        ToolCall { id: id.into(), name: name.into(), arguments: arguments.into() }
    }

    #[test]
    fn a_model_with_a_smaller_context_gets_less_of_everything() {
        let gemma = crate::test_mail::ScriptedModel::new(vec![]);
        assert_eq!(gemma.context_tokens(), crate::agent::CONTEXT_TOKENS);
        assert_eq!(room(48_000, &gemma), 48_000);
        let apple = crate::test_mail::ScriptedModel::new(vec![]).with_context(8_192);
        assert_eq!(room(48_000, &apple), 12_000);
    }

    #[test]
    fn messages_go_out_in_the_openai_shape() {
        assert_eq!(Msg::System("s".into()).to_json(), json!({"role": "system", "content": "s"}));
        assert_eq!(Msg::User("u".into()).to_json(), json!({"role": "user", "content": "u"}));
        assert_eq!(
            Msg::Assistant { text: "a".into(), calls: vec![] }.to_json(),
            json!({"role": "assistant", "content": "a"})
        );
        assert_eq!(
            Msg::Assistant { text: "".into(), calls: vec![call("c1", "search_mail", "{\"query\":\"q4\"}")] }.to_json(),
            json!({"role": "assistant", "content": "", "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "search_mail", "arguments": "{\"query\":\"q4\"}"}}
            ]})
        );
        assert_eq!(
            Msg::Tool { id: "c1".into(), text: "r".into() }.to_json(),
            json!({"role": "tool", "tool_call_id": "c1", "content": "r"})
        );
    }

    #[test]
    fn counts_the_characters_of_each_message() {
        assert_eq!(Msg::User("héllo".into()).chars(), 5);
        assert_eq!(Msg::Tool { id: "x".into(), text: "abc".into() }.chars(), 3);
        assert_eq!(Msg::Assistant { text: "ab".into(), calls: vec![call("c", "find", "{}")] }.chars(), 2 + 4 + 2);
    }

    #[test]
    fn the_body_streams_with_thinking_off_and_tools_only_when_offered() {
        let mut request = ChatRequest::new(vec![Msg::User("hi".into())], 50);
        let without = body(&request);
        assert_eq!(without["stream"], true);
        assert_eq!(without["max_tokens"], 50);
        assert_eq!(without["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(without["messages"][0]["content"], "hi");
        assert!(without.get("tools").is_none());
        request.tools = vec![json!({"type": "function"})];
        assert_eq!(body(&request)["tools"], json!([{"type": "function"}]));
        assert_eq!(body(&request)["parallel_tool_calls"], true, "several emails in one step");
        assert!(body(&request).get("tool_choice").is_none());
        request.must_call = true;
        assert_eq!(body(&request)["tool_choice"], "required", "an answer in a shape: a call it has to make");
        assert!(without.get("parallel_tool_calls").is_none());
    }

    #[test]
    fn a_request_says_how_freely_to_write_and_the_shape_of_the_answer() {
        let plain = ChatRequest::ask("Be brief.", "Email", 40);
        assert_eq!(plain.messages, [Msg::System("Be brief.".into()), Msg::User("Email".into())]);
        let plain = body(&plain);
        assert_eq!((plain["temperature"].as_f64(), plain.get("response_format")), (Some(0.2), None));
        let schema = json!({"type": "object"});
        let shaped = body(&ChatRequest::ask("s", "u", 40).temperature(0.4).format(schema.clone()));
        assert_eq!(shaped["temperature"].as_f64(), Some(0.4));
        assert_eq!(shaped["response_format"], json!({"type": "json_schema", "json_schema": {"name": "answer", "schema": schema}}));
    }

    fn chunk(delta: Value) -> String {
        format!("data: {}\n\n", json!({"choices": [{"index": 0, "delta": delta}]}))
    }

    fn sse(deltas: &[Value]) -> String {
        let mut text: String = deltas.iter().map(|d| chunk(d.clone())).collect();
        text.push_str("data: [DONE]\n\n");
        text
    }

    fn read(pieces: &[&[u8]]) -> (Result<Reply, AiError>, Vec<String>) {
        let mut heard = vec![];
        let mut on_text = |t: &str| {
            heard.push(t.to_string());
            true
        };
        let mut events = Events::default();
        for piece in pieces {
            if let Err(e) = events.push(piece, &mut on_text) {
                return (Err(e), heard);
            }
        }
        (events.finish(&mut on_text), heard)
    }

    #[test]
    fn text_arrives_piece_by_piece_even_split_inside_a_letter() {
        let stream = sse(&[json!({"role": "assistant", "content": null}), json!({"content": "Dana "}), json!({"content": "said 182k é"})]);
        let bytes = stream.as_bytes();
        let cut = stream.find('é').unwrap() + 1; // inside the two bytes of é
        let (reply, heard) = read(&[&bytes[..10], &bytes[10..cut], &bytes[cut..]]);
        assert_eq!(reply.unwrap(), Reply { text: "Dana said 182k é".into(), ..Default::default() });
        assert_eq!(heard, ["Dana ", "said 182k é"]);
    }

    #[test]
    fn tool_calls_are_put_together_from_their_pieces() {
        let stream = sse(&[
            json!({"tool_calls": [{"index": 0, "id": "c1", "type": "function", "function": {"name": "search_mail"}}]}),
            json!({"tool_calls": [{"index": 0, "function": {"arguments": "{"}}]}),
            json!({"tool_calls": [{"index": 0, "function": {"arguments": "\"query\":\"q4\"}"}}]}),
            json!({"tool_calls": [{"index": 1, "id": "c2", "function": {"name": "find_people", "arguments": "{}"}}]}),
        ]);
        let (reply, heard) = read(&[stream.as_bytes()]);
        let reply = reply.unwrap();
        assert_eq!(reply.calls, [call("c1", "search_mail", "{\"query\":\"q4\"}"), call("c2", "find_people", "{}")]);
        assert_eq!(reply.text, "");
        assert!(heard.is_empty());
    }

    #[test]
    fn a_last_line_without_a_line_break_still_counts_and_other_lines_are_ignored() {
        let stream = format!(": ping\n\nevent: x\n{}", chunk(json!({"content": "ok"})).trim_end());
        let (reply, _) = read(&[stream.as_bytes()]);
        assert_eq!(reply.unwrap().text, "ok");
        let (reply, _) = read(&[b"data: {\"choices\": []}\n"]);
        assert_eq!(reply.unwrap(), Reply::default());
    }

    #[test]
    fn nothing_after_done_is_read() {
        let stream = format!("{}data: [DONE]\n\n{}", chunk(json!({"content": "a"})), chunk(json!({"content": "b"})));
        let (reply, _) = read(&[stream.as_bytes()]);
        assert_eq!(reply.unwrap().text, "a");
    }

    #[test]
    fn an_error_in_the_stream_or_a_line_it_cannot_read_fails() {
        let (reply, _) = read(&[b"data: {\"error\": {\"message\": \"context full\"}}\n\n"]);
        assert_eq!(reply, Err(AiError::Model("context full".into())));
        let (reply, _) = read(&[b"data: {\"error\": \"boom\"}\n\n"]);
        assert_eq!(reply, Err(AiError::Model("\"boom\"".into())));
        let (reply, _) = read(&[b"data: {not json\n\n"]);
        assert!(matches!(reply, Err(AiError::Model(_))));
        let (reply, _) = read(&[b"data: {not json"]);
        assert!(matches!(reply, Err(AiError::Model(_))), "the last line too");
    }

    #[test]
    fn on_text_can_stop_it() {
        let stream = sse(&[json!({"content": "one "}), json!({"content": "two"})]);
        let mut events = Events::default();
        let mut on_text = |_: &str| false;
        assert!(!events.push(stream.as_bytes(), &mut on_text).unwrap());
        assert_eq!(events.reply.text, "one ", "what came before the stop is kept");
    }

    async fn server(response: ResponseTemplate) -> (MockServer, Endpoint) {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(header("authorization", "Bearer k"))
            .and(body_partial_json(json!({"stream": true, "messages": [{"role": "user", "content": "hi"}]})))
            .respond_with(response)
            .mount(&server)
            .await;
        let endpoint = Endpoint { url: server.uri(), key: "k".into() };
        (server, endpoint)
    }

    fn hi() -> ChatRequest {
        ChatRequest::new(vec![Msg::User("hi".into())], 10)
    }

    fn streamed(text: String) -> ResponseTemplate {
        ResponseTemplate::new(200).insert_header("content-type", "text/event-stream").set_body_string(text)
    }

    #[tokio::test]
    async fn streams_an_answer_from_the_server() {
        let (_s, endpoint) = server(streamed(sse(&[json!({"content": "Hel"}), json!({"content": "lo"})]))).await;
        let mut heard = String::new();
        let mut on_text = |t: &str| {
            heard.push_str(t);
            true
        };
        let reply = stream(&crate::http::local(), &endpoint, &hi(), &mut on_text).await.unwrap();
        assert_eq!((reply.text.as_str(), heard.as_str()), ("Hello", "Hello"));
    }

    #[tokio::test]
    async fn stopping_ends_the_stream_early() {
        let (_s, endpoint) = server(streamed(sse(&[json!({"content": "a"}), json!({"content": "b"})]))).await;
        let reply = stream(&crate::http::local(), &endpoint, &hi(), &mut |_| false).await.unwrap();
        assert_eq!(reply, Reply { text: "a".into(), calls: vec![], stopped: true });
    }

    /// Reads on to the end. One function, not a closure per test, so the failures below all
    /// run through the same copy of `stream`.
    fn keep(_: &str) -> bool {
        true
    }

    #[tokio::test]
    async fn a_connection_that_drops_mid_answer_is_an_error() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await;
            let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 500\r\n\r\n";
            socket.write_all(format!("{head}{}", chunk(json!({"content": "Dana "}))).as_bytes()).await.unwrap();
        });
        let endpoint = Endpoint { url, key: "k".into() };
        let reply = stream(&crate::http::local(), &endpoint, &hi(), &mut keep).await;
        assert!(matches!(reply, Err(AiError::Model(_))), "{reply:?}");
    }

    #[tokio::test]
    async fn a_refused_request_or_no_server_is_an_error_that_says_why() {
        let client = crate::http::local();
        let (_s, endpoint) = server(ResponseTemplate::new(400).set_body_string("context too long")).await;
        assert_eq!(
            stream(&client, &endpoint, &hi(), &mut keep).await,
            Err(AiError::Model("the model answered 400: context too long".into()))
        );
        let (_s, endpoint) = server(streamed("data: {bad\n\n".into())).await;
        assert!(matches!(stream(&client, &endpoint, &hi(), &mut keep).await, Err(AiError::Model(_))));
        let gone = Endpoint { url: "http://127.0.0.1:9".into(), key: "k".into() };
        assert!(matches!(stream(&client, &gone, &hi(), &mut keep).await, Err(AiError::Model(_))));
    }
}
