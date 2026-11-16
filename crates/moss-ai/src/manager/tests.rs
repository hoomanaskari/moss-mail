use super::*;

use sha2::{Digest, Sha256};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const GIB: u64 = 1 << 30;

fn sha(bytes: &[u8]) -> &'static str {
    let hex: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Box::leak(hex.into_boxed_str())
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// A runtime archive with a `llama-server` in it.
fn runtime_archive() -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(vec![], flate2::Compression::fast());
    {
        let mut tar = tar::Builder::new(&mut gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(6);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "llama-test/llama-server", &b"server"[..])
            .unwrap();
        tar.finish().unwrap();
    }
    gz.finish().unwrap()
}

fn file(n: u8, len: usize) -> Vec<u8> {
    (0..len).map(|i| (i as u8).wrapping_mul(n)).collect()
}

fn asset(server: &MockServer, id: &'static str, bytes: &[u8]) -> Asset {
    Asset {
        id,
        url: leak(format!("{}/{id}", server.uri())),
        file: leak(format!("{id}.bin")),
        bytes: bytes.len() as u64,
        sha256: sha(bytes),
    }
}

fn writer(id: &'static str, asset: Asset, needs_gb: u64, recommended_from_gb: u64) -> Writer {
    Writer {
        id,
        name: "Gemma",
        sub: "A model",
        asset,
        needs_gb,
        recommended_from_gb,
    }
}

/// Records each model it starts and answers from the mock server.
#[derive(Default)]
struct FakeLauncher {
    url: Mutex<String>,
    started: Mutex<Vec<(PathBuf, Role)>>,
    /// The program each start ran.
    binaries: Mutex<Vec<PathBuf>>,
    fail: AtomicBool,
    /// Models running now.
    live: Arc<std::sync::atomic::AtomicUsize>,
}

struct FakeRunning(Endpoint, Arc<std::sync::atomic::AtomicUsize>);

impl Drop for FakeRunning {
    fn drop(&mut self) {
        self.1.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Running for FakeRunning {
    fn endpoint(&self) -> Endpoint {
        self.0.clone()
    }
}

#[async_trait]
impl Launch for FakeLauncher {
    async fn launch(
        &self,
        binary: &Path,
        model: &Path,
        role: Role,
        key_file: &Path,
    ) -> Result<Box<dyn Running>, ServerError> {
        assert!(binary.ends_with("llama-server") || binary.ends_with("moss-apple"), "{binary:?}");
        self.binaries.lock().unwrap().push(binary.to_path_buf());
        assert!(key_file.parent().unwrap().ends_with("run"), "{key_file:?}");
        if self.fail.load(Ordering::Relaxed) {
            return Err(ServerError::Exited);
        }
        self.started
            .lock()
            .unwrap()
            .push((model.to_path_buf(), role));
        self.live.fetch_add(1, Ordering::Relaxed);
        let endpoint = Endpoint {
            url: self.url.lock().unwrap().clone(),
            key: "k".into(),
        };
        Ok(Box::new(FakeRunning(endpoint, self.live.clone())))
    }
}

struct Setup {
    server: MockServer,
    dir: tempfile::TempDir,
    catalog: Catalog,
    launcher: Arc<FakeLauncher>,
    seen: Arc<Mutex<Vec<Status>>>,
    files: Vec<(Asset, Vec<u8>)>,
    apple: Option<Apple>,
}

async fn setup() -> Setup {
    let server = MockServer::start().await;
    let runtime = runtime_archive();
    let small = file(3, 50_000);
    let big = file(7, 80_000);
    let decider = file(5, 20_000);
    let embedder = file(11, 10_000);
    let catalog = Catalog {
        runtime: asset(&server, "runtime", &runtime),
        runtime_dir: "llama-test",
        embedder: asset(&server, "embedder", &embedder),
        decider: asset(&server, "decider", &decider),
        writers: vec![
            writer("small", asset(&server, "small", &small), 0, 0),
            writer("big", asset(&server, "big", &big), 16, 16),
            writer("huge", asset(&server, "huge", &file(9, 10)), 64, 64),
        ],
    };
    let files = vec![
        (catalog.runtime, runtime),
        (catalog.embedder, embedder),
        (catalog.decider, decider),
        (catalog.writers[0].asset, small),
        (catalog.writers[1].asset, big),
    ];
    let launcher = Arc::new(FakeLauncher::default());
    *launcher.url.lock().unwrap() = server.uri();
    Setup {
        server,
        dir: tempfile::tempdir().unwrap(),
        catalog,
        launcher,
        seen: Arc::default(),
        files,
        apple: None,
    }
}

/// A Mac with Apple Intelligence on.
async fn with_apple() -> Setup {
    let apple = Apple { binary: PathBuf::from("/Applications/Moss.app/Contents/MacOS/moss-apple"), context: 8192 };
    Setup { apple: Some(apple), ..setup().await }
}

impl Setup {
    fn parts(&self, memory_gb: u64, free_space: fn(&Path) -> Option<u64>) -> Parts {
        let seen = self.seen.clone();
        Parts {
            dir: self.dir.path().join("ai"),
            machine: Machine {
                chip: "Apple M3".into(),
                memory: memory_gb * GIB,
            },
            catalog: self.catalog.clone(),
            launcher: self.launcher.clone(),
            events: Arc::new(move |s| seen.lock().unwrap().push(s)),
            free_space,
            runtime: None,
            conditions: || None,
            recheck: Duration::from_millis(20),
            apple: self.apple.clone(),
        }
    }

    fn ai(&self) -> Ai {
        Ai::new(self.parts(16, |_| Some(u64::MAX)))
    }

    async fn serve_all(&self) {
        for (asset, bytes) in &self.files {
            Mock::given(method("GET"))
                .and(path(format!("/{}", asset.id)))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(bytes.clone()))
                .mount(&self.server)
                .await;
        }
    }

    fn total(&self, writer: usize) -> u64 {
        self.catalog.runtime.bytes
            + self.catalog.embedder.bytes
            + self.catalog.decider.bytes
            + self.catalog.writers[writer].asset.bytes
    }

    fn states(&self) -> Vec<Download> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .map(|s| s.download.clone())
            .collect()
    }
}

async fn downloaded(s: &Setup) -> Ai {
    s.serve_all().await;
    let ai = s.ai();
    ai.download().unwrap().await.unwrap();
    assert_eq!(ai.status().download, Download::Ready);
    ai
}

#[tokio::test]
async fn shows_the_real_mac_and_recommends_a_model_for_it() {
    let s = setup().await;
    let status = s.ai().status();
    assert_eq!(status.machine, "M3, 16 GB memory");
    assert_eq!(status.writer, "big");
    let marks: Vec<_> = status
        .writers
        .iter()
        .map(|w| (w.id.as_str(), w.recommended, w.too_big))
        .collect();
    assert_eq!(
        marks,
        [
            ("small", false, false),
            ("big", true, false),
            ("huge", false, true)
        ]
    );
    assert_eq!(status.decider_size, catalog::size_label(20_000));
    assert_eq!(
        status.download,
        Download::Missing {
            done: 0,
            total: s.total(1)
        }
    );

    let small_mac = Ai::new(s.parts(8, |_| None)).status();
    assert_eq!(small_mac.writer, "small");
}

#[tokio::test]
async fn downloads_everything_with_progress_and_says_ready_only_at_the_end() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    let states = s.states();
    assert!(
        matches!(states.first(), Some(Download::Downloading { done: 0, .. })),
        "{states:?}"
    );
    assert_eq!(states.last(), Some(&Download::Ready));
    // Ready appears once, at the end; progress only goes up.
    assert_eq!(states.iter().filter(|d| **d == Download::Ready).count(), 1);
    let done: Vec<u64> = states
        .iter()
        .filter_map(|d| {
            if let Download::Downloading { done, .. } = d {
                Some(*done)
            } else {
                None
            }
        })
        .collect();
    assert!(done.windows(2).all(|w| w[0] <= w[1]), "{done:?}");
    assert_eq!(done.last(), Some(&s.total(1)));

    let ai_dir = s.dir.path().join("ai");
    assert!(ai_dir.join("runtime/llama-test/llama-server").is_file());
    assert!(
        !ai_dir.join("runtime/runtime.bin").exists(),
        "the archive is removed once unpacked"
    );
    assert!(ai_dir.join("models/decider.bin").is_file());
    assert!(ai_dir.join("models/big.bin").is_file());
    assert!(ai.decider_ready() && ai.writer_ready());
    // Nothing left to get.
    assert!(ai.download().is_none());
    // A new start reads it all from disk.
    assert_eq!(s.ai().status().download, Download::Ready);
}

