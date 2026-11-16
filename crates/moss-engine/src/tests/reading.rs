//! Reading the text inside attachments end to end: in the background, on demand, the setting.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use super::*;
use crate::reading::{MAX_BYTES, RECENT_MS, THREAD_FILES};

/// Reads PDFs as "text of <name>: <bytes>"; anything else it doesn't read. A file called
/// "blank.pdf" has no words; "broken.pdf" can't be read.
struct FakeReader {
    paused: AtomicBool,
    read: Mutex<Vec<String>>,
    panics: std::sync::atomic::AtomicUsize,
}

impl Default for FakeReader {
    fn default() -> Self {
        FakeReader { paused: AtomicBool::new(false), read: Mutex::default(), panics: Default::default() }
    }
}

impl FakeReader {
    fn names(&self) -> Vec<String> {
        self.read.lock().unwrap().clone()
    }
}

impl Reader for FakeReader {
    fn may_run(&self) -> bool {
        bug(&self.panics);
        !self.paused.load(Ordering::Relaxed)
    }

    fn reads(&self, name: &str, _content_type: &str) -> bool {
        name.ends_with(".pdf")
    }

    fn read(&self, bytes: &[u8], name: &str, _content_type: &str) -> Option<String> {
        self.read.lock().unwrap().push(name.to_string());
        match name {
            "blank.pdf" => Some(String::new()),
            "broken.pdf" => None,
            _ => Some(format!("text of {name}: {}", String::from_utf8_lossy(bytes))),
        }
    }
}

/// An email from Daniel with `files` (name, contents) attached.
fn with(id: &str, subject: &str, at: i64, files: &[(&str, &str)]) -> Vec<u8> {
    let date = chrono::DateTime::from_timestamp_millis(at).unwrap().to_rfc2822();
    let mut raw = format!(
        "From: Daniel Okafor <daniel@northwind.co>\r\nTo: Maya Ross <maya.ross@gmail.com>\r\n\
         Subject: {subject}\r\nMessage-ID: <{id}>\r\nDate: {date}\r\n\
         Content-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: text/plain\r\n\r\nSee attached.\r\n"
    );
    for (name, contents) in files {
        raw.push_str(&format!(
            "--m\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename={name}\r\n\r\n{contents}\r\n"
        ));
    }
    raw.push_str("--m--\r\n");
    raw.into_bytes()
}

impl Setup {
    fn mail_with(&self, uid: u32, subject: &str, at: i64, files: &[(&str, &str)]) {
        self.fake.add(message("INBOX", uid, with(&format!("{uid}@x"), subject, at, files), at));
    }

    fn found(&self, query: &str) -> Vec<(String, Option<String>)> {
        let hits = self.engine.search(query, None, 10).unwrap();
        hits.into_iter().map(|h| (h.thread.subject, h.file.map(|f| f.name))).collect()
    }
}

#[tokio::test(start_paused = true)]
async fn a_reader_that_panics_starts_again_by_itself_after_a_pause() {
    let s = setup();
    s.mail_with(1, "Q4 signed", NOW - DAY, &[("Q4.pdf", "PDFDATA")]);
    s.gmail().await;
    s.synced().await;
    let reader = Arc::new(FakeReader::default());
    reader.panics.store(1, Ordering::SeqCst);
    s.engine.set_reader(reader.clone());
    settle().await;
    assert!(reader.names().is_empty());
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    assert_eq!(reader.names(), ["Q4.pdf"]);
}

