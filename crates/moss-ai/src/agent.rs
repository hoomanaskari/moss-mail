//! Ask Moss's loop: the model asks for tools, Moss runs them and hands back what they found,
//! until it answers. Capped in steps and in context, so Gemma 4 E4B fits on a 16 GB Mac (see
//! `docs/research/ask-agent.md`).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};

use moss_core::{Draft, ThreadSummary};
use serde::Serialize;

use serde_json::{json, Value};

use crate::chat::{ChatModel, ChatRequest, Msg};
use crate::tools::{cut, FileSource, ToolOutput, Tools, READ_CHARS};
use crate::AiError;

/// Model calls per question: enough to find some emails and act on each. The last one gets no
/// tools, so it has to answer.
pub const MAX_STEPS: usize = 10;
/// Tokens the model may write per call: a full answer, or a long draft reply.
pub const ANSWER_TOKENS: u32 = 1_024;
/// The Gemma writing model's context (`-c 32768` in `manager::Role::Writer`). Apple
/// Intelligence's is smaller: see [`ChatModel::context_tokens`](crate::chat::ChatModel).
pub const CONTEXT_TOKENS: usize = 32_768;

/// Rough tokens in some characters: a third of the characters, which overcounts English.
pub fn tokens(chars: usize) -> usize {
    chars / 3 + 1
}

/// What the UI hears while an answer is written, on the `moss://ask` event.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AskEvent {
    /// The model is thinking: step 1, 2… Text after this replaces the text before it.
    Step { chat: String, step: usize },
    /// It's looking something up.
    Tool { chat: String, tool: String, label: String },
    /// More of the answer, without the citation marks.
    Text { chat: String, delta: String },
    Done { chat: String, answer: Answer },
    Failed { chat: String, error: AiError },
}

/// A finished answer: the emails it used, to show as cards, the files it took facts from, the
/// drafts it saved, and what it changed.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    pub text: String,
    pub sources: Vec<ThreadSummary>,
    /// The messages it cites (`[m:ID]`), in Ask about this email: ids in the open thread.
    pub messages: Vec<String>,
    /// The files it cites (`[f:ID]`), or failing that the files it read whole.
    pub files: Vec<FileSource>,
    /// Each once, as it last wrote it.
    pub drafts: Vec<Draft>,
    /// Drafts the user asked to send later, by id: when (ms). Send in its card sends it then.
    pub send_at: BTreeMap<String, i64>,
    /// What it changed, in order: "Marked done: Lunch on Friday".
    pub did: Vec<String>,
    /// You stopped it: `text` is what it wrote until then.
    pub stopped: bool,
}

/// Takes `[t:ID]` citation marks out of text as it streams in, and keeps their ids. A space
/// before a mark goes with it, so "182k [t:41]." reads "182k.".
#[derive(Debug, Default)]
pub struct Marks {
    held: String,
    /// Threads cited (`[t:ID]`).
    pub ids: Vec<String>,
    /// Messages cited (`[m:ID]`).
    pub messages: Vec<String>,
    /// Files cited (`[f:ID]`).
    pub files: Vec<String>,
}

impl Marks {
    /// Some more text; returns what to show now.
    pub fn push(&mut self, text: &str) -> String {
        let mut shown = String::new();
        for c in text.chars() {
            self.take(c, &mut shown);
        }
        shown
    }

    /// The end of the text: anything held that wasn't a mark after all.
    pub fn finish(&mut self) -> String {
        std::mem::take(&mut self.held)
    }

    fn take(&mut self, c: char, shown: &mut String) {
        if self.held.is_empty() && c != ' ' && c != '[' {
            shown.push(c);
            return;
        }
        self.held.push(c);
        match mark(&self.held) {
            Some(Mark::Maybe) => {}
            Some(Mark::Done(items)) => {
                for (kind, id) in items {
                    let list = match kind {
                        'm' => &mut self.messages,
                        'f' => &mut self.files,
                        _ => &mut self.ids,
                    };
                    if !list.contains(&id) {
                        list.push(id);
                    }
                }
                self.held.clear();
            }
            None => {
                // Not a mark: show what was held, and look at the last letter afresh.
                self.held.pop();
                shown.push_str(&std::mem::take(&mut self.held));
                self.take(c, shown);
            }
        }
    }
}

/// Where some held text stands as a citation mark.
enum Mark {
    /// Spaces, or the start of a mark.
    Maybe,
    /// Each cited item: `t` (a thread), `m` (a message) or `f` (a file), and its id.
    Done(Vec<(char, String)>),
}

/// Longest mark Moss waits for, in characters.
const MARK_CHARS: usize = 120;

/// Reads held text (spaces, then `[t:ID, t:ID]`): still a mark, a whole one, or `None`.
fn mark(held: &str) -> Option<Mark> {
    let text = held.trim_start_matches(' ');
    if text.is_empty() {
        return Some(Mark::Maybe);
    }
    let inner = text.strip_prefix('[')?;
    let id_char = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    if let Some(list) = inner.strip_suffix(']') {
        let items: Option<Vec<(char, String)>> = list
            .split([',', ';'])
            .map(|item| {
                let (kind, id) = item.trim().split_once(':')?;
                let kind = match kind {
                    "t" => 't',
                    "m" => 'm',
                    "f" => 'f',
                    _ => return None,
                };
                Some((kind, id.to_string())).filter(|_| !id.is_empty() && id.chars().all(id_char))
            })
            .collect();
        return items.map(Mark::Done);
    }
    let allowed = inner.chars().all(|c| id_char(c) || matches!(c, ':' | ',' | ';' | ' '));
    let starts = ["", "t", "m", "f"].contains(&inner) || ["t:", "m:", "f:"].iter().any(|k| inner.starts_with(k));
    (allowed && starts && inner.chars().count() < MARK_CHARS).then_some(Mark::Maybe)
}

/// One question to answer.
pub struct Run<'a> {
    pub chat: &'a str,
    pub model: &'a dyn ChatModel,
    pub tools: &'a dyn Tools,
    /// Files the model can cite before any tool runs: the open thread's, in its own chat.
    pub files: &'a [FileSource],
    /// Threads it can cite before any tool runs: the ones the question names.
    pub threads: &'a [String],
    /// The model reviews each answer against the question before it stands.
    pub review: bool,
    pub emit: &'a (dyn Fn(AskEvent) + Send + Sync),
    pub stop: &'a AtomicBool,
}

/// Answers the question at the end of `messages` (a system message, earlier turns, then the
/// question). Returns the answer and its text as the model wrote it, marks and all, for the
/// chat's history. Emits every step on the way, and `done` or `failed` at the end.
pub async fn run(run: Run<'_>, messages: Vec<Msg>) -> Result<(Answer, String), AiError> {
    let result = steps(&run, messages).await;
    let chat = run.chat.to_string();
    (run.emit)(match &result {
        Ok((answer, _)) => AskEvent::Done { chat, answer: answer.clone() },
        Err(error) => AskEvent::Failed { chat, error: error.clone() },
    });
    result
}

/// Asked once when the model answers with nothing.
const NUDGE: &str = "Answer the question now, in a few short sentences.";

/// How the model checks an answer before the user sees it: a small model asks the user what it
/// could look up, says it did what it didn't, or stops partway.
const REVIEW: &str = "You check an assistant's work in the user's mail app before the user sees it. \
The assistant can search and read mail, act on it (Done, reminders, moving, deleting, blocking) and write emails with write_email, which the user then sends. \
Say whether it did what the user asked. It hasn't if it asked the user for something it could have found with its tools, said it did something none of its tool calls did, acted on only some of the emails it should have, said an email was sent (it can't send: the user presses Send), wrote an email out in its answer, even one it also wrote with write_email (Moss shows that email under the answer, so the answer only says in a sentence what it wrote), wrote a new email where the user asked for a reply (write_email says which it wrote), or stopped partway. \
An answer to a question is done when it answers the question. When it isn't done, say in next, as the user, what it should do now.";
/// What the review sees of each tool's result, in characters.
const SAID_CHARS: usize = 2_000;
/// Reviews per question: each costs a model call.
const REVIEWS: usize = 2;