#[tokio::test]
async fn nothing_downloads_until_asked() {
    let s = setup().await;
    s.serve_all().await;
    let ai = s.ai();
    assert!(matches!(
        ai.needs_reply("email").await,
        Err(AiError::NotReady)
    ));
    assert!(matches!(ai.ask_line("email").await, Err(AiError::NotReady)));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(s.server.received_requests().await.unwrap().is_empty());
    assert!(s.launcher.started.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_second_download_request_while_one_runs_is_ignored() {
    let s = setup().await;
    s.serve_all().await;
    let ai = s.ai();
    let first = ai.download().unwrap();
    assert!(ai.download().is_none());
    first.await.unwrap();
    assert_eq!(ai.status().download, Download::Ready);
}

#[tokio::test]
async fn a_damaged_file_fails_and_a_retry_gets_it() {
    let s = setup().await;
    Mock::given(method("GET"))
        .and(path("/decider"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![
            0u8;
            s.catalog.decider.bytes
                as usize
        ]))
        .up_to_n_times(1)
        .mount(&s.server)
        .await;
    s.serve_all().await;
    let ai = s.ai();
    ai.download().unwrap().await.unwrap();
    let Download::Failed {
        message,
        done,
        total,
    } = ai.status().download
    else {
        panic!("{:?}", ai.status())
    };
    assert_eq!(
        message,
        "A downloaded file was damaged, so Moss deleted it. Try again."
    );
    assert_eq!(done, s.catalog.runtime.bytes + s.catalog.embedder.bytes, "the runtime and the embedding model stay");
    assert_eq!(total, s.total(1));
    assert!(!ai.decider_ready());

    ai.download().unwrap().await.unwrap();
    assert_eq!(ai.status().download, Download::Ready);
}

#[tokio::test]
async fn says_why_a_download_failed() {
    let s = setup().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&s.server)
        .await;
    let ai = s.ai();
    ai.download().unwrap().await.unwrap();
    assert!(
        matches!(ai.status().download, Download::Failed { message, .. } if message == "The download server answered 503. Try again later.")
    );

    for (e, text) in [
        (
            DownloadError::Network("x".into()),
            "Can’t reach the download server. Check your connection and try again.",
        ),
        (
            DownloadError::Incomplete,
            "The download stopped early. Try again to pick up where it stopped.",
        ),
        (DownloadError::Disk("disk full".into()), "Disk full."),
        (DownloadError::Disk(String::new()), "."),
        (DownloadError::Cancelled, "Stopped."),
    ] {
        assert_eq!(message(&e), text);
    }
}

#[tokio::test]
async fn checks_for_disk_space_first() {
    let s = setup().await;
    s.serve_all().await;
    let ai = Ai::new(s.parts(16, |_| Some(1000)));
    ai.download().unwrap().await.unwrap();
    let Download::Failed { message, .. } = ai.status().download else {
        panic!()
    };
    assert!(
        message.starts_with("Not enough free space. Moss needs "),
        "{message}"
    );
    assert!(s.server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn stopping_keeps_what_is_in_and_resumes_later() {
    let s = setup().await;
    // The writer is slow, so there's time to stop.
    let big = s.files[4].1.clone();
    Mock::given(method("GET"))
        .and(path("/big"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(big)
                .set_delay(Duration::from_millis(300)),
        )
        .mount(&s.server)
        .await;
    s.serve_all().await;
    let ai = s.ai();
    let task = ai.download().unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    ai.cancel();
    task.await.unwrap();
    let after = s.catalog.runtime.bytes + s.catalog.embedder.bytes + s.catalog.decider.bytes;
    let Download::Missing { done, total } = ai.status().download else {
        panic!("{:?}", ai.status())
    };
    assert!(done >= after, "{done} {after}");
    assert_eq!(total, s.total(1));
    assert!(
        ai.decider_ready(),
        "the decision model is in and can run already"
    );
    assert!(!ai.writer_ready());

    ai.download().unwrap().await.unwrap();
    assert_eq!(ai.status().download, Download::Ready);
}

#[tokio::test]
async fn choosing_a_model_is_saved_and_changes_what_to_download() {
    let s = setup().await;
    let ai = s.ai();
    let status = ai.choose_writer("small").await.unwrap();
    assert_eq!(status.writer, "small");
    assert_eq!(
        status.download,
        Download::Missing {
            done: 0,
            total: s.total(0)
        }
    );
    assert_eq!(s.ai().status().writer, "small", "saved for the next start");
    assert!(matches!(s.seen.lock().unwrap().last(), Some(st) if st.writer == "small"));

    assert_eq!(
        ai.choose_writer("huge").await.err(),
        Some(AiError::Choice("huge".into()))
    );
    assert_eq!(
        ai.choose_writer("gpt").await.err(),
        Some(AiError::Choice("gpt".into()))
    );
    assert_eq!(ai.status().writer, "small");

    // A saved model that's gone from the list falls back to the recommended one.
    std::fs::write(
        s.dir.path().join("ai/settings.json"),
        br#"{"writer":"retired"}"#,
    )
    .unwrap();
    assert_eq!(s.ai().status().writer, "big");
}

#[tokio::test]
async fn a_settings_file_it_cannot_write_is_an_error() {
    let s = setup().await;
    std::fs::write(s.dir.path().join("ai"), b"a file, not a folder").unwrap();
    assert!(matches!(
        s.ai().choose_writer("small").await,
        Err(AiError::Disk(_))
    ));
}

fn needs_reply_answer(p: f64) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({"answers": {"reply": {"noul": p}}}))
}

/// The writing model's answer, streamed.
fn written(text: &str) -> ResponseTemplate {
    let event = serde_json::json!({"choices": [{"delta": {"content": text}}]});
    ResponseTemplate::new(200).set_body_raw(format!("data: {event}\n\ndata: [DONE]\n\n"), "text/event-stream")
}

#[tokio::test]
async fn the_decision_model_starts_once_and_answers() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(needs_reply_answer(0.93))
        .mount(&s.server)
        .await;
    assert_eq!(
        ai.needs_reply("From: Dana\n\nCan you send it?").await,
        Ok(0.93)
    );
    assert_eq!(ai.needs_reply("again").await, Ok(0.93));
    let started = s.launcher.started.lock().unwrap().clone();
    assert_eq!(started.len(), 1);
    assert_eq!(started[0].1, Role::Decider);
    assert!(started[0].0.ends_with("models/decider.bin"));
}

#[tokio::test]
async fn a_failed_answer_restarts_the_model_next_time() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(needs_reply_answer(0.1))
        .mount(&s.server)
        .await;
    assert!(matches!(ai.needs_reply("x").await, Err(AiError::Model(_))));
    assert_eq!(ai.needs_reply("x").await, Ok(0.1));
    assert_eq!(s.launcher.started.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_model_that_will_not_start_is_an_error() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    s.launcher.fail.store(true, Ordering::Relaxed);
    assert_eq!(
        ai.needs_reply("x").await,
        Err(AiError::Server(ServerError::Exited))
    );
    assert_eq!(
        ai.ask_line("x").await,
        Err(AiError::Server(ServerError::Exited))
    );
}

#[tokio::test]
async fn the_writing_model_writes_the_ask_and_follows_your_choice() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(written("\"Send Dana the deck by Friday.\""))
        .up_to_n_times(2)
        .mount(&s.server)
        .await;
    assert_eq!(
        ai.ask_line("From: Dana\n\nCan you send the deck by Friday?")
            .await
            .unwrap()
            .as_deref(),
        Some("Send Dana the deck by Friday.")
    );
    assert!(ai.ask_line("again").await.unwrap().is_some());
    assert_eq!(s.launcher.started.lock().unwrap().len(), 1);

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(written("Nothing to do."))
        .mount(&s.server)
        .await;
    assert_eq!(ai.ask_line("FYI").await, Ok(None));

    // Another writing model: the old one stops at once to free its memory. Download the new
    // one, and the next request runs it instead.
    assert_eq!(s.launcher.live.load(Ordering::Relaxed), 1);
    ai.choose_writer("small").await.unwrap();
    assert_eq!(s.launcher.live.load(Ordering::Relaxed), 0);
    assert!(!ai.writer_ready());
    assert_eq!(ai.ask_line("x").await, Err(AiError::NotReady));
    ai.download().unwrap().await.unwrap();
    ai.ask_line("x").await.unwrap();
    let started = s.launcher.started.lock().unwrap().clone();
    assert_eq!(started.len(), 2);
    assert_eq!(started[1].1, Role::Writer);
    assert!(started[1].0.ends_with("models/small.bin"));
}