#[tokio::test(start_paused = true)]
async fn recent_files_from_people_are_read_in_the_background_once() {
    let s = setup();
    s.fake.add(message("INBOX", 1, with_files("q4@x", "Q4 signed", NOW - DAY), NOW - DAY));
    s.mail_with(2, "Old letter", NOW - RECENT_MS - DAY, &[("old.pdf", "OLD")]);
    s.mail_with(3, "Blank", NOW - 2 * DAY, &[("blank.pdf", "x"), ("broken.pdf", "y"), ("notes.zip", "z")]);
    s.gmail().await;
    s.synced().await;
    assert!(s.engine.reading_attachments().unwrap());
    assert!(s.found("PDFDATA").is_empty(), "nothing read yet");
    let reader = Arc::new(FakeReader::default());
    s.engine.set_reader(reader.clone());
    settle().await;
    assert_eq!(reader.names(), ["Q4.pdf", "blank.pdf", "broken.pdf"], "newest first; the logo and the zip aren't read");
    assert_eq!(s.found("pdfdata"), [("Q4 signed".to_string(), Some("Q4.pdf".to_string()))]);
    let q4 = s.message_with_subject("Q4 signed");
    assert_eq!(s.engine.file_text(&q4.id, 1).await.unwrap().as_deref(), Some("text of Q4.pdf: PDFDATA"));
    // Each file is read once, even one with no words or that couldn't be read.
    s.engine.wake_reader();
    s.inbox(4, "n@x", "Newer", NOW);
    s.engine.sync_now(Some("gmail"));
    settle().await;
    settle().await;
    assert_eq!(reader.names().len(), 3);
    let blank = s.message_with_subject("Blank");
    assert_eq!(s.engine.file_text(&blank.id, 1).await.unwrap(), None);
    assert!(s.found("old").iter().all(|(_, file)| file.is_none()), "older mail waits until it's needed");
}

#[tokio::test(start_paused = true)]
async fn new_mail_is_read_as_it_arrives_and_its_text_is_found_by_meaning() {
    let s = setup();
    s.gmail().await;
    s.synced().await;
    let reader = Arc::new(FakeReader::default());
    s.engine.set_reader(reader.clone());
    let embedder = Arc::new(FakeEmbedder::default());
    s.engine.set_embedder(embedder.clone());
    s.mail_with(1, "Letter from the council", NOW, &[("P2026-0457.pdf", "your flight to Lisbon")]);
    s.engine.sync_now(Some("gmail"));
    settle().await;
    settle().await;
    settle().await;
    assert_eq!(reader.names(), ["P2026-0457.pdf"]);
    let hits = s.engine.find("travel plans", None, 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].file.as_ref().map(|f| f.name.as_str()), Some("P2026-0457.pdf"), "found by what the file means");
}

#[tokio::test(start_paused = true)]
async fn background_reading_waits_while_the_reader_says_so_and_while_its_off() {
    let s = setup();
    s.mail_with(1, "Lease", NOW - DAY, &[("lease.pdf", "rent")]);
    s.gmail().await;
    s.synced().await;
    let reader = Arc::new(FakeReader::default());
    reader.paused.store(true, Ordering::Relaxed);
    s.engine.set_reader(reader.clone());
    settle().await;
    assert!(reader.names().is_empty(), "on a low battery");
    s.engine.set_reading_attachments(false).unwrap();
    reader.paused.store(false, Ordering::Relaxed);
    s.engine.wake_reader();
    settle().await;
    assert!(reader.names().is_empty(), "turned off");
    let lease = s.message_with_subject("Lease");
    assert_eq!(s.engine.file_text(&lease.id, 0).await.unwrap(), None, "off: not even when asked");
    s.engine.set_reading_attachments(true).unwrap();
    settle().await;
    assert_eq!(reader.names(), ["lease.pdf"], "turning it on starts it again");
    // Turning it off forgets the text.
    s.engine.set_reading_attachments(false).unwrap();
    assert!(s.found("rent").is_empty());
}

#[tokio::test(start_paused = true)]
async fn background_reading_stops_as_soon_as_the_reader_says_to_wait() {
    let s = setup();
    s.mail_with(1, "Two", NOW - DAY, &[("a.pdf", "a"), ("b.pdf", "b")]);
    s.gmail().await;
    s.synced().await;
    /// Says to wait once it has read one file.
    #[derive(Default)]
    struct OneThenWait(FakeReader);
    impl Reader for OneThenWait {
        fn may_run(&self) -> bool {
            self.0.names().is_empty()
        }
        fn reads(&self, name: &str, content_type: &str) -> bool {
            self.0.reads(name, content_type)
        }
        fn read(&self, bytes: &[u8], name: &str, content_type: &str) -> Option<String> {
            self.0.read(bytes, name, content_type)
        }
    }
    let reader = Arc::new(OneThenWait::default());
    s.engine.set_reader(reader.clone());
    settle().await;
    assert_eq!(reader.0.names(), ["a.pdf"]);
}