/// What the model makes of an answer, looking back at the question and the tool calls: what
/// the assistant should do now, or None when it's done (or the review didn't come back).
async fn reviewed(run: &Run<'_>, question: &str, made: &[(String, Value, String)], answer: &str) -> Option<String> {
    let calls: Vec<String> = made.iter().map(|(name, args, said)| format!("- {name} {args}: {said}")).collect();
    let calls = if calls.is_empty() { "(none)".to_string() } else { calls.join("\n") };
    let ask = format!("The user asked:\n{question}\n\nThe assistant's tool calls, with what they said:\n{calls}\n\nIts answer:\n{answer}");
    let parameters = json!({"type": "object", "properties": {"done": {"type": "boolean"}, "next": {"type": "string"}}, "required": ["done", "next"]});
    let tool = ("review", "Whether the assistant did what the user asked, and if not, what it should do now.", parameters);
    let review = crate::chat::decided(run.model, REVIEW, ask, tool, 200).await?;
    let next = review["next"].as_str().unwrap_or_default().trim().to_string();
    (review["done"] == false && !next.is_empty()).then_some(next)
}

/// How the model picks the tools a request needs, when offering them all would fill much of a
/// small context (Apple Intelligence's 8k holds the full set's definitions only with little to
/// spare).
const PICK: &str = "You choose the tools an assistant in the user's mail app needs for a request. \
Pick every tool it may need to do all of what is asked. When unsure whether it needs one, pick it.";
/// Always on offer: most requests start by finding and reading mail, by words or down a list,
/// or finding the person an email is for (without it a small model makes up their address).
const CORE: [&str; 4] = ["search_mail", "read_thread", "list_inbox", "find_people"];

/// Whether tools whose definitions take `spec_tokens` crowd a context of `context` tokens: more
/// than a quarter of it.
fn crowded(spec_tokens: usize, context: usize) -> bool {
    spec_tokens * 4 > context
}

fn name(spec: &Value) -> &str {
    spec["function"]["name"].as_str().unwrap_or_default()
}

/// The tools to offer for `request`: every one when they fit easily, else the ones the model
/// picks for it, beside the core ones. All of them when the pick doesn't come back.
async fn offered(run: &Run<'_>, all: &[Value], request: &str) -> Vec<Value> {
    let spec_chars = Value::Array(all.to_vec()).to_string().chars().count();
    if !crowded(tokens(spec_chars), run.model.context_tokens()) {
        return all.to_vec();
    }
    let names: Vec<&str> = all.iter().map(name).filter(|n| !CORE.contains(n)).collect();
    let menu: Vec<String> = all
        .iter()
        .filter(|s| names.contains(&name(s)))
        .map(|s| {
            let about = s["function"]["description"].as_str().unwrap_or_default();
            format!("- {}: {}", name(s), about.split(". ").next().unwrap_or_default().trim_end_matches('.'))
        })
        .collect();
    let ask = format!("The request:\n{request}\n\nThe tools:\n{}", menu.join("\n"));
    let parameters = json!({"type": "object", "properties": {"tools": {"type": "array", "items": {"type": "string", "enum": names}}}, "required": ["tools"]});
    let picked = crate::chat::decided(run.model, PICK, ask, ("tools", "The tools the assistant needs.", parameters), 200).await;
    let Some(picked) = picked.and_then(|p| p["tools"].as_array().cloned()) else {
        return all.to_vec();
    };
    all.iter().filter(|s| CORE.contains(&name(s)) || picked.iter().any(|p| p == name(s))).cloned().collect()
}

/// `more` added to `specs`, each tool once, in the order of `all`.
fn with(all: &[Value], specs: &[Value], more: &[Value]) -> Vec<Value> {
    all.iter().filter(|s| specs.contains(s) || more.contains(s)).cloned().collect()
}

/// Adds the ids not there yet, in order.
fn add_new(to: &mut Vec<String>, ids: impl IntoIterator<Item = String>) {
    for id in ids {
        if !to.contains(&id) {
            to.push(id);
        }
    }
}

/// Adds the files not there yet, in order.
fn add_files(to: &mut Vec<FileSource>, files: impl IntoIterator<Item = FileSource>) {
    for file in files {
        if !to.contains(&file) {
            to.push(file);
        }
    }
}

async fn steps(run: &Run<'_>, mut messages: Vec<Msg>) -> Result<(Answer, String), AiError> {
    let all = run.tools.specs();
    let mut question = messages.len() - 1;
    let asked = |messages: &[Msg], at: usize| messages[at].to_json()["content"].as_str().unwrap_or_default().to_string();
    let mut specs = offered(run, &all, &asked(&messages, question)).await;
    let context = run.model.context_tokens();
    let budget = |specs: &[Value]| {
        let spec_chars = Value::Array(specs.to_vec()).to_string().chars().count();
        context.saturating_sub(ANSWER_TOKENS as usize + tokens(spec_chars))
    };
    // What one tool's result may hold: two long threads' worth of the context.
    let read = crate::chat::room(READ_CHARS, run.model);
    let (mut seen, mut opened, mut drafts) = (run.threads.to_vec(), vec![], Vec::<Draft>::new());
    let (mut send_at, mut did) = (BTreeMap::new(), vec![]);
    let (mut seen_files, mut read_files): (Vec<FileSource>, Vec<FileSource>) = (run.files.to_vec(), vec![]);
    let (mut raw, mut shown, mut stopped, mut nudged) = (String::new(), String::new(), false, false);
    // Answers reviewed so far, and the calls made, with what they said.
    let mut reviews = 0;
    let mut made: Vec<(String, Value, String)> = vec![];
    let (mut cited, mut cited_messages, mut cited_files) = (vec![], vec![], vec![]);
    for step in 1..=MAX_STEPS {
        if run.stop.load(Ordering::Relaxed) {
            stopped = true;
            break;
        }
        (run.emit)(AskEvent::Step { chat: run.chat.into(), step });
        fit(&mut messages, &mut question, budget(&specs));
        let last = step == MAX_STEPS;
        let tools = if last { vec![] } else { specs.clone() };
        let request = ChatRequest { tools, ..ChatRequest::new(messages.clone(), ANSWER_TOKENS) };
        let mut marks = Marks::default();
        let mut text = String::new();
        let show = |piece: String, text: &mut String| {
            if !piece.is_empty() {
                text.push_str(&piece);
                (run.emit)(AskEvent::Text { chat: run.chat.into(), delta: piece });
            }
        };
        let reply = run
            .model
            .chat(&request, &mut |piece: &str| {
                show(marks.push(piece), &mut text);
                !run.stop.load(Ordering::Relaxed)
            })
            .await?;
        show(marks.finish(), &mut text);
        if reply.stopped || reply.calls.is_empty() || last {
            if !reply.stopped && text.trim().is_empty() && !last && !nudged {
                nudged = true;
                messages.push(Msg::Assistant { text: String::new(), calls: vec![] });
                messages.push(Msg::User(NUDGE.into()));
                continue;
            }
            if run.review && !reply.stopped && !last && reviews < REVIEWS {
                reviews += 1;
                if let Some(next) = reviewed(run, &asked(&messages, question), &made, &text).await {
                    // What's left may need tools the question's pick didn't.
                    let more = offered(run, &all, &next).await;
                    specs = with(&all, &specs, &more);
                    messages.push(Msg::Assistant { text: reply.text.clone(), calls: vec![] });
                    messages.push(Msg::User(next));
                    continue;
                }
            }
            // A tool call written out as text, when the model had no tools left: not for the user.
            let text = text.split("<|tool_call").next().unwrap_or_default().to_string();
            (raw, shown, stopped) = (reply.text, text, reply.stopped);
            (cited, cited_messages, cited_files) = (marks.ids, marks.messages, marks.files);
            break;
        }
        messages.push(Msg::Assistant { text: reply.text.clone(), calls: reply.calls.clone() });
        for call in &reply.calls {
            let arguments = if call.arguments.trim().is_empty() { "{}" } else { &call.arguments };
            let parsed = serde_json::from_str::<Value>(arguments);
            // A small model may call the same thing over and over: once is enough.
            if let Some((.., said)) = parsed.as_ref().ok().and_then(|args| made.iter().find(|(n, a, _)| n == &call.name && a == args)) {
                let text = format!("You did that already, with the same arguments. Don't do it again: answer me in a sentence. It said: {said}");
                messages.push(Msg::Tool { id: call.id.clone(), text: cut(&text, read) });
                continue;
            }
            let output = match parsed {
                Ok(args) => {
                    let label = run.tools.label(&call.name, &args);
                    (run.emit)(AskEvent::Tool { chat: run.chat.into(), tool: call.name.clone(), label });
                    let output = run.tools.call(&call.name, &args).await;
                    // Enough for the review to see a list's every email.
                    made.push((call.name.clone(), args, cut(&output.text, SAID_CHARS)));
                    output
                }
                Err(e) => ToolOutput { text: format!("Couldn’t read the arguments: {e}"), ..Default::default() },
            };
            add_new(&mut seen, output.threads);
            add_new(&mut opened, output.opened);
            add_files(&mut seen_files, output.files);
            add_files(&mut read_files, output.read_files);
            if let Some(draft) = output.draft {
                if let Some(at) = output.send_at {
                    send_at.insert(draft.id.clone(), at);
                }
                drafts.retain(|d| d.id != draft.id);
                drafts.push(draft);
            }
            did.extend(output.did);
            messages.push(Msg::Tool { id: call.id.clone(), text: cut(&output.text, read) });
        }
    }
    let mut ids: Vec<String> = cited.into_iter().filter(|id| seen.contains(id)).collect();
    if ids.is_empty() {
        ids = opened;
    }
    let mut files: Vec<FileSource> = vec![];
    add_files(&mut files, cited_files.iter().filter_map(|id| seen_files.iter().find(|f| &f.id() == id).cloned()));
    if files.is_empty() {
        files = read_files;
    }
    let answer = Answer { text: shown.trim().to_string(), sources: run.tools.sources(&ids), messages: cited_messages, files, drafts, send_at, did, stopped };
    Ok((answer, raw))
}