#[tokio::test]
async fn a_failed_reply_restarts_the_writing_model_next_time() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(written("Reply to Dana."))
        .mount(&s.server)
        .await;
    assert!(matches!(ai.ask_line("x").await, Err(AiError::Model(_))));
    assert_eq!(
        ai.ask_line("x").await.unwrap().as_deref(),
        Some("Reply to Dana.")
    );
    assert_eq!(s.launcher.started.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_runtime_on_this_mac_can_stand_in_for_the_download() {
    let s = setup().await;
    s.serve_all().await;
    let binary = s.dir.path().join("llama-server");
    std::fs::write(&binary, b"server").unwrap();
    let ai = Ai::new(Parts {
        runtime: Some(binary),
        ..s.parts(16, |_| None)
    });
    let total = s.catalog.embedder.bytes + s.catalog.decider.bytes + s.catalog.writers[1].asset.bytes;
    assert_eq!(ai.status().download, Download::Missing { done: 0, total });
    ai.download().unwrap().await.unwrap();
    assert_eq!(ai.status().download, Download::Ready);
    let paths: Vec<String> = s
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| r.url.path().to_string())
        .collect();
    assert!(!paths.contains(&"/runtime".to_string()), "{paths:?}");

    let missing = Ai::new(Parts {
        runtime: Some(s.dir.path().join("nope")),
        ..s.parts(16, |_| None)
    });
    assert!(!missing.decider_ready());
}

#[tokio::test]
async fn a_runtime_archive_without_the_server_is_damaged() {
    let s = setup().await;
    let mut gz = flate2::write::GzEncoder::new(vec![], flate2::Compression::fast());
    tar::Builder::new(&mut gz).finish().unwrap();
    let empty = gz.finish().unwrap();
    let catalog = Catalog {
        runtime: asset(&s.server, "empty", &empty),
        ..s.catalog.clone()
    };
    Mock::given(method("GET"))
        .and(path("/empty"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(empty))
        .mount(&s.server)
        .await;
    let ai = Ai::new(Parts {
        catalog,
        ..s.parts(16, |_| None)
    });
    ai.download().unwrap().await.unwrap();
    assert!(
        matches!(ai.status().download, Download::Failed { message, .. } if message.contains("damaged"))
    );
}

#[tokio::test]
async fn a_runtime_archive_that_will_not_unpack_fails() {
    let s = setup().await;
    let junk = b"not an archive".to_vec();
    let catalog = Catalog {
        runtime: asset(&s.server, "junk", &junk),
        ..s.catalog.clone()
    };
    Mock::given(method("GET"))
        .and(path("/junk"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(junk))
        .mount(&s.server)
        .await;
    let ai = Ai::new(Parts {
        catalog,
        ..s.parts(16, |_| None)
    });
    ai.download().unwrap().await.unwrap();
    assert!(
        matches!(ai.status().download, Download::Failed { message, .. } if message.starts_with("Couldn’t unpack the AI runtime"))
    );
}

#[test]
fn the_app_downloads_the_pinned_files() {
    let pinned = Catalog::pinned();
    assert_eq!(pinned.runtime, catalog::RUNTIME);
    assert_eq!(pinned.decider, catalog::DECIDER);
    assert_eq!(pinned.writers, catalog::WRITERS);
    assert_eq!(pinned.runtime_dir, catalog::RUNTIME_DIR);
}

#[test]
fn knows_how_much_space_is_free() {
    let dir = tempfile::tempdir().unwrap();
    assert!(free_space(dir.path()).is_some_and(|n| n > 0));
    assert_eq!(Role::Decider.name(), "decider");
    assert!(Role::Decider.flags().contains(&"--cache-ram"));
    assert!(!Role::Writer.flags().contains(&"--cache-ram"));
    // Room for a whole email or conversation, and the summary of it.
    let writer = Role::Writer.flags();
    let c = writer.iter().position(|f| *f == "-c").unwrap();
    assert_eq!(writer[c + 1], crate::agent::CONTEXT_TOKENS.to_string());
    assert!(crate::agent::CONTEXT_TOKENS >= 32_768);
}

fn thread(messages: &[(&str, &str)]) -> Thread {
    let person = |email: &str| moss_core::Person { name: String::new(), email: email.into() };
    let messages: Vec<moss_core::Message> = messages
        .iter()
        .map(|(id, from)| moss_core::Message {
            id: id.to_string(),
            thread_id: "t".into(),
            from: person(from),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: "Deck".into(),
            sent_at: 0,
            paragraphs: vec![format!("Message {id}")],
            body_text: String::new(),
            body_html: None,
            attachments: vec![],
            read: true,
            image_trust: Default::default(),
            sender_unverified: false,
            headers_only: false,
            images_missing: false,
            forwards: vec![],
            forwarded: false,
        })
        .collect();
    Thread {
        summary: moss_core::ThreadSummary {
            id: "t".into(),
            account_id: "work".into(),
            split: moss_core::SplitId::Important,
            section: moss_core::Section::Reply,
            from: person("dana@acme.co"),
            subject: "Deck".into(),
            summary: String::new(),
            last_message_at: 0,
            snoozed_until: None,
            unread: false,
            done: false,
            done_at: None,
            labels: vec![],
            message_count: messages.len() as u32,
            has_attachments: false,
            ai_sure: None,
            ai_summary: None,
            to_address: None,
            ai_summary_why: None,
            sender: Default::default(),
            due: None,
            split_why: None,
            waiting: None,
            moved_by_you: false,
            you_wrote_last: false,
        },
        messages,
    }
}

#[tokio::test]
async fn the_ask_reads_the_latest_message_from_them_and_is_written_once() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_string_contains("Message 2"))
        .respond_with(written("Send Dana the deck."))
        .expect(1)
        .mount(&s.server)
        .await;
    // Dana wrote 1 and 2; you answered with 3.
    let t = thread(&[("1", "dana@acme.co"), ("2", "dana@acme.co"), ("3", "maya@northwind.co")]);
    assert_eq!(ai.ask_for(&t).await.unwrap().as_deref(), Some("Send Dana the deck."));
    assert_eq!(ai.ask_for(&t).await.unwrap().as_deref(), Some("Send Dana the deck."), "remembered");

    // Only your own messages: the latest one. No messages: nothing to ask.
    Mock::given(method("POST")).and(path("/v1/chat/completions")).respond_with(written("Nothing to do.")).mount(&s.server).await;
    assert_eq!(ai.ask_for(&thread(&[("9", "maya@northwind.co")])).await, Ok(None));
    assert_eq!(ai.ask_for(&thread(&[])).await, Ok(None));
}

#[tokio::test]
async fn the_first_run_choice_offers_real_sizes_and_nothing_is_picked_yet() {
    let s = setup().await;
    let status = s.ai().status();
    assert_eq!(status.plan, None);
    let plans: Vec<(Plan, u64)> = status.plans.iter().map(|p| (p.plan, p.bytes)).collect();
    let essentials = s.catalog.runtime.bytes + s.catalog.embedder.bytes + s.catalog.decider.bytes;
    assert_eq!(plans, [(Plan::Recommended, s.total(1)), (Plan::Essentials, essentials), (Plan::Skip, 0)]);
    assert_eq!(status.embedder_size, catalog::size_label(10_000));
    assert_eq!(status.features, Features::default());
    assert_eq!((status.on_disk, status.paused), (0, None));
}