#[tokio::test(start_paused = true)]
async fn a_file_the_server_wont_give_is_tried_again_next_time() {
    let s = setup();
    s.mail_with(1, "Lease", NOW - DAY, &[("lease.pdf", "rent"), ("deposit.pdf", "500")]);
    s.gmail().await;
    s.synced().await;
    let reader = Arc::new(FakeReader::default());
    s.fake.forget("INBOX", "1:1");
    s.engine.set_reader(reader.clone());
    settle().await;
    assert!(reader.names().is_empty(), "gone from the server: nothing to read, and it doesn't keep trying");
    s.mail_with(1, "Lease", NOW - DAY, &[("lease.pdf", "rent"), ("deposit.pdf", "500")]);
    s.engine.wake_reader();
    settle().await;
    assert_eq!(reader.names(), ["lease.pdf", "deposit.pdf"]);
}

#[tokio::test(start_paused = true)]
async fn ask_moss_reads_a_threads_files_when_it_needs_them() {
    let s = setup();
    s.mail_with(1, "Old letter", NOW - 200 * DAY, &[("old.pdf", "permit starts on 12 December"), ("notes.zip", "z")]);
    s.gmail().await;
    s.synced().await;
    let id = s.thread_id("Old letter");
    // Without a reader, a thread is as it was.
    let thread = s.engine.thread_with_files(&id).await.unwrap();
    assert_eq!(thread.messages[0].attachments[0].text, None);
    let reader = Arc::new(FakeReader::default());
    s.engine.set_reader(reader.clone());
    settle().await;
    assert!(reader.names().is_empty(), "too old for the background");
    s.fake.fail_next(moss_providers::Error::Network("down".into()));
    let thread = s.engine.thread_with_files(&id).await.unwrap();
    assert_eq!(thread.messages[0].attachments[0].text, None, "offline: no text, no error");
    let thread = s.engine.thread_with_files(&id).await.unwrap();
    let files = &thread.messages[0].attachments;
    assert_eq!(files[0].text.as_deref(), Some("text of old.pdf: permit starts on 12 December"));
    assert_eq!(files[1].text, None, "Moss doesn't read zips");
    // Read once: kept, and found by search.
    s.engine.thread_with_files(&id).await.unwrap();
    assert_eq!(reader.names(), ["old.pdf"]);
    assert_eq!(s.found("permit"), [("Old letter".to_string(), Some("old.pdf".to_string()))]);
    assert!(s.engine.thread_with_files("999").await.is_err());
}

#[tokio::test(start_paused = true)]
async fn a_thread_with_many_files_reads_only_a_few_at_once() {
    let s = setup();
    let names: Vec<String> = (0..THREAD_FILES + 2).map(|n| format!("page{n}.pdf")).collect();
    let files: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "p")).collect();
    s.mail_with(1, "Scans", NOW - 200 * DAY, &files);
    s.gmail().await;
    s.synced().await;
    let reader = Arc::new(FakeReader::default());
    s.engine.set_reader(reader.clone());
    let id = s.thread_id("Scans");
    let thread = s.engine.thread_with_files(&id).await.unwrap();
    let read = thread.messages[0].attachments.iter().filter(|a| a.text.is_some()).count();
    assert_eq!(read, THREAD_FILES);
    // Next time, the ones read are kept and the rest are read.
    let thread = s.engine.thread_with_files(&id).await.unwrap();
    assert!(thread.messages[0].attachments.iter().all(|a| a.text.is_some()));
}