/// Keeps a chat inside the context: drops the oldest earlier turns first (never the system
/// message or the question at `question`), then shortens the oldest tool results.
pub fn fit(messages: &mut Vec<Msg>, question: &mut usize, budget_tokens: usize) {
    let size = |m: &[Msg]| m.iter().map(|m| tokens(m.chars()) + 4).sum::<usize>();
    while size(messages) > budget_tokens && *question > 1 {
        messages.remove(1);
        *question -= 1;
    }
    for n in *question + 1..messages.len() {
        if size(messages) <= budget_tokens {
            return;
        }
        if let Msg::Tool { text, .. } = &mut messages[n] {
            *text = "(Left out to save room.)".into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_mail::*;
    use crate::tools::{match_line, Mail, MailTools};
    use std::sync::Mutex;

    fn marks(pieces: &[&str]) -> (String, Vec<String>) {
        let mut m = Marks::default();
        let mut shown: String = pieces.iter().map(|p| m.push(p)).collect();
        shown.push_str(&m.finish());
        (shown, m.ids)
    }

    #[test]
    fn citation_marks_are_taken_out_even_split_across_pieces() {
        assert_eq!(marks(&["Dana said 182k [t:41]."]), ("Dana said 182k.".into(), vec!["41".into()]));
        assert_eq!(marks(&["Dana said 182k [", "t:4", "1] and Sam ", "approved [t:41, t:57]"]).0, "Dana said 182k and Sam approved");
        assert_eq!(marks(&["a [t:41][t:57]; b"]).1, ["41", "57"]);
        assert_eq!(marks(&["see [t:41; t:57]"]).1, ["41", "57"]);
        assert_eq!(marks(&["x[t:41]"]), ("x".into(), vec!["41".into()]));
    }

    #[test]
    fn message_marks_are_kept_apart_from_thread_marks() {
        let mut m = Marks::default();
        let shown = m.push("Dana agreed [m:m1, t:41] and Sam too [m:m2][m:m1].");
        assert_eq!(shown + &m.finish(), "Dana agreed and Sam too.");
        assert_eq!((m.ids, m.messages), (vec!["41".to_string()], vec!["m1".to_string(), "m2".to_string()]));
        assert_eq!(marks(&["[m:]"]).0, "[m:]");
        assert_eq!(marks(&["[x:1]"]).0, "[x:1]");
        assert_eq!(marks(&["[t:1, x:2]"]), ("[t:1, x:2]".into(), vec![]), "one unknown kind and it isn't a mark");
    }

    #[test]
    fn other_brackets_and_spaces_stay() {
        assert_eq!(marks(&["a [b] c"]), ("a [b] c".into(), vec![]));
        assert_eq!(marks(&["a  b"]).0, "a  b");
        assert_eq!(marks(&["list:\n[t:4"]).0, "list:\n[t:4", "an unfinished mark at the end is shown");
        assert_eq!(marks(&["[t:]"]).0, "[t:]");
        assert_eq!(marks(&["[t:4 2]"]).0, "[t:4 2]");
        assert_eq!(marks(&["a [ [t:41]"]), ("a [".into(), vec!["41".into()]));
        assert_eq!(marks(&["[t:1-a_B]"]).1, ["1-a_B"]);
        assert_eq!(marks(&[&format!("[t:{}", "9".repeat(200))]).0.len(), 203, "too long to be a mark");
    }

    #[test]
    fn a_chat_too_long_loses_old_turns_then_old_tool_results() {
        let user = |t: &str| Msg::User(t.into());
        let said = |t: &str| Msg::Assistant { text: t.into(), calls: vec![] };
        let long = "x".repeat(300);
        let mut messages = vec![
            Msg::System("sys".into()),
            user(&long),
            said(&long),
            user("question"),
            Msg::Assistant { text: String::new(), calls: vec![crate::chat::ToolCall::default()] },
            Msg::Tool { id: "c0".into(), text: long.clone() },
            Msg::Tool { id: "c1".into(), text: long.clone() },
        ];
        let mut question = 3;
        let size = |m: &[Msg]| m.iter().map(|m| tokens(m.chars()) + 4).sum::<usize>();
        let all = size(&messages);
        fit(&mut messages, &mut question, all);
        assert_eq!(messages.len(), 7, "it fits: nothing changes");

        fit(&mut messages, &mut question, all - 150);
        assert_eq!(question, 1);
        assert_eq!(messages[0], Msg::System("sys".into()));
        assert_eq!(messages[1], user("question"));
        assert_eq!(messages.len(), 5);

        let left = size(&messages);
        fit(&mut messages, &mut question, left - 50);
        assert_eq!(messages[3], Msg::Tool { id: "c0".into(), text: "(Left out to save room.)".into() });
        assert_eq!(messages[4], Msg::Tool { id: "c1".into(), text: long.clone() });

        fit(&mut messages, &mut question, 1);
        assert_eq!(messages.len(), 5, "the system message, the question and the calls always stay");
        assert_eq!(messages[4], Msg::Tool { id: "c1".into(), text: "(Left out to save room.)".into() });
    }

    struct Heard(Mutex<Vec<AskEvent>>);

    impl Heard {
        fn new() -> Heard {
            Heard(Mutex::new(vec![]))
        }
        fn emit(&self) -> impl Fn(AskEvent) + Send + Sync + '_ {
            |e| self.0.lock().unwrap().push(e)
        }
        fn events(&self) -> Vec<AskEvent> {
            self.0.lock().unwrap().clone()
        }
        fn text(&self) -> String {
            self.events()
                .into_iter()
                .filter_map(|e| match e {
                    AskEvent::Text { delta, .. } => Some(delta),
                    _ => None,
                })
                .collect()
        }
    }

    fn question(q: &str) -> Vec<Msg> {
        vec![Msg::System("You are Moss.".into()), Msg::User(q.into())]
    }

    async fn ask(model: &ScriptedModel, mail: &FakeMail, heard: &Heard, q: &str) -> Result<(Answer, String), AiError> {
        let tools = MailTools::new(mail, NOV_1);
        let stop = AtomicBool::new(false);
        let emit = heard.emit();
        run(Run { chat: "c", model, tools: &tools, files: &[], threads: &[], review: false, emit: &emit, stop: &stop }, question(q)).await
    }

    fn names(request: &ChatRequest) -> Vec<String> {
        request.tools.iter().map(|t| name(t).to_string()).collect()
    }

    fn user_text(request: &ChatRequest) -> String {
        request.messages.iter().filter_map(|m| if let Msg::User(t) = m { Some(t.clone()) } else { None }).collect::<Vec<_>>().join("\n")
    }

    /// Asks with the app's actions, of a model whose context the full set of tools crowds.
    async fn ask_crowded(steps: Vec<Step>, review: bool) -> (ScriptedModel, FakeActions, Vec<Value>) {
        let mail = FakeMail::new();
        let actions = FakeActions::default();
        let tools = MailTools::new(&mail, NOV_1).with_actions(&actions);
        let all = tools.specs();
        let spec_tokens = tokens(Value::Array(all.clone()).to_string().chars().count());
        assert!(spec_tokens > 600, "enough tools to crowd a small context: {spec_tokens}");
        let model = ScriptedModel::new(steps).with_context(3 * spec_tokens);
        let heard = Heard::new();
        let (emit, stop) = (heard.emit(), AtomicBool::new(false));
        let run_ = Run { chat: "c", model: &model, tools: &tools, files: &[], threads: &[], review, emit: &emit, stop: &stop };
        run(run_, question("Mark the Q4 budget email done")).await.unwrap();
        (model, actions, all)
    }

    #[test]
    fn tools_crowd_a_context_when_they_would_fill_more_than_a_quarter() {
        assert!(!crowded(2_048, 8_192));
        assert!(crowded(2_049, 8_192));
        assert!(!crowded(4_000, CONTEXT_TOKENS));
    }

    #[tokio::test]
    async fn a_model_with_little_room_is_offered_the_tools_it_picks() {
        let (model, actions, _) = ask_crowded(
            vec![
                Step::Call(vec![("tools", r#"{"tools":["mark_done"]}"#)]),
                Step::Call(vec![("mark_done", r#"{"thread_id":"41"}"#)]),
                Step::Say(vec!["Marked it done."]),
            ],
            false,
        )
        .await;
        let requests = model.requests();
        assert!(requests[0].must_call);
        assert_eq!(names(&requests[0]), ["tools"]);
        let menu = user_text(&requests[0]);
        assert!(menu.contains("Mark the Q4 budget email done"), "{menu}");
        assert!(menu.contains("- mark_done") && menu.contains("- write_email"), "{menu}");
        assert!(!menu.contains("- search_mail") && !menu.contains("- list_inbox") && !menu.contains("- find_people"), "finding mail and people is always on offer: {menu}");
        // The pick, beside finding and reading mail, for every step of the answer.
        assert_eq!(names(&requests[1]), ["search_mail", "read_thread", "find_people", "mark_done", "list_inbox"]);
        assert_eq!(names(&requests[2]), names(&requests[1]));
        assert_eq!(actions.acted.lock().unwrap()[0].0, "mark_done");
    }

    #[tokio::test]
    async fn a_pick_that_doesnt_come_back_offers_every_tool() {
        let (model, _, all) = ask_crowded(vec![Step::Say(vec!["Hmm."]), Step::Say(vec!["Done."])], false).await;
        assert_eq!(model.requests()[1].tools, all);
    }

    #[tokio::test]
    async fn what_a_review_says_is_left_gets_the_tools_it_needs() {
        let (model, actions, _) = ask_crowded(
            vec![
                Step::Call(vec![("tools", r#"{"tools":["mark_done"]}"#)]),
                Step::Call(vec![("mark_done", r#"{"thread_id":"41"}"#)]),
                Step::Say(vec!["Marked it done."]),
                Step::Call(vec![("review", r#"{"done":false,"next":"Now remind me about it tomorrow."}"#)]),
                Step::Call(vec![("tools", r#"{"tools":["remind_me"]}"#)]),
                Step::Call(vec![("remind_me", r#"{"thread_id":"41","until":"2026-11-02T09:00:00Z"}"#)]),
                Step::Say(vec!["Reminder set too."]),
                Step::Call(vec![("review", r#"{"done":true,"next":""}"#)]),
            ],
            true,
        )
        .await;
        let requests = model.requests();
        assert!(user_text(&requests[4]).contains("Now remind me about it tomorrow."));
        assert_eq!(names(&requests[5]), ["search_mail", "read_thread", "find_people", "mark_done", "remind_me", "list_inbox"]);
        let acted: Vec<String> = actions.acted.lock().unwrap().iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(acted, ["mark_done", "remind_me"]);
    }

    #[tokio::test]
    async fn a_tools_result_is_cut_to_the_models_room() {
        let mut mail = FakeMail::new();
        // Two long messages: whole within the Gemma writer's room, not within an 8k model's.
        for m in &mut mail.threads[0].messages {
            m.paragraphs = vec!["word ".repeat(1_600)];
        }
        let tools = MailTools::new(&mail, NOV_1);
        let model = ScriptedModel::new(vec![Step::Call(vec![("read_thread", r#"{"thread_id":"41"}"#)]), Step::Say(vec!["Long."])]).with_context(8_192);
        let heard = Heard::new();
        let (emit, stop) = (heard.emit(), AtomicBool::new(false));
        run(Run { chat: "c", model: &model, tools: &tools, files: &[], threads: &[], review: false, emit: &emit, stop: &stop }, question("Read it")).await.unwrap();
        let read = model.requests()[1].messages.iter().find_map(|m| if let Msg::Tool { text, .. } = m { Some(text.chars().count()) } else { None });
        let room = crate::chat::room(READ_CHARS, &model);
        assert!(read.is_some_and(|n| n <= room && n > room - 10), "{read:?} of {room}");
    }

    #[tokio::test]
    async fn looks_things_up_then_answers_citing_the_emails_it_used() {
        let model = ScriptedModel::new(vec![
            Step::Call(vec![("search_mail", r#"{"query":"Q4 budget"}"#)]),
            Step::Call(vec![("read_thread", r#"{"thread_id":"41"}"#)]),
            Step::Say(vec!["Dana said 182k [t:41", "]. Sam approved it [t:57]. Ignore [t:1]."]),
        ]);
        let mail = FakeMail::new();
        let heard = Heard::new();
        let (answer, raw) = ask(&model, &mail, &heard, "What did Dana say?").await.unwrap();
        assert_eq!(answer.text, "Dana said 182k. Sam approved it. Ignore.");
        assert_eq!(raw, "Dana said 182k [t:41]. Sam approved it [t:57]. Ignore [t:1].");
        let sources: Vec<String> = answer.sources.iter().map(|t| t.id.clone()).collect();
        assert_eq!(sources, ["41", "57"], "only ids a tool returned");
        assert!(answer.drafts.is_empty() && !answer.stopped && answer.messages.is_empty());
        assert_eq!(heard.text(), answer.text);

        let kinds: Vec<String> = heard
            .events()
            .iter()
            .map(|e| match e {
                AskEvent::Step { step, .. } => format!("step {step}"),
                AskEvent::Tool { label, .. } => label.clone(),
                AskEvent::Text { .. } => "text".into(),
                AskEvent::Done { .. } | AskEvent::Failed { .. } => "end".into(),
            })
            .filter(|k| k != "text")
            .collect();
        assert_eq!(kinds, ["step 1", "Searching mail for “Q4 budget”", "step 2", "Reading an email", "step 3", "end"]);
        assert_eq!(heard.events().last(), Some(&AskEvent::Done { chat: "c".into(), answer: answer.clone() }));

        // The model saw each tool's result, under the call it made.
        let requests = model.requests();
        let last = &requests[2].messages;
        assert!(matches!(&last[2], Msg::Assistant { calls, .. } if calls[0].name == "search_mail"));
        assert!(matches!(&last[3], Msg::Tool { id, text } if id == "c0" && text.starts_with("[t:41]")));
        assert!(matches!(&last[5], Msg::Tool { text, .. } if text.starts_with("Thread [t:41]")));
        assert!(requests.iter().all(|r| r.max_tokens == ANSWER_TOKENS && r.tools.len() == crate::tools::ALL.len()));
    }

    #[test]
    fn file_marks_are_kept_apart_too() {
        let mut m = Marks::default();
        let shown = m.push("On 12 December [t:41, f:4406-2] at 10:00 [f:4406-2][f:9-0].");
        assert_eq!(shown + &m.finish(), "On 12 December at 10:00.");
        assert_eq!((m.ids, m.files), (vec!["41".to_string()], vec!["4406-2".to_string(), "9-0".to_string()]));
        assert_eq!(marks(&["[f:]"]).0, "[f:]");
        assert_eq!(marks(&["[f:1", "-2]"]).0, "");
    }

    /// The budget thread with its spreadsheet read, and the file listed.
    fn with_a_read_file() -> FakeMail {
        let mut mail = FakeMail::new();
        mail.threads[0].messages[0].attachments[0].text = Some("Total: 182k".into());
        mail
    }

    #[tokio::test]
    async fn the_files_it_cites_come_back_with_the_answer() {
        let model = ScriptedModel::new(vec![
            Step::Call(vec![("read_thread", r#"{"thread_id": "41"}"#)]),
            Step::Say(vec!["The total is 182k [t:41, f:m1-0]. Also [f:m9-9]."]),
        ]);
        let (answer, _) = ask(&model, &with_a_read_file(), &Heard::new(), "What's the total?").await.unwrap();
        assert_eq!(answer.text, "The total is 182k. Also.");
        let budget = FileSource { thread_id: "41".into(), message_id: "m1".into(), index: 0, name: "budget-q4.xlsx".into() };
        assert_eq!(answer.files, [budget], "only files a tool showed");
        // The thread it read had the file's text in it.
        let requests = model.requests();
        assert!(matches!(&requests[1].messages[3], Msg::Tool { text, .. } if text.contains("Inside the attachment budget-q4.xlsx [f:m1-0]:\nTotal: 182k")));
    }

    #[tokio::test]
    async fn a_file_it_read_is_shown_even_when_it_forgets_to_cite_it() {
        let model = ScriptedModel::new(vec![
            Step::Call(vec![("read_attachment", r#"{"file_id": "m1-0"}"#), ("read_attachment", r#"{"file_id": "[f:m1-0]"}"#)]),
            Step::Say(vec!["It's 182k."]),
        ]);
        let (answer, _) = ask(&model, &with_a_read_file(), &Heard::new(), "What's the total?").await.unwrap();
        let names: Vec<&str> = answer.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["budget-q4.xlsx"], "once");
        assert_eq!(answer.sources.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["41"]);
        // Listing files isn't reading them.
        let model = ScriptedModel::new(vec![Step::Call(vec![("list_attachments", "{}")]), Step::Say(vec!["One file."])]);
        let (answer, _) = ask(&model, &with_a_read_file(), &Heard::new(), "Files?").await.unwrap();
        assert!(answer.files.is_empty());
    }

    #[tokio::test]
    async fn the_messages_it_cites_come_back_with_the_answer() {
        let model = ScriptedModel::new(vec![Step::Say(vec!["182k [m:m1], confirmed [m:m2]."])]);
        let (answer, _) = ask(&model, &FakeMail::new(), &Heard::new(), "How much?").await.unwrap();
        assert_eq!(answer.text, "182k, confirmed.");
        assert_eq!(answer.messages, ["m1", "m2"]);
    }

    #[tokio::test]
    async fn without_citations_the_threads_it_read_are_the_sources() {
        let model = ScriptedModel::new(vec![
            Step::Call(vec![("search_mail", r#"{"query":"budget"}"#), ("read_thread", r#"{"thread_id":"57"}"#)]),
            Step::Say(vec!["Sam approved it."]),
        ]);
        let (answer, _) = ask(&model, &FakeMail::new(), &Heard::new(), "Did Sam approve?").await.unwrap();
        let sources: Vec<String> = answer.sources.iter().map(|t| t.id.clone()).collect();
        assert_eq!(sources, ["57"]);
    }

    #[tokio::test]
    async fn a_drafted_reply_comes_back_for_you_to_check() {
        let model = ScriptedModel::new(vec![
            Step::Call(vec![("draft_reply", r#"{"thread_id":"41","body":"I confirm 182k."}"#)]),
            Step::Say(vec!["I drafted a reply to Dana [t:41], like Sam's [t:57]."]),
        ]);
        let mail = FakeMail::new();
        let (answer, _) = ask(&model, &mail, &Heard::new(), "Reply yes").await.unwrap();
        assert_eq!(answer.drafts.len(), 1);
        assert_eq!(answer.drafts[0], mail.saved.lock().unwrap()[0]);
        let sources: Vec<String> = answer.sources.iter().map(|t| t.id.clone()).collect();
        assert_eq!(sources, ["41"], "57 is real, but no tool showed it: made up");
    }

    #[tokio::test]
    async fn arguments_it_cannot_read_go_back_to_the_model() {
        let model = ScriptedModel::new(vec![
            Step::Call(vec![("search_mail", "{query"), ("find_people", "")]),
            Step::Say(vec!["Sorry."]),
        ]);
        let heard = Heard::new();
        ask(&model, &FakeMail::new(), &heard, "?").await.unwrap();
        let second = &model.requests()[1].messages;
        assert!(matches!(&second[3], Msg::Tool { text, .. } if text.starts_with("Couldn’t read the arguments:")));
        assert!(matches!(&second[4], Msg::Tool { text, .. } if text == "find_people needs a query."), "no arguments is {{}}");
        let tools = heard.events().iter().filter(|e| matches!(e, AskEvent::Tool { .. })).count();
        assert_eq!(tools, 1, "only the call that ran");
    }

    #[tokio::test]
    async fn the_last_step_gets_no_tools_so_it_has_to_answer() {
        let mut steps: Vec<Step> = (0..MAX_STEPS - 1).map(|_| Step::Call(vec![("find_people", r#"{"query":"dana"}"#)])).collect();
        steps.push(Step::Call(vec![("find_people", r#"{"query":"dana"}"#)]));
        let model = ScriptedModel::new(steps);
        let (answer, _) = ask(&model, &FakeMail::new(), &Heard::new(), "Who?").await.unwrap();
        let requests = model.requests();
        assert_eq!(requests.len(), MAX_STEPS);
        assert!(requests[..MAX_STEPS - 1].iter().all(|r| !r.tools.is_empty()));
        assert!(requests[MAX_STEPS - 1].tools.is_empty());
        assert_eq!(answer.text, "", "still calling tools at the end: no answer");
    }

    #[tokio::test]
    async fn an_empty_answer_gets_one_more_try() {
        let model = ScriptedModel::new(vec![Step::Say(vec![" "]), Step::Say(vec!["Here."])]);
        let (answer, _) = ask(&model, &FakeMail::new(), &Heard::new(), "?").await.unwrap();
        assert_eq!(answer.text, "Here.");
        let second = &model.requests()[1].messages;
        assert_eq!(second.last(), Some(&Msg::User("Answer the question now, in a few short sentences.".into())));

        let model = ScriptedModel::new(vec![Step::Say(vec![""]), Step::Say(vec![""])]);
        let (answer, _) = ask(&model, &FakeMail::new(), &Heard::new(), "?").await.unwrap();
        assert_eq!((answer.text.as_str(), model.requests().len()), ("", 2), "only once");
    }

    #[tokio::test]
    async fn a_failed_model_is_an_error_the_ui_hears() {
        let model = ScriptedModel::new(vec![Step::Fail(AiError::NotReady)]);
        let heard = Heard::new();
        assert_eq!(ask(&model, &FakeMail::new(), &heard, "?").await, Err(AiError::NotReady));
        assert_eq!(heard.events().last(), Some(&AskEvent::Failed { chat: "c".into(), error: AiError::NotReady }));
    }

    #[tokio::test]
    async fn stopping_keeps_what_was_written_so_far() {
        let model = ScriptedModel::new(vec![Step::Say(vec!["Dana ", "said ", "more"])]);
        let mail = FakeMail::new();
        let tools = MailTools::new(&mail, NOV_1);
        let stop = AtomicBool::new(false);
        let heard = Heard::new();
        let emit = |e: AskEvent| {
            if matches!(&e, AskEvent::Text { delta, .. } if delta.ends_with("said")) {
                stop.store(true, Ordering::Relaxed);
            }
            heard.0.lock().unwrap().push(e);
        };
        let (answer, raw) = run(Run { chat: "c", model: &model, tools: &tools, files: &[], threads: &[], review: false, emit: &emit, stop: &stop }, question("?")).await.unwrap();
        assert_eq!((answer.text.as_str(), answer.stopped, raw.as_str()), ("Dana said", true, "Dana said "));
        assert!(matches!(heard.events().last(), Some(AskEvent::Done { answer, .. }) if answer.stopped));
    }

    #[tokio::test]
    async fn stopped_between_steps_it_asks_the_model_nothing_more() {
        let model = ScriptedModel::new(vec![Step::Call(vec![("find_people", r#"{"query":"dana"}"#)]), Step::Say(vec!["x"])]);
        let mail = FakeMail::new();
        let tools = MailTools::new(&mail, NOV_1);
        let stop = AtomicBool::new(false);
        let emit = |e: AskEvent| {
            if matches!(e, AskEvent::Tool { .. }) {
                stop.store(true, Ordering::Relaxed);
            }
        };
        let (answer, _) = run(Run { chat: "c", model: &model, tools: &tools, files: &[], threads: &[], review: false, emit: &emit, stop: &stop }, question("?")).await.unwrap();
        assert!(answer.stopped);
        assert_eq!(model.requests().len(), 1);
    }

    #[tokio::test]
    async fn every_request_fits_the_context() {
        // Each old turn is bigger than the whole context.
        let long = "word ".repeat(CONTEXT_TOKENS);
        let mut messages = question("?");
        messages.insert(1, Msg::User(long.clone()));
        messages.insert(2, Msg::Assistant { text: long, calls: vec![] });
        let model = ScriptedModel::new(vec![Step::Say(vec!["ok"])]);
        let mail = FakeMail::new();
        let tools = MailTools::new(&mail, NOV_1);
        let stop = AtomicBool::new(false);
        run(Run { chat: "c", model: &model, tools: &tools, files: &[], threads: &[], review: false, emit: &|_| {}, stop: &stop }, messages).await.unwrap();
        let sent = &model.requests()[0].messages;
        assert_eq!(sent.len(), 2, "the old turns went");
        assert_eq!(sent[1], Msg::User("?".into()));
    }

    async fn act(model: &ScriptedModel, q: &str) -> (Answer, Vec<String>) {
        let mail = FakeMail::new();
        let actions = FakeActions::default();
        let tools = MailTools::new(&mail, NOV_1).with_actions(&actions);
        let stop = AtomicBool::new(false);
        let heard = Heard::new();
        let emit = heard.emit();
        let (answer, _) = run(Run { chat: "c", model, tools: &tools, files: &[], threads: &[], review: false, emit: &emit, stop: &stop }, question(q)).await.unwrap();
        let labels = heard.events().into_iter().filter_map(|e| match e {
            AskEvent::Tool { label, .. } => Some(label),
            _ => None,
        });
        (answer, labels.collect())
    }

    #[tokio::test]
    async fn an_email_from_the_chat_is_written_as_quick_reply_writes_it() {
        let model = ScriptedModel::new(vec![
            Step::Call(vec![("write_email", r#"{"reply_to_thread":"41","body":"I'll text you soon."}"#)]),
            // Quick reply's writer, from the gist: greeting, the user's voice, a sign-off to fit the signature.
            Step::Say(vec!["Hi Dana,\n\nThanks for this. I'll text you soon.\n\nCheers,"]),
            Step::Say(vec!["I drafted a reply to Dana."]),
        ]);
        let mail = FakeMail::new();
        let actions = FakeActions::default();
        let tools = MailTools::new(&mail, NOV_1).with_actions(&actions).asked("Tell Dana I'll text her soon").writing_with(&model);
        let stop = AtomicBool::new(false);
        let (answer, _) = run(Run { chat: "c", model: &model, tools: &tools, files: &[], threads: &[], review: false, emit: &|_| {}, stop: &stop }, question("Tell Dana I'll text her soon")).await.unwrap();
        assert_eq!(answer.drafts.len(), 1);
        assert_eq!(answer.drafts[0].body, "Hi Dana,\n\nThanks for this. I'll text you soon.\n\nCheers,");
        let acted = actions.acted.lock().unwrap().clone();
        assert_eq!(acted[0].0, "write_email");
        assert_eq!(acted[1].0, "update_draft", "the draft gets the written email");
        assert_eq!(acted[1].1["draft_id"], "ready");
        // The model hears what write_email said (which kind of email, not to write it out), not
        // the written email: a small model given it pastes it into its answer.
        let Msg::Tool { text, .. } = model.requests()[2].messages.last().unwrap().clone() else { panic!("a tool result") };
        assert_eq!(text, "waiting for the user to press Send");
        let writing = &model.requests()[1];
        assert!(writing.tools.is_empty());
        let Msg::User(asked) = writing.messages.last().unwrap() else { panic!("a question") };
        assert!(asked.contains("I'll text you soon."), "the gist is the note: {asked}");
        assert!(asked.contains("The final Q4 budget is 182k"), "it reads the conversation");
        // Quick reply's own prompt, from what the app says about the user: their voice, the signature.
        let context = mail.reply_context(&answer.drafts[0]).unwrap();
        let job = crate::reply::Job::Instructions { instructions: "I'll text you soon.".into() };
        assert_eq!(writing.messages[0], Msg::System(crate::reply::prompt(&job, "", &context).unwrap().0));
        // No words at all: the note is what the user asked.
        let model = ScriptedModel::new(vec![Step::Say(vec!["Hi Dana,\n\nYes, 182k works.\n\nCheers,"])]);
        let tools = MailTools::new(&mail, NOV_1).with_actions(&actions).asked("Reply yes to Dana").writing_with(&model);
        tools.call("write_email", &serde_json::json!({"reply_to_thread": "41"})).await;
        let Msg::User(asked) = model.requests()[0].messages.last().unwrap().clone() else { panic!("a question") };
        assert!(asked.contains("Reply yes to Dana"), "{asked}");
        // A forward, with words or none, or a writer that fails, leaves the email as it came:
        // Quick reply writes replies to the sender, and a forward goes to someone else.
        let model = ScriptedModel::new(vec![Step::Fail(AiError::NotReady)]);
        let tools = MailTools::new(&mail, NOV_1).with_actions(&actions).asked("Forward it to Sam").writing_with(&model);
        tools.call("write_email", &serde_json::json!({"forward_thread": "41", "to": ["sam@x.co"]})).await;
        tools.call("write_email", &serde_json::json!({"forward_thread": "41", "to": ["sam@x.co"], "body": "Sam, see below."})).await;
        tools.call("write_email", &serde_json::json!({"reply_to_thread": "41", "body": "Yes."})).await;
        let acted = actions.acted.lock().unwrap().clone();
        assert_eq!(acted.iter().filter(|(n, _)| n == "update_draft").count(), 2, "only the two written");
        assert!(model.requests().len() == 1, "the forward asked nothing");
    }

    async fn picking(model: &ScriptedModel, mail: &FakeMail, actions: &FakeActions, asked: &str, name: &str, args: serde_json::Value) -> ToolOutput {
        let tools = MailTools::new(mail, NOV_1).with_actions(actions).asked(asked).writing_with(model);
        tools.call(name, &args).await
    }

    #[tokio::test]
    async fn the_model_picks_the_email_an_action_is_about_from_what_search_finds() {
        let mail = FakeMail::new();
        let actions = FakeActions::schema(&["thread_id"]);
        // No email named: Moss searches what the user asked, and the model picks.
        let model = ScriptedModel::new(vec![Step::Call(vec![("pick", r#"{"thread_id": "57"}"#)])]);
        let done = picking(&model, &mail, &actions, "Mark Sam's budget approval done", "mark_done", serde_json::json!({})).await;
        assert_eq!(actions.acted.lock().unwrap()[0].1["thread_id"], "57");
        assert_eq!(done.did.as_deref(), Some("Marked done: 57"));
        let asked = &model.requests()[0];
        assert!(asked.must_call);
        assert_eq!(asked.tools[0]["function"]["parameters"]["properties"]["thread_id"]["enum"], serde_json::json!(["41", "57", "none"]), "only the emails found, or none");
        let Msg::User(question) = asked.messages.last().unwrap() else { panic!("a question") };
        assert!(question.starts_with("The user asked: Mark Sam's budget approval done\nThe action: mark_done {}\nThe emails:\n[t:41] “Q4 budget” from Dana Lee\n[t:57]"), "{question}");
        // Something that isn't an id (a subject): the same.
        let model = ScriptedModel::new(vec![Step::Call(vec![("pick", r#"{"thread_id": "41"}"#)])]);
        picking(&model, &mail, &actions, "Reply to the budget email", "write_email", serde_json::json!({"reply_to_thread": "Q4 budget", "body": "Yes"})).await;
        assert_eq!(actions.acted.lock().unwrap()[1].1["reply_to_thread"], "41");
        // The model can't tell: the choices go back, and nothing is done.
        let model = ScriptedModel::new(vec![Step::Call(vec![("pick", r#"{"thread_id": "none"}"#)])]);
        let unsure = picking(&model, &mail, &actions, "Mark the budget done", "mark_done", serde_json::json!({})).await;
        assert_eq!(unsure.text, "Which email? These may be it: [t:41] “Q4 budget” from Dana Lee; [t:57] “Re: Q4 budget” from Sam Ortiz. Call again with the thread_id of the one meant.");
        assert_eq!(unsure.threads, ["41", "57"]);
        assert_eq!(actions.acted.lock().unwrap().len(), 3, "the reply, written, and nothing since");
        // Nothing found: look first.
        let model = ScriptedModel::new(vec![]);
        let none = picking(&model, &mail, &actions, "Mark the zebra one done", "mark_done", serde_json::json!({"thread_id": "zebra"})).await;
        assert!(none.text.starts_with("There's no email “zebra”."), "{}", none.text);
        // With no model to ask, the choices go back.
        let alone = MailTools::new(&mail, NOV_1).with_actions(&actions).asked("Mark the budget done");
        assert!(alone.call("mark_done", &serde_json::json!({})).await.text.starts_with("Which email? These may be it:"));
        // A real id goes straight through, with no question to the model.
        let model = ScriptedModel::new(vec![]);
        picking(&model, &mail, &actions, "Mark it done", "mark_done", serde_json::json!({"thread_id": "41"})).await;
        assert!(model.requests().is_empty());
        assert_eq!(actions.acted.lock().unwrap()[3].1["thread_id"], "41");
    }

    #[test]
    fn a_choice_says_how_the_email_stands() {
        let mut t = budget().summary;
        assert_eq!(match_line(&t), "[t:41] “Q4 budget” from Dana Lee");
        t.snoozed_until = Some(NOV_1);
        assert_eq!(match_line(&t), "[t:41] “Q4 budget” from Dana Lee (coming back later)");
        t.done = true;
        assert_eq!(match_line(&t), "[t:41] “Q4 budget” from Dana Lee (Done)");
    }

    async fn reviewing(model: &ScriptedModel, q: &str) -> Answer {
        let mail = FakeMail::new();
        let actions = FakeActions::default();
        let tools = MailTools::new(&mail, NOV_1).with_actions(&actions);
        let stop = AtomicBool::new(false);
        run(Run { chat: "c", model, tools: &tools, files: &[], threads: &[], review: true, emit: &|_| {}, stop: &stop }, question(q)).await.unwrap().0
    }

    #[tokio::test]
    async fn the_model_reviews_each_answer_and_carries_on_when_it_isnt_done() {
        let model = ScriptedModel::new(vec![
            Step::Say(vec!["Which email do you mean?"]),
            Step::Call(vec![("review", r#"{"done": false, "next": "Find Dana's budget email yourself and draft the reply."}"#)]),
            Step::Call(vec![("write_email", r#"{"reply_to_thread":"41","body":"Yes."}"#)]),
            Step::Say(vec!["I drafted a reply to Dana."]),
            Step::Call(vec![("review", r#"{"done": true, "next": ""}"#)]),
        ]);
        let answer = reviewing(&model, "Draft a reply to Dana").await;
        assert_eq!(answer.text, "I drafted a reply to Dana.");
        assert_eq!(answer.drafts.len(), 1);
        let requests = model.requests();
        let review = &requests[1];
        assert!(review.must_call && review.tools[0]["function"]["name"] == "review");
        let Msg::System(rules) = &review.messages[0] else { panic!("the review's rules") };
        assert!(rules.contains("wrote an email out in its answer, even one it also wrote with write_email (Moss shows that email under the answer, so the answer only says in a sentence what it wrote)"), "{rules}");
        let Msg::User(looked_at) = review.messages.last().unwrap() else { panic!("a question") };
        assert_eq!(looked_at, "The user asked:\nDraft a reply to Dana\n\nThe assistant's tool calls, with what they said:\n(none)\n\nIts answer:\nWhich email do you mean?");
        assert_eq!(requests[2].messages.last(), Some(&Msg::User("Find Dana's budget email yourself and draft the reply.".into())));
        let Msg::User(second) = requests[4].messages.last().unwrap() else { panic!("a question") };
        assert!(second.contains("- write_email {\"body\":\"Yes.\",\"reply_to_thread\":\"41\"}: waiting for the user to press Send"), "{second}");
    }

    #[tokio::test]
    async fn a_review_that_says_done_or_doesnt_come_back_leaves_the_answer_and_there_are_two_at_most() {
        let model = ScriptedModel::new(vec![Step::Say(vec!["Dana wants a yes."]), Step::Call(vec![("review", r#"{"done": true, "next": ""}"#)])]);
        assert_eq!(reviewing(&model, "What does Dana want?").await.text, "Dana wants a yes.");
        let model = ScriptedModel::new(vec![Step::Say(vec!["Dana wants a yes."]), Step::Say(vec!["not json"])]);
        assert_eq!(reviewing(&model, "What does Dana want?").await.text, "Dana wants a yes.");
        let model = ScriptedModel::new(vec![Step::Say(vec!["Dana wants a yes."]), Step::Call(vec![("review", r#"{"done": false, "next": ""}"#)])]);
        assert_eq!(reviewing(&model, "What does Dana want?").await.text, "Dana wants a yes.", "nothing to do next");
        let not_done = r#"{"done": false, "next": "Try again."}"#;
        let model = ScriptedModel::new(vec![Step::Say(vec!["A"]), Step::Call(vec![("review", not_done)]), Step::Say(vec!["B"]), Step::Call(vec![("review", not_done)]), Step::Say(vec!["C"])]);
        assert_eq!(reviewing(&model, "Do it").await.text, "C");
        assert_eq!(model.requests().len(), 5, "two reviews, then the answer stands");
    }

    #[tokio::test]
    async fn the_same_call_again_is_not_run_twice() {
        let call = ("write_email", r#"{"reply_to_thread":"41","body":"Thanks!"}"#);
        let model = ScriptedModel::new(vec![Step::Call(vec![call]), Step::Call(vec![call]), Step::Say(vec!["Drafted."])]);
        let mail = FakeMail::new();
        let actions = FakeActions::default();
        let tools = MailTools::new(&mail, NOV_1).with_actions(&actions);
        let stop = AtomicBool::new(false);
        let (answer, _) = run(Run { chat: "c", model: &model, tools: &tools, files: &[], threads: &[], review: false, emit: &|_| {}, stop: &stop }, question("Reply thanks")).await.unwrap();
        assert_eq!(actions.acted.lock().unwrap().len(), 1, "one draft, not two");
        assert_eq!(answer.drafts.len(), 1);
        let Msg::Tool { text, .. } = model.requests()[2].messages.last().unwrap().clone() else { panic!("a tool result") };
        assert!(text.starts_with("You did that already, with the same arguments. Don't do it again: answer me in a sentence."), "{text}");
    }

    #[tokio::test]
    async fn a_tool_call_written_as_text_is_left_out_of_the_answer() {
        let model = ScriptedModel::new(vec![Step::Say(vec!["I drafted it.\n\nHere it is:<|tool_call>call:write_email{body:<|\"|>Hi<|\"|>}<tool_call|>"])]);
        let (answer, _) = act(&model, "What's new?").await;
        assert_eq!(answer.text, "I drafted it.\n\nHere it is:");
    }

    #[tokio::test]
    async fn it_acts_on_several_emails_and_says_what_it_did() {
        let model = ScriptedModel::new(vec![
            Step::Call(vec![("list_inbox", "{}")]),
            Step::Call(vec![("mark_done", r#"{"thread_id":"41"}"#), ("mark_done", r#"{"thread_id":"41","done":false}"#), ("mark_done", r#"{"thread_id":"57"}"#)]),
            Step::Say(vec!["Done: the budget [t:41] and the approval [t:57]."]),
        ]);
        let (answer, labels) = act(&model, "Mark the budget emails done").await;
        assert_eq!(answer.did, ["Marked done: 41", "Marked done: 57"], "only what worked");
        let ids: Vec<&str> = answer.sources.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["41", "57"], "threads it listed can be cited");
        assert_eq!(labels, ["Working", "Marking an email Done", "Marking an email Done", "Marking an email Done"]);
        let Msg::Tool { text, .. } = &model.requests()[2].messages[6] else { panic!("the failure went back") };
        assert_eq!(text, "Couldn’t do that: it's in the inbox already");
        assert!(model.requests()[0].tools.len() == 9, "5 of its own, 4 of the app's");
    }

    #[tokio::test]
    async fn ids_written_as_citations_reach_the_apps_actions_plain() {
        let mail = FakeMail::new();
        let actions = FakeActions::default();
        let tools = MailTools::new(&mail, NOV_1).with_actions(&actions);
        tools.call("mark_done", &serde_json::json!({"thread_id": "[t:41]"})).await;
        tools.call("mark_done", &serde_json::json!({"thread_id": 57, "done": true})).await;
        tools.call("write_email", &serde_json::json!({"reply_to_thread": "t:41", "forward_thread": " 57 ", "draft_id": "[d1]", "body": "t:41"})).await;
        tools.call("mark_done", &serde_json::Value::Null).await;
        let got: Vec<serde_json::Value> = actions.acted.lock().unwrap().iter().map(|(_, a)| a.clone()).collect();
        assert_eq!(got[0], serde_json::json!({"thread_id": "41"}));
        assert_eq!(got[1], serde_json::json!({"thread_id": "57", "done": true}), "a number is an id too");
        assert_eq!(got[2], serde_json::json!({"reply_to_thread": "41", "forward_thread": "57", "draft_id": "d1", "body": "t:41"}), "only ids change");
        assert_eq!(got[3], serde_json::Value::Null, "no arguments stay none");
    }

    #[tokio::test]
    async fn an_email_to_send_comes_back_once_with_when_it_should_go() {
        let model = ScriptedModel::new(vec![
            Step::Call(vec![("write_email", r#"{"body":"Yes"}"#)]),
            Step::Call(vec![("write_email", r#"{"body":"Yes, Friday","send_at":1793610000000}"#), ("nope", "{}")]),
            Step::Say(vec!["It's ready: press Send."]),
        ]);
        let (answer, _) = act(&model, "Tell Dana yes, send it tomorrow").await;
        assert_eq!(answer.drafts.len(), 1, "the same draft, as it last was");
        assert_eq!(answer.drafts[0].body, "Yes, Friday");
        assert_eq!(answer.send_at.get("ready"), Some(&1_793_610_000_000));
        let Msg::Tool { text, .. } = &model.requests()[2].messages[6] else { panic!("no tool result") };
        assert_eq!(text, "There is no tool called nope.");
    }

    #[test]
    fn events_reach_the_ui_tagged_by_type() {
        let json = serde_json::to_value(AskEvent::Text { chat: "c".into(), delta: "Hi".into() }).unwrap();
        assert_eq!(json, serde_json::json!({"type": "text", "chat": "c", "delta": "Hi"}));
        let done = serde_json::to_value(AskEvent::Done { chat: "c".into(), answer: Answer::default() }).unwrap();
        assert_eq!(done["answer"], serde_json::json!({"text": "", "sources": [], "messages": [], "files": [], "drafts": [], "sendAt": {}, "did": [], "stopped": false}));
        let file = FileSource { thread_id: "41".into(), message_id: "m1".into(), index: 2, name: "a.pdf".into() };
        let cited = Answer { files: vec![file], ..Answer::default() };
        let json = serde_json::to_value(AskEvent::Done { chat: "c".into(), answer: cited }).unwrap();
        assert_eq!(json["answer"]["files"], serde_json::json!([{"threadId": "41", "messageId": "m1", "index": 2, "name": "a.pdf"}]));
        let failed = serde_json::to_value(AskEvent::Failed { chat: "c".into(), error: AiError::NotReady }).unwrap();
        assert_eq!(failed["error"]["kind"], "notReady");
    }

    #[test]
    fn two_long_threads_read_in_one_question_fit_the_context() {
        // The system message, the tools and the question take well under 3,000 tokens.
        assert!(2 * tokens(READ_CHARS) + ANSWER_TOKENS as usize + 3_000 < CONTEXT_TOKENS);
        assert!(READ_CHARS >= 40_000, "a long email and its replies, whole");
    }
}