#[tokio::test]
async fn essentials_get_the_small_models_only_and_each_feature_starts_as_it_arrives() {
    let s = setup().await;
    s.serve_all().await;
    let ai = s.ai();
    let status = ai.choose_plan(Plan::Essentials).unwrap();
    assert_eq!(status.plan, Some(Plan::Essentials));
    // Wait for the download the choice started.
    while matches!(ai.status().download, Download::Downloading { .. }) {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let status = ai.status();
    assert_eq!(status.download, Download::Ready);
    assert_eq!(status.features, Features { search: true, sorting: true, writing: false });
    let essentials = s.catalog.runtime.bytes + s.catalog.embedder.bytes + s.catalog.decider.bytes;
    assert_eq!(status.on_disk, essentials);
    // Search came first: smallest first.
    let first = s.seen.lock().unwrap().iter().position(|st| st.features.search).unwrap();
    let sorting = s.seen.lock().unwrap().iter().position(|st| st.features.sorting).unwrap();
    assert!(first < sorting);
    // The choice is remembered.
    assert_eq!(s.ai().status().plan, Some(Plan::Essentials));
}

#[tokio::test]
async fn skipping_downloads_nothing_and_the_download_button_gets_the_lot() {
    let s = setup().await;
    s.serve_all().await;
    let ai = s.ai();
    assert_eq!(ai.choose_plan(Plan::Skip).unwrap().plan, Some(Plan::Skip));
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(s.server.received_requests().await.unwrap().is_empty());
    assert_eq!(ai.status().download, Download::Missing { done: 0, total: s.total(1) });
    ai.download().unwrap().await.unwrap();
    assert_eq!(ai.status().plan, Some(Plan::Recommended));
    assert!(ai.status().features.writing);
}

#[tokio::test]
async fn a_writing_model_chosen_before_plans_existed_means_the_lot() {
    let s = setup().await;
    std::fs::create_dir_all(s.dir.path().join("ai")).unwrap();
    std::fs::write(s.dir.path().join("ai/settings.json"), br#"{"writer":"small"}"#).unwrap();
    assert_eq!(s.ai().status().plan, Some(Plan::Recommended));
}

static WAIT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

fn waiting() -> Option<Pause> {
    match WAIT.load(Ordering::Relaxed) {
        1 => Some(Pause::Battery),
        2 => Some(Pause::Hotspot),
        _ => None,
    }
}

#[tokio::test]
async fn a_download_waits_on_low_battery_or_a_hotspot_then_goes_on() {
    let s = setup().await;
    // Slow enough to be caught halfway.
    Mock::given(method("GET"))
        .and(path("/big"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(s.files[4].1.clone()).set_delay(Duration::from_millis(200)))
        .mount(&s.server)
        .await;
    s.serve_all().await;
    WAIT.store(1, Ordering::Relaxed);
    let ai = Ai::new(Parts { conditions: waiting, ..s.parts(16, |_| Some(u64::MAX)) });
    let task = ai.download().unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert_eq!(ai.status().paused, Some(Pause::Battery));
    assert!(s.server.received_requests().await.unwrap().is_empty(), "nothing while on low battery");
    WAIT.store(0, Ordering::Relaxed);
    // Charging: it starts, then a hotspot stops the slow file halfway.
    while !ai.status().features.sorting {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    WAIT.store(2, Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(ai.status().paused, Some(Pause::Hotspot));
    assert!(!ai.writer_ready());
    WAIT.store(0, Ordering::Relaxed);
    task.await.unwrap();
    assert_eq!(ai.status().download, Download::Ready);
    assert_eq!(ai.status().paused, None);
}

#[tokio::test]
async fn stopping_while_waiting_stops() {
    let s = setup().await;
    s.serve_all().await;
    let ai = Ai::new(Parts { conditions: || Some(Pause::Hotspot), ..s.parts(16, |_| Some(u64::MAX)) });
    let task = ai.download().unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    ai.cancel();
    task.await.unwrap();
    assert!(matches!(ai.status().download, Download::Missing { .. }));
    assert_eq!(ai.status().paused, None);
}

#[tokio::test]
async fn removing_the_models_frees_the_space_and_turns_the_ai_off() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(needs_reply_answer(0.9))
        .mount(&s.server)
        .await;
    ai.needs_reply("x").await.unwrap();
    let status = ai.remove_models().await.unwrap();
    assert_eq!((status.plan, status.on_disk), (Some(Plan::Skip), 0));
    assert_eq!(status.features, Features::default());
    assert!(!s.dir.path().join("ai/models").exists());
    assert_eq!(s.launcher.live.load(Ordering::Relaxed), 0, "the models stopped");
    // Again: nothing to remove is fine.
    assert!(ai.remove_models().await.is_ok());
}

#[tokio::test]
async fn a_writing_model_you_switched_away_from_stays_until_you_delete_it() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    assert_eq!(ai.status().downloaded, ["embedder", "decider", "big"]);
    ai.choose_writer("small").await.unwrap();
    ai.download().unwrap().await.unwrap();
    let both = ai.status();
    assert_eq!(both.downloaded, ["embedder", "decider", "small", "big"]);

    let status = ai.remove_model("big").await.unwrap();
    assert_eq!(status.downloaded, ["embedder", "decider", "small"]);
    assert_eq!(status.on_disk, both.on_disk - s.catalog.writers[1].asset.bytes);
    assert_eq!(status.download, Download::Ready, "the chosen one is still in");
    assert!(!s.dir.path().join("ai/models/big.bin").exists());
    assert!(!s.dir.path().join("ai/models/big.bin.ok").exists());
    assert!(matches!(s.seen.lock().unwrap().last(), Some(st) if st.downloaded.len() == 3));
}

#[tokio::test]
async fn quitting_stops_every_model_and_starting_clears_what_a_crash_left() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(needs_reply_answer(0.9))
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(written("Nothing to do."))
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [{"index": 0, "embedding": [0.5, 0.5]}]})))
        .mount(&s.server)
        .await;
    ai.needs_reply("x").await.unwrap();
    ai.ask_line("x").await.unwrap();
    ai.embed(&["x".into()]).await.unwrap();
    assert_eq!(s.launcher.live.load(Ordering::Relaxed), 3);
    ai.stop_all().await;
    assert_eq!(s.launcher.live.load(Ordering::Relaxed), 0);

    // A process id left by a run that crashed; here it belongs to another program, so it stays.
    let leftover = s.dir.path().join("ai/run/decider.pid");
    std::fs::write(&leftover, std::process::id().to_string()).unwrap();
    assert_eq!(ai.stop_leftovers(), 0);
    assert!(!leftover.exists());
    assert_eq!(setup().await.ai().stop_leftovers(), 0, "no runtime yet: nothing to stop");
}

#[tokio::test]
async fn deleting_a_model_stops_it_and_turns_its_job_off_until_downloaded_again() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(needs_reply_answer(0.9))
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(written("Nothing to do."))
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [{"index": 0, "embedding": [0.5, 0.5]}]})))
        .mount(&s.server)
        .await;
    ai.needs_reply("x").await.unwrap();
    ai.ask_line("x").await.unwrap();
    ai.embed(&["x".into()]).await.unwrap();
    assert_eq!(s.launcher.live.load(Ordering::Relaxed), 3);

    let status = ai.remove_model("big").await.unwrap();
    assert_eq!(s.launcher.live.load(Ordering::Relaxed), 2, "the writing model stopped");
    assert_eq!(status.features, Features { search: true, sorting: true, writing: false });
    assert_eq!(
        status.download,
        Download::Missing { done: s.total(1) - s.catalog.writers[1].asset.bytes, total: s.total(1) }
    );
    assert_eq!(ai.ask_line("x").await, Err(AiError::NotReady));

    let status = ai.remove_model("decider").await.unwrap();
    assert_eq!(s.launcher.live.load(Ordering::Relaxed), 1);
    assert!(!status.features.sorting);
    let status = ai.remove_model("embedder").await.unwrap();
    assert_eq!(s.launcher.live.load(Ordering::Relaxed), 0);
    assert_eq!(status.features, Features::default());
    assert!(status.downloaded.is_empty());
    assert_eq!(status.plan, Some(Plan::Recommended), "Download gets them back");

    ai.download().unwrap().await.unwrap();
    assert_eq!(ai.status().download, Download::Ready);
}

#[tokio::test]
async fn deleting_a_model_deletes_what_was_downloaded_of_it_so_far() {
    let s = setup().await;
    let ai = s.ai();
    std::fs::create_dir_all(s.dir.path().join("ai/models")).unwrap();
    std::fs::write(s.dir.path().join("ai/models/small.bin.part"), b"half").unwrap();
    assert_eq!(ai.status().downloaded, ["small"]);
    assert_eq!(ai.remove_model("small").await.unwrap().downloaded, Vec::<String>::new());
    assert!(!s.dir.path().join("ai/models/small.bin.part").exists());
    // Again: nothing to delete is fine. A model Moss doesn't know is an error.
    assert!(ai.remove_model("small").await.is_ok());
    assert_eq!(ai.remove_model("gpt").await.err(), Some(AiError::Choice("gpt".into())));
}