#[tokio::test(start_paused = true)]
async fn a_file_is_read_on_demand_only_when_moss_reads_its_kind_and_it_isnt_too_big() {
    let s = setup();
    s.mail_with(1, "Files", NOW - 200 * DAY, &[("notes.zip", "z"), ("big.pdf", "b")]);
    s.gmail().await;
    s.synced().await;
    let m = s.message_with_subject("Files");
    assert_eq!(s.engine.file_text(&m.id, 1).await.unwrap(), None, "no reader yet");
    let reader = Arc::new(FakeReader::default());
    s.engine.set_reader(reader.clone());
    assert_eq!(s.engine.file_text(&m.id, 0).await.unwrap(), None, "a zip");
    let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
    db.execute("UPDATE attachments SET bytes = ?1 WHERE position = 1", [MAX_BYTES as i64 + 1]).unwrap();
    assert_eq!(s.engine.file_text(&m.id, 1).await.unwrap(), None, "too big");
    assert!(matches!(s.engine.file_text(&m.id, 9).await, Err(Error::Unavailable(_))));
    assert!(reader.names().is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_file_the_server_cant_give_on_demand_is_an_error() {
    let s = setup();
    s.mail_with(1, "Lease", NOW - 200 * DAY, &[("lease.pdf", "rent")]);
    s.gmail().await;
    s.synced().await;
    s.engine.set_reader(Arc::new(FakeReader::default()));
    let m = s.message_with_subject("Lease");
    s.fake.fail_next(moss_providers::Error::Network("down".into()));
    assert!(matches!(s.engine.file_text(&m.id, 0).await, Err(Error::Offline(_))));
}

/// Gives messages their vectors; for files it's down, or gives the wrong number.
struct NoFileVectors;

#[async_trait::async_trait]
impl Embedder for NoFileVectors {
    fn model(&self) -> String {
        "fake-embed".into()
    }

    async fn documents(&self, emails: &[moss_store::ToEmbed]) -> Option<Vec<Vec<f32>>> {
        match emails[0].subject.as_str() {
            "down.pdf" => None,
            "wrong.pdf" => Some(vec![]),
            _ => Some(emails.iter().map(|_| vec![1.0, 0.0]).collect()),
        }
    }

    async fn query(&self, _text: &str) -> Option<Vec<f32>> {
        Some(vec![1.0, 0.0])
    }
}

#[tokio::test(start_paused = true)]
async fn files_wait_for_their_vectors_while_the_model_cant_make_them_or_they_cant_be_saved() {
    for name in ["down.pdf", "wrong.pdf", "unsaved.pdf"] {
        let s = setup();
        s.mail_with(1, "Letter", NOW - DAY, &[(name, "permit")]);
        s.gmail().await;
        s.synced().await;
        if name == "unsaved.pdf" {
            let db = rusqlite::Connection::open(s._dir.path().join("moss.db")).unwrap();
            db.execute_batch("CREATE TRIGGER full BEFORE INSERT ON attachment_embeddings BEGIN SELECT RAISE(FAIL, 'full'); END").unwrap();
        }
        s.engine.set_reader(Arc::new(FakeReader::default()));
        s.engine.set_embedder(Arc::new(NoFileVectors));
        settle().await;
        settle().await;
        let hits = s.engine.find("anything", None, 10).await.unwrap();
        assert!(hits.iter().all(|h| h.file.is_none()), "{name}: the file has no vector");
        assert_eq!(hits.len(), 1, "{name}: the email has one");
    }
}

struct Stalled {
    path: std::path::PathBuf,
    message: String,
    entered: Mutex<Option<tokio::sync::oneshot::Sender<bool>>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}
impl Reader for Stalled {
    fn may_run(&self) -> bool { true }
    fn reads(&self, _: &str, _: &str) -> bool { true }
    fn read(&self, _: &[u8], _: &str, _: &str) -> Option<String> {
        let db = rusqlite::Connection::open(&self.path).unwrap();
        let recorded: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM attachment_text WHERE message_id=?1 AND position=0)", [&self.message], |row| row.get(0)).unwrap();
        self.entered.lock().unwrap().take().unwrap().send(recorded).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Some("late text must not overwrite the abandoned attempt".into())
    }
}

#[tokio::test(start_paused = true)]
async fn extraction_is_recorded_before_native_code_and_a_stalled_reader_releases_its_caller() {
    let s = setup();
    s.mail_with(1, "Stalled extraction", NOW, &[("stalled.pdf", "fixture")]);
    s.gmail().await;
    s.synced().await;
    let message = s.message_with_subject("Stalled extraction");
    let (entered, ready) = tokio::sync::oneshot::channel();
    let (release, released) = std::sync::mpsc::channel();
    *s.engine.shared.reader.lock().unwrap() = Some(Arc::new(Stalled {path:s._dir.path().join("moss.db"), message:message.id.clone(), entered:Mutex::new(Some(entered)), release:Mutex::new(released)}));
    let engine = s.engine.clone();
    let id = message.id.clone();
    let task = tokio::spawn(async move { engine.file_text(&id, 0).await });
    let recorded_before_read = ready.await.unwrap();
    tokio::time::advance(std::time::Duration::from_secs(31)).await;
    for _ in 0..10 { tokio::task::yield_now().await; }
    let completed_before_release = task.is_finished();
    let slots_while_stalled = s.engine.shared.attachment_read_slots.available_permits();
    // Always release the native stand-in before asserting, including a regression.
    release.send(()).unwrap();
    let result = task.await.unwrap().unwrap();
    assert!(recorded_before_read, "an extraction attempt must be committed before native parsing");
    assert!(completed_before_release, "a stalled native reader must not hold its caller indefinitely");
    assert_eq!(slots_while_stalled, 1, "a detached native reader must keep its concurrency slot");
    assert_eq!(result, None);
    assert_eq!(s.engine.file_text(&message.id, 0).await.unwrap(), None);
    let reopened = moss_store::Store::open(s._dir.path().join("moss.db")).unwrap();
    assert_eq!(reopened.attachment_text(&message.id, 0).unwrap(), None);
    assert!(reopened.to_read(0, MAX_BYTES, &|_, _| true, &[], 10).unwrap().is_empty());
}

#[tokio::test(start_paused = true)]
async fn occupied_native_read_slots_do_not_start_or_discard_another_file() {
    let s = setup();
    s.mail_with(1, "Waiting extraction", NOW, &[("waiting.pdf", "fixture")]);
    s.gmail().await;
    s.synced().await;
    let message = s.message_with_subject("Waiting extraction");
    let reader = Arc::new(FakeReader::default());
    *s.engine.shared.reader.lock().unwrap() = Some(reader.clone());
    let slots = s.engine.shared.attachment_read_slots.acquire_many(2).await.unwrap();
    assert!(s.engine.file_text(&message.id, 0).await.is_err());
    assert!(reader.names().is_empty());
    assert_eq!(s.engine.shared.store().attachment_text(&message.id, 0).unwrap(), None);
    drop(slots);
    assert!(s.engine.file_text(&message.id, 0).await.unwrap().unwrap().contains("fixture"));
    assert_eq!(reader.names(), ["waiting.pdf"]);
}

#[tokio::test(start_paused = true)]
async fn late_extraction_cannot_restore_disabled_text_or_replace_a_new_attempt() {
    for restart in [false, true] {
        let s = setup();
        s.mail_with(1, "Reading preference", NOW, &[("waiting.pdf", "fixture")]);
        s.gmail().await;
        s.synced().await;
        let message = s.message_with_subject("Reading preference");
        let (entered, ready) = tokio::sync::oneshot::channel();
        let (release, released) = std::sync::mpsc::channel();
        *s.engine.shared.reader.lock().unwrap() = Some(Arc::new(Stalled {path:s._dir.path().join("moss.db"), message:message.id.clone(), entered:Mutex::new(Some(entered)), release:Mutex::new(released)}));
        let engine = s.engine.clone();
        let id = message.id.clone();
        let task = tokio::spawn(async move { engine.file_text(&id, 0).await });
        ready.await.unwrap();
        s.engine.set_reading_attachments(false).unwrap();
        let expected = if restart {
            *s.engine.shared.reader.lock().unwrap() = Some(Arc::new(FakeReader::default()));
            s.engine.set_reading_attachments(true).unwrap();
            s.engine.file_text(&message.id, 0).await.unwrap()
        } else { None };
        release.send(()).unwrap();
        let late = task.await.unwrap().unwrap();
        assert_eq!(late, None, "an obsolete native result must be discarded");
        assert_eq!(s.engine.shared.store().attachment_text(&message.id, 0).unwrap(), expected);
    }
}