#[tokio::test]
async fn deleting_a_model_the_download_is_not_getting_leaves_it_running() {
    let s = setup().await;
    let big = s.files[4].1.clone();
    Mock::given(method("GET"))
        .and(path("/big"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(big)
                .set_delay(Duration::from_millis(300)),
        )
        .mount(&s.server)
        .await;
    s.serve_all().await;
    let ai = s.ai();
    std::fs::create_dir_all(s.dir.path().join("ai/models")).unwrap();
    std::fs::write(s.dir.path().join("ai/models/small.bin.part"), b"half").unwrap();
    let task = ai.download().unwrap();
    // On to the slow writing model.
    while !ai.decider_ready() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    ai.remove_model("small").await.unwrap();
    task.await.unwrap();
    assert_eq!(ai.status().download, Download::Ready, "it went on");
    assert_eq!(ai.status().downloaded, ["embedder", "decider", "big"]);

    // The one it's getting: the download stops, keeping what else is in.
    ai.choose_writer("small").await.unwrap();
    std::fs::write(s.dir.path().join("ai/models/small.bin.part"), b"half").unwrap();
    Mock::given(method("GET"))
        .and(path("/small"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(s.files[3].1.clone())
                .set_delay(Duration::from_millis(300)),
        )
        .with_priority(1)
        .mount(&s.server)
        .await;
    let task = ai.download().unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    ai.remove_model("small").await.unwrap();
    task.await.unwrap();
    assert!(matches!(ai.status().download, Download::Missing { .. }), "{:?}", ai.status().download);
    assert!(ai.decider_ready(), "the rest is kept");
}

#[tokio::test]
async fn a_model_it_cannot_delete_is_an_error() {
    let s = setup().await;
    let ai = s.ai();
    std::fs::create_dir_all(s.dir.path().join("ai/models/small.bin")).unwrap();
    assert!(matches!(ai.remove_model("small").await, Err(AiError::Disk(_))));
}

#[tokio::test]
async fn models_it_cannot_remove_are_an_error() {
    let s = setup().await;
    let ai = s.ai();
    std::fs::create_dir_all(s.dir.path().join("ai")).unwrap();
    std::fs::write(s.dir.path().join("ai/models"), b"a file, not a folder").unwrap();
    assert!(matches!(ai.remove_models().await, Err(AiError::Disk(_))));
}

#[tokio::test]
async fn the_embedding_model_starts_once_and_says_what_texts_mean() {
    let s = setup().await;
    assert!(matches!(s.ai().embed(&["x".into()]).await, Err(AiError::NotReady)));
    let ai = downloaded(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [{"index": 0, "embedding": [0.5, 0.5]}]})))
        .mount(&s.server)
        .await;
    assert!(ai.embed(&["x".into()]).await.is_err());
    assert_eq!(ai.embed(&["x".into()]).await.unwrap(), [vec![0.5, 0.5]]);
    assert_eq!(ai.embed(&["x".into()]).await.unwrap().len(), 1);
    let started = s.launcher.started.lock().unwrap().clone();
    assert_eq!(started.iter().filter(|(_, r)| *r == Role::Embedder).count(), 2, "restarted after the failure");
    assert!(started[0].0.ends_with("models/embedder.bin"));
    assert_eq!(ai.embedder_model(), "embedder");
    assert!(Role::Embedder.flags().contains(&"--embeddings"));
}

#[tokio::test]
async fn the_decision_model_picks_a_split_reads_deadlines_and_whether_you_wait_on_an_answer() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"answers": {
            "split": {"probabilities": {"important": 0.2, "receipts": 0.7, "other": 0.1}},
            "deadline": {"noul": 0.8},
            "expects": {"noul": 0.9}
        }})))
        .mount(&s.server)
        .await;
    assert_eq!(ai.split("email").await.unwrap(), ("receipts".into(), 0.7));
    assert_eq!(ai.split_for("email", false).await.unwrap(), ("receipts".into(), 0.7));
    assert_eq!(ai.deadline("email").await.unwrap(), 0.8);
    assert_eq!(ai.expects_reply("email").await.unwrap(), 0.9);
    assert_eq!(decide::Answer::YesNo(0.5).best(), None);
}

#[tokio::test]
async fn mail_from_a_mailing_list_is_asked_its_own_question() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    let Question::Choice { instructions, .. } = prompt::list_split() else { panic!("a choice") };
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(wiremock::matchers::body_partial_json(serde_json::json!({"questions": {"split": {"instructions": instructions}}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"answers": {
            "split": {"probabilities": {"newsletters": 0.4, "other": 0.55, "important": 0.05}}
        }})))
        .mount(&s.server)
        .await;
    assert_eq!(ai.list_split("email").await.unwrap(), ("other".into(), 0.55));
    assert_eq!(ai.split_for("email", true).await.unwrap(), ("other".into(), 0.55));
    assert!(ai.split_for("email", false).await.is_err(), "mail from people is asked the usual question");
}

#[tokio::test]
async fn a_sales_pitch_from_a_mailing_list_is_a_newsletter() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"answers": {
            "split": {"probabilities": {"outreach": 0.6, "other": 0.3, "important": 0.1}}
        }})))
        .mount(&s.server)
        .await;
    assert_eq!(ai.list_split("email").await.unwrap(), ("newsletters".into(), 0.6));
}

#[tokio::test]
async fn the_decision_model_says_whether_an_email_is_a_sales_pitch() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    let Question::YesNo { instructions: pitch } = prompt::pitch() else { panic!("yes or no") };
    let Question::YesNo { instructions: wants } = prompt::wants() else { panic!("yes or no") };
    // Both in one request: is it a pitch, and does the sender need something of the user's?
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .and(wiremock::matchers::body_partial_json(serde_json::json!({"questions": {
            "pitch": {"instructions": pitch}, "wants": {"instructions": wants}
        }})))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"answers": {
            "pitch": {"noul": 0.93}, "wants": {"noul": 0.12}
        }})))
        .expect(1)
        .mount(&s.server)
        .await;
    assert_eq!(ai.pitch("email").await.unwrap(), (0.93, 0.12));
}

#[tokio::test]
async fn decisions_wait_for_the_decision_model() {
    let s = setup().await;
    let ai = s.ai();
    assert_eq!(ai.split("email").await, Err(AiError::NotReady));
    assert_eq!(ai.list_split("email").await, Err(AiError::NotReady));
    assert_eq!(ai.pitch("email").await, Err(AiError::NotReady));
    assert_eq!(ai.deadline("email").await, Err(AiError::NotReady));
    assert_eq!(ai.expects_reply("email").await, Err(AiError::NotReady));
}

#[tokio::test]
async fn the_writing_model_sums_up_a_conversation() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(written("Dana needs the deck by Friday.\n"))
        .mount(&s.server)
        .await;
    assert_eq!(ai.summary("Subject: Deck").await.unwrap().as_deref(), Some("Dana needs the deck by Friday."));
}

#[tokio::test]
async fn the_writing_model_writes_each_kind_of_summary() {
    use moss_core::{SummaryContent, SummaryKind, SummaryRequest};
    let s = setup().await;
    let not_yet = s.ai();
    assert_eq!(
        not_yet.summarize_as("Subject: Deck", &SummaryRequest::of(SummaryKind::Short)).await,
        Err(AiError::NotReady)
    );
    let ai = downloaded(&s).await;
    let answer = Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(serde_json::json!({"max_tokens": 600})))
        .respond_with(written("- Send the deck | Friday\n"))
        .expect(1)
        .mount_as_scoped(&s.server)
        .await;
    let actions = ai.summarize_as("Subject: Deck", &SummaryRequest::of(SummaryKind::Actions)).await.unwrap();
    let SummaryContent::Actions { items } = actions else { panic!("actions") };
    assert_eq!((items[0].text.as_str(), items[0].due.as_deref()), ("Send the deck", Some("Friday")));
    drop(answer);
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(written("  \n"))
        .mount(&s.server)
        .await;
    assert!(matches!(
        ai.summarize_as("Subject: Deck", &SummaryRequest::of(SummaryKind::Detailed)).await,
        Err(AiError::Model(_))
    ));
}

#[tokio::test]
async fn a_summary_you_ask_for_streams_in_read_as_its_kind() {
    use moss_core::{ActionItem, SummaryContent, SummaryKind, SummaryRequest};
    let s = setup().await;
    let actions = SummaryRequest::of(SummaryKind::Actions);
    let mut quiet = |_: &SummaryContent| true;
    assert_eq!(s.ai().summarize_streaming("Subject: Deck", &actions, &mut quiet).await, Err(AiError::NotReady));
    let ai = downloaded(&s).await;
    let event = |piece: &str| format!("data: {}\n\n", serde_json::json!({"choices": [{"delta": {"content": piece}}]}));
    let sse = format!("{}{}{}{}{}{}data: [DONE]\n\n", event(" "), event("- Send the"), event(" deck | Fri"), event("day\n"), event("- Book a room\n"), event("\n"));
    let answer = Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(serde_json::json!({"stream": true, "max_tokens": 600})))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse, "text/event-stream"))
        .expect(2)
        .mount_as_scoped(&s.server)
        .await;
    let item = |text: &str, due: Option<&str>| ActionItem { text: text.into(), due: due.map(String::from) };
    let mut heard = vec![];
    let got = ai
        .summarize_streaming("Subject: Deck", &actions, &mut |c| {
            heard.push(c.clone());
            true
        })
        .await
        .unwrap();
    let as_items = |items: Vec<ActionItem>| SummaryContent::Actions { items };
    // Heard each time what it reads as changes, never the same twice.
    assert_eq!(
        heard,
        [
            as_items(vec![item("Send the", None)]),
            as_items(vec![item("Send the deck", Some("Fri"))]),
            as_items(vec![item("Send the deck", Some("Friday"))]),
            as_items(vec![item("Send the deck", Some("Friday")), item("Book a room", None)]),
        ]
    );
    assert_eq!(got, as_items(vec![item("Send the deck", Some("Friday")), item("Book a room", None)]));
    // Stopping gives what's read so far.
    let got = ai.summarize_streaming("Subject: Deck", &actions, &mut |_| false).await.unwrap();
    assert_eq!(got, as_items(vec![item("Send the", None)]));
    drop(answer);
    // Nothing that reads as a summary is a failure; an empty list isn't heard on the way.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_raw("data: [DONE]\n\n", "text/event-stream"))
        .mount(&s.server)
        .await;
    let detailed = SummaryRequest::of(SummaryKind::Detailed);
    assert!(matches!(ai.summarize_streaming("Subject: Deck", &detailed, &mut quiet).await, Err(AiError::Model(_))));
}

/// A model that answers 500 to the first `failures` requests, then writes "Sorry, no." to
/// everything, with quick replies "Yes" and "No".
async fn failing_first(s: &Setup, failures: u64) {
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(failures)
        .with_priority(1)
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(serde_json::json!({"stream": true})))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Sorry, no.\"}}]}\n\ndata: [DONE]\n\n",
            "text/event-stream",
        ))
        .mount(&s.server)
        .await;
    let options = serde_json::json!({"options": [{"label": "Yes", "says": "Say yes"}, {"label": "No", "says": "Say no"}]});
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(serde_json::json!({"response_format": {"type": "json_schema"}})))
        .respond_with(written(&options.to_string()))
        .with_priority(3)
        .mount(&s.server)
        .await;
}

#[tokio::test]
async fn a_reply_that_fails_is_tried_once_more_on_a_fresh_start() {
    use crate::reply::{Context, Job};
    let me = moss_core::Person { name: "Maya".into(), email: "maya@moss.app".into() };
    let context = Context { subject: "Deck".into(), messages: vec![], me, past: vec![], now: 0, signature: vec![], to: None };
    let mut quiet = |_: &str| true;
    // Failing once is what Retry used to fix by hand: the model starts again and it works.
    let s = setup().await;
    failing_first(&s, 1).await;
    let ai = downloaded(&s).await;
    assert_eq!(ai.reply_options(&context, &mut |_| true).await.unwrap().len(), 2);
    assert_eq!(s.launcher.started.lock().unwrap().len(), 2);
    let s = setup().await;
    failing_first(&s, 1).await;
    let ai = downloaded(&s).await;
    assert_eq!(ai.reply(&Job::Decline, "", &context, &mut quiet).await.unwrap(), "Sorry, no.");
    assert_eq!(s.launcher.started.lock().unwrap().len(), 2);
    // Failing twice is the answer, and the model is stopped for the next one.
    let s = setup().await;
    failing_first(&s, 2).await;
    let ai = downloaded(&s).await;
    assert!(matches!(ai.reply_options(&context, &mut |_| true).await, Err(AiError::Model(_))));
    assert_eq!(s.launcher.live.load(Ordering::Relaxed), 0);
    let s = setup().await;
    failing_first(&s, 2).await;
    let ai = downloaded(&s).await;
    assert!(matches!(ai.reply(&Job::Decline, "", &context, &mut quiet).await, Err(AiError::Model(_))));
    assert_eq!(s.launcher.started.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn the_writing_model_writes_replies_and_streams_them() {
    use crate::reply::{Context, Job, ReplyOption};
    let s = setup().await;
    let me = moss_core::Person { name: "Maya".into(), email: "maya@moss.app".into() };
    let context = Context { subject: "Deck".into(), messages: vec![], me, past: vec![], now: 0, signature: vec![], to: None };
    let mut quiet = |_: &str| true;
    assert_eq!(s.ai().reply_options(&context, &mut |_| true).await, Err(AiError::NotReady));
    assert_eq!(s.ai().reply(&Job::Decline, "", &context, &mut quiet).await, Err(AiError::NotReady));
    let ai = downloaded(&s).await;
    // Rewriting nothing doesn't start the model.
    assert_eq!(ai.reply(&Job::Shorter, " ", &context, &mut quiet).await, Ok(String::new()));
    assert!(s.launcher.started.lock().unwrap().is_empty());

    // Quick replies are chosen first, then each is written (here every one as "Sorry, no.").
    let options = serde_json::json!({"options": [{"label": "Yes", "says": "Say yes"}, {"label": "No", "says": "Say no"}]});
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(serde_json::json!({"stream": true})))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "data: {\"choices\":[{\"delta\":{\"content\":\"Sorry, no.\"}}]}\n\ndata: [DONE]\n\n",
            "text/event-stream",
        ))
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(wiremock::matchers::body_partial_json(serde_json::json!({"response_format": {"type": "json_schema"}})))
        .respond_with(written(&options.to_string()))
        .with_priority(3)
        .mount(&s.server)
        .await;
    assert_eq!(
        ai.reply_options(&context, &mut |_| true).await.unwrap(),
        [
            ReplyOption { label: "Yes".into(), draft: "Sorry, no.".into() },
            ReplyOption { label: "No".into(), draft: "Sorry, no.".into() }
        ]
    );
    let mut seen = vec![];
    let mut hear = |t: &str| {
        seen.push(t.to_string());
        true
    };
    assert_eq!(ai.reply(&Job::Decline, "", &context, &mut hear).await.unwrap(), "Sorry, no.");
    assert_eq!(seen, ["Sorry, no."]);
}

#[tokio::test]
async fn chatting_streams_from_the_writing_model_and_a_failure_restarts_it() {
    use crate::chat::{ChatModel, ChatRequest, Msg};
    let s = setup().await;
    let request = ChatRequest::new(vec![Msg::User("hi".into())], 20);
    // Not downloaded: nothing starts.
    assert_eq!(s.ai().chat(&request, &mut |_| true).await, Err(AiError::NotReady));

    let ai = downloaded(&s).await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500))
        .up_to_n_times(1)
        .mount(&s.server)
        .await;
    let sse = "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: [DONE]\n\n";
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(sse))
        .mount(&s.server)
        .await;
    assert!(matches!(ai.chat(&request, &mut |_| true).await, Err(AiError::Model(_))));
    let mut heard = String::new();
    let reply = ai
        .chat(&request, &mut |t| {
            heard.push_str(t);
            true
        })
        .await
        .unwrap();
    assert_eq!((reply.text.as_str(), heard.as_str()), ("Hello", "Hello"));
    assert_eq!(s.launcher.started.lock().unwrap().len(), 2, "started again after the failure");
}

/// A writing model's own download, from a status.
fn own(status: &Status, id: &str) -> ModelDownload {
    status.writers.iter().find(|w| w.id == id).unwrap().download.clone()
}

/// Serves `id` slowly, so there's time to look while it downloads.
async fn slow(s: &Setup, id: &str, n: usize) {
    Mock::given(method("GET"))
        .and(path(format!("/{id}")))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(s.files[n].1.clone()).set_delay(Duration::from_millis(300)))
        .with_priority(1)
        .mount(&s.server)
        .await;
}

async fn until(ai: &Ai, done: impl Fn(&Status) -> bool) {
    for _ in 0..300 {
        if done(&ai.status()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never happened: {:?}", ai.status());
}

#[tokio::test]
async fn each_writing_model_counts_only_its_own_file() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    let status = ai.status();
    // Sorting and search are in, but they're no part of a writing model.
    assert_eq!(own(&status, "big"), ModelDownload::Ready);
    assert_eq!(own(&status, "small"), ModelDownload::Missing { done: 0 });
    assert_eq!(own(&status, "huge"), ModelDownload::Missing { done: 0 });
    assert_eq!(status.embedder_download, ModelDownload::Ready);
    assert_eq!(status.decider_download, ModelDownload::Ready);

    std::fs::write(s.dir.path().join("ai/models/small.bin.part"), b"half").unwrap();
    assert_eq!(own(&ai.status(), "small"), ModelDownload::Missing { done: 4 }, "partly in: its own numbers");
    // Deleted means deleted: nothing of it is left, so nothing is partly in.
    let status = ai.remove_model("small").await.unwrap();
    assert_eq!(own(&status, "small"), ModelDownload::Missing { done: 0 });
    let status = ai.remove_model("big").await.unwrap();
    assert_eq!(own(&status, "big"), ModelDownload::Missing { done: 0 });
    assert_eq!(status.embedder_download, ModelDownload::Ready);

    let fresh = setup().await.ai().status();
    assert_eq!(fresh.embedder_download, ModelDownload::Missing { done: 0 });
    assert!(fresh.writers.iter().all(|w| w.download == ModelDownload::Missing { done: 0 }));
}

#[tokio::test]
async fn a_writing_model_downloads_on_its_own_with_its_own_progress() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    s.seen.lock().unwrap().clear();
    let task = ai.download_models(&["small".into()]).unwrap().unwrap();
    task.await.unwrap();
    let status = ai.status();
    assert_eq!(own(&status, "small"), ModelDownload::Ready);
    assert_eq!(status.writer, "small", "asked for and in: it writes now");
    assert_eq!(status.download, Download::Ready);
    // Its progress was its own, from nothing to all of it.
    let seen: Vec<u64> = s
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter_map(|st| match own(st, "small") {
            ModelDownload::Downloading { done } => Some(done),
            _ => None,
        })
        .collect();
    assert!(!seen.is_empty() && seen.windows(2).all(|w| w[0] <= w[1]), "{seen:?}");
    assert_eq!(seen.last(), Some(&s.catalog.writers[0].asset.bytes));
    assert!(s.seen.lock().unwrap().iter().all(|st| own(st, "big") == ModelDownload::Ready));
    // Nothing more to get; a model Moss doesn't know is an error.
    assert!(ai.download_models(&["small".into()]).unwrap().is_none());
    assert_eq!(ai.download_models(&["gpt".into()]).err(), Some(AiError::Choice("gpt".into())));
}

#[tokio::test]
async fn a_model_on_its_own_brings_the_runtime_it_needs() {
    let s = setup().await;
    s.serve_all().await;
    let ai = s.ai();
    ai.download_models(&["small".into()]).unwrap().unwrap().await.unwrap();
    assert!(s.dir.path().join("ai/runtime/llama-test/llama-server").is_file());
    assert_eq!(own(&ai.status(), "small"), ModelDownload::Ready);
    assert_eq!(ai.status().decider_download, ModelDownload::Missing { done: 0 }, "only what was asked for");

    ai.download_models(&["embedder".into(), "decider".into()]).unwrap().unwrap().await.unwrap();
    assert!(ai.embedder_ready() && ai.decider_ready());
}

#[tokio::test]
async fn choosing_a_model_leaves_a_download_running() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    slow(&s, "small", 3).await;
    let task = ai.download_models(&["small".into()]).unwrap().unwrap();
    until(&ai, |st| matches!(own(st, "small"), ModelDownload::Downloading { .. })).await;
    ai.choose_writer("big").await.unwrap();
    task.await.unwrap();
    assert_eq!(own(&ai.status(), "small"), ModelDownload::Ready, "it carried on");
}

#[tokio::test]
async fn asking_for_another_model_while_one_downloads_queues_it() {
    let s = setup().await;
    slow(&s, "big", 4).await;
    s.serve_all().await;
    let ai = s.ai();
    let task = ai.download().unwrap();
    assert!(ai.download_models(&["small".into()]).unwrap().is_none(), "the running download takes it");
    let status = ai.status();
    assert_eq!(own(&status, "small"), ModelDownload::Queued { done: 0 });
    assert!(matches!(status.download, Download::Downloading { .. }));
    task.await.unwrap();
    let status = ai.status();
    assert_eq!(own(&status, "small"), ModelDownload::Ready);
    assert_eq!(own(&status, "big"), ModelDownload::Ready);
}

#[tokio::test]
async fn pausing_one_model_lets_the_others_go_on() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    ai.remove_model("big").await.unwrap();
    slow(&s, "small", 3).await;
    let task = ai.download_models(&["small".into(), "big".into()]).unwrap().unwrap();
    until(&ai, |st| matches!(own(st, "small"), ModelDownload::Downloading { .. })).await;
    // Paused while waiting: it leaves the queue.
    assert_eq!(own(&ai.stop_model("big").unwrap(), "big"), ModelDownload::Missing { done: 0 });
    // Paused while downloading: what's in is kept, and the download moves on.
    ai.download_models(&["big".into()]).unwrap();
    let status = ai.stop_model("small").unwrap();
    task.await.unwrap();
    let status_after = ai.status();
    assert!(matches!(own(&status_after, "small"), ModelDownload::Missing { .. }), "{:?}", status);
    assert_eq!(own(&status_after, "big"), ModelDownload::Ready, "the next one went on");
    assert_eq!(ai.stop_model("gpt").err(), Some(AiError::Choice("gpt".into())));
}

#[tokio::test]
async fn a_failed_model_says_so_on_its_own_row_and_a_retry_clears_it() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    Mock::given(method("GET"))
        .and(path("/small"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&s.server)
        .await;
    ai.download_models(&["small".into()]).unwrap().unwrap().await.unwrap();
    let status = ai.status();
    assert_eq!(
        own(&status, "small"),
        ModelDownload::Failed { done: 0, message: "The download server answered 503. Try again later.".into() }
    );
    assert_eq!(own(&status, "big"), ModelDownload::Ready);
    // Deleting what came of it leaves no failure behind.
    assert_eq!(own(&ai.remove_model("small").await.unwrap(), "small"), ModelDownload::Missing { done: 0 });
    ai.download_models(&["small".into()]).unwrap().unwrap().await.unwrap();
    assert_eq!(own(&ai.status(), "small"), ModelDownload::Ready);
}

#[tokio::test]
async fn deleting_the_model_being_downloaded_lets_the_rest_go_on() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    ai.remove_model("big").await.unwrap();
    slow(&s, "small", 3).await;
    let task = ai.download_models(&["small".into(), "big".into()]).unwrap().unwrap();
    until(&ai, |st| matches!(own(st, "small"), ModelDownload::Downloading { .. })).await;
    ai.remove_model("small").await.unwrap();
    task.await.unwrap();
    let status = ai.status();
    assert_eq!(own(&status, "small"), ModelDownload::Missing { done: 0 });
    assert_eq!(own(&status, "big"), ModelDownload::Ready);
}

#[test]
fn a_finished_model_takes_over_when_asked_for_or_when_nothing_else_can_write() {
    // Asked for, and nothing chosen since: it writes.
    assert!(takes_over("small", Some("small"), true));
    // Something else chosen since (the ask was let go): the one chosen stays.
    assert!(!takes_over("small", None, true));
    assert!(!takes_over("small", Some("big"), true));
    // The model that wrote was deleted, so nothing on this Mac can write: it writes.
    assert!(takes_over("small", None, false));
}

#[tokio::test]
async fn a_model_you_download_writes_when_it_is_in_and_says_so() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    assert_eq!(ai.status().took_over, None);
    ai.download_models(&["small".into()]).unwrap().unwrap().await.unwrap();
    let status = ai.status();
    assert_eq!(status.writer, "small", "downloading it meant: use this one");
    assert_eq!(status.took_over.as_deref(), Some("small"));
    assert_eq!(s.ai().status().writer, "small", "saved for the next start");
    // Your next choice ends the note.
    assert_eq!(ai.choose_writer("big").await.unwrap().took_over, None);
}

#[tokio::test]
async fn a_choice_made_while_it_downloads_stays() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    slow(&s, "small", 3).await;
    let task = ai.download_models(&["small".into()]).unwrap().unwrap();
    until(&ai, |st| matches!(own(st, "small"), ModelDownload::Downloading { .. })).await;
    ai.choose_writer("big").await.unwrap();
    task.await.unwrap();
    let status = ai.status();
    assert_eq!(own(&status, "small"), ModelDownload::Ready);
    assert_eq!((status.writer.as_str(), status.took_over), ("big", None));
}

#[tokio::test]
async fn with_the_writer_deleted_the_next_model_in_writes() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    ai.remove_model("big").await.unwrap();
    // Asked for as part of something else, but nothing on this Mac could write: it does now.
    ai.download_models(&["embedder".into(), "small".into()]).unwrap();
    ai.choose_writer("big").await.unwrap();
    until(&ai, |st| own(st, "small") == ModelDownload::Ready && !matches!(st.download, Download::Downloading { .. })).await;
    let status = ai.status();
    assert_eq!((status.writer.as_str(), status.took_over.as_deref()), ("small", Some("small")));
    // The chosen model itself coming back has nothing to say.
    ai.choose_writer("big").await.unwrap();
    ai.download_models(&["big".into()]).unwrap().unwrap().await.unwrap();
    let status = ai.status();
    assert_eq!((status.writer.as_str(), status.took_over), ("big", None));
}

#[tokio::test]
async fn a_mac_with_apple_intelligence_writes_with_it_and_downloads_no_writing_model() {
    let s = with_apple().await;
    let ai = s.ai();
    let status = ai.status();
    assert_eq!(status.writer, "apple");
    let marks: Vec<_> = status.writers.iter().map(|w| (w.id.as_str(), w.recommended)).collect();
    assert_eq!(marks, [("apple", true), ("small", false), ("big", false), ("huge", false)]);
    assert_eq!(status.writers[0].download, ModelDownload::Ready);
    assert!(status.features.writing, "it writes with nothing downloaded");
    assert!(!status.features.search && !status.features.sorting);
    assert!(status.downloaded.is_empty(), "nothing of it to delete");
    // Everything this Mac runs well is sorting and search: no Gemma model.
    let essentials = s.catalog.runtime.bytes + s.catalog.embedder.bytes + s.catalog.decider.bytes;
    let plan = |p: Plan| status.plans.iter().find(|i| i.plan == p).unwrap().bytes;
    assert_eq!((plan(Plan::Recommended), plan(Plan::Essentials)), (essentials, essentials));
    assert_eq!(ai.context_tokens(), 8192);
    assert_eq!(crate::chat::ChatModel::context_tokens(&ai), 8192);

    s.serve_all().await;
    ai.download().unwrap().await.unwrap();
    assert_eq!(ai.status().download, Download::Ready);
    assert!(!ai.parts().dir.join("models").join(s.catalog.writers[1].asset.file).exists());
}

#[tokio::test]
async fn apple_intelligence_writes_through_its_helper() {
    let s = with_apple().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string("data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\ndata: [DONE]\n\n"))
        .mount(&s.server)
        .await;
    let ai = s.ai();
    assert_eq!(ai.write("system", "user", 10).await.unwrap(), "Hi");
    assert_eq!(*s.launcher.binaries.lock().unwrap(), [s.apple.clone().unwrap().binary]);
    assert_eq!(*s.launcher.started.lock().unwrap(), [(PathBuf::from("apple"), Role::Writer)]);
    // Running already: not started again.
    ai.write("system", "user", 10).await.unwrap();
    assert_eq!(s.launcher.started.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_gemma_model_can_write_instead_and_back() {
    let s = with_apple().await;
    let ai = downloaded(&s).await;
    let status = ai.choose_writer("small").await.unwrap();
    assert_eq!(status.writer, "small");
    assert!(!status.features.writing, "small isn't downloaded");
    assert_eq!(ai.context_tokens(), crate::agent::CONTEXT_TOKENS);
    // The choice is kept across a restart.
    assert_eq!(s.ai().status().writer, "small");
    let status = ai.choose_writer("apple").await.unwrap();
    assert_eq!(status.writer, "apple");
    assert!(status.features.writing);
    assert_eq!(s.ai().status().writer, "apple");
}

#[tokio::test]
async fn a_gemma_model_asked_for_while_apple_writes_takes_over_once_in() {
    let s = with_apple().await;
    s.serve_all().await;
    let ai = s.ai();
    ai.download_models(&["small".into()]).unwrap().unwrap().await.unwrap();
    let status = ai.status();
    assert_eq!(status.writer, "small");
    assert_eq!(status.took_over.as_deref(), Some("small"));
}

#[tokio::test]
async fn without_apple_intelligence_there_is_no_such_model() {
    let s = with_apple().await;
    s.ai().choose_writer("apple").await.unwrap();
    // Apple Intelligence was turned off since: Moss writes with the Gemma model this Mac runs best.
    let off = Setup { apple: None, ..setup().await };
    std::fs::create_dir_all(off.dir.path().join("ai")).unwrap();
    std::fs::copy(s.dir.path().join("ai/settings.json"), off.dir.path().join("ai/settings.json")).unwrap();
    let ai = off.ai();
    let status = ai.status();
    assert_eq!(status.writer, "big");
    assert!(status.writers.iter().all(|w| w.id != "apple"));
    assert!(matches!(ai.choose_writer("apple").await, Err(AiError::Choice(_))));
    assert!(matches!(ai.remove_model("apple").await, Err(AiError::Choice(_))));
}

#[tokio::test]
async fn models_are_reached_without_a_proxy_and_downloads_get_their_client_when_they_start() {
    let s = setup().await;
    s.serve_all().await;
    let ai = s.ai();
    assert!(!format!("{:?}", ai.http()).contains("proxies"), "the models are on this Mac");
    assert!(ai.0.downloads.get().is_none(), "building it can take seconds: never while Moss starts");
    ai.download().unwrap().await.unwrap();
    assert!(ai.0.downloads.get().is_some());
}

/// Each model, with a slow answer to what it's asked, and the way to ask it.
async fn slow_models(s: &Setup) {
    let slow = Duration::from_millis(300);
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(needs_reply_answer(0.9).set_delay(slow))
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(written("Nothing to do.").set_delay(slow))
        .mount(&s.server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/embeddings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [{"index": 0, "embedding": [0.5, 0.5]}]})).set_delay(slow))
        .mount(&s.server)
        .await;
}

async fn ask_all(ai: &Ai) {
    let texts = vec!["x".to_string()];
    let (decided, wrote, embedded) = tokio::join!(ai.needs_reply("x"), ai.ask_line("x"), ai.embed(&texts));
    decided.unwrap();
    wrote.unwrap();
    embedded.unwrap();
}

#[tokio::test]
async fn a_model_left_alone_is_stopped_but_never_mid_request_and_starts_again_when_needed() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    slow_models(&s).await;
    let live = || s.launcher.live.load(Ordering::Relaxed);
    let started = || s.launcher.started.lock().unwrap().len();
    let long_after = || tokio::time::Instant::now() + IDLE_UNLOAD * 2;
    tokio::join!(ask_all(&ai), async {
        // All three are answering now.
        tokio::time::sleep(Duration::from_millis(150)).await;
        ai.unload_idle(long_after()).await;
    });
    assert_eq!(live(), 3, "nothing was stopped while it answered");
    ai.unload_idle(tokio::time::Instant::now() + IDLE_UNLOAD - Duration::from_secs(1)).await;
    assert_eq!(live(), 3, "used a moment ago");
    ai.unload_idle(tokio::time::Instant::now() + IDLE_UNLOAD).await;
    assert_eq!(live(), 0, "left alone long enough, every model gives its memory back");
    ask_all(&ai).await;
    assert_eq!((live(), started()), (3, 6), "the next request starts each again");
}

#[tokio::test]
async fn moss_looks_for_models_left_alone_by_itself() {
    let s = setup().await;
    let ai = downloaded(&s).await;
    slow_models(&s).await;
    ai.embed(&["x".into()]).await.unwrap();
    tokio::time::pause();
    let live = || s.launcher.live.load(Ordering::Relaxed);
    tokio::time::sleep(IDLE_UNLOAD - IDLE_CHECK).await;
    assert_eq!(live(), 1);
    tokio::time::sleep(IDLE_CHECK * 2).await;
    assert_eq!(live(), 0);
    assert_eq!(IDLE_UNLOAD, Duration::from_secs(10 * 60));
    drop(ai);
    tokio::time::sleep(IDLE_CHECK * 2).await;
}
