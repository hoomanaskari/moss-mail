//! Reading the text inside attachments, so search finds words in them and Ask Moss can
//! answer from them. The file is fetched from the server (and kept only as cache), read on
//! this Mac, and only its text is kept (see `moss_store::Store::keep_attachment_text`).
//!
//! In the background: files people sent lately (Important and Other, the last
//! [`RECENT_MS`]), newest first, while [`Reader::may_run`] says so (not on a low battery or
//! a phone's hotspot). Everything else is read when it's needed: when Ask Moss reads a
//! thread, or a file ([`Engine::file_text`]). Each file is read once.

use std::sync::Arc;

use moss_core::Thread;

use crate::{Engine, Result, Shared};

/// Reads the text in files. The app passes PDFKit, Vision and the Office readers from
/// `moss-extract`, and the AI's power check; tests pass a fake.
pub trait Reader: Send + Sync {
    /// Whether background reading may run now: not on a low battery or a phone's hotspot.
    fn may_run(&self) -> bool;
    /// Whether it reads files like this one, by name and content type.
    fn reads(&self, name: &str, content_type: &str) -> bool;
    /// The text in a file; `None` if it can't be read. Slow (text recognition): it runs on
    /// a blocking thread.
    fn read(&self, bytes: &[u8], name: &str, content_type: &str) -> Option<String>;
}

/// How far back background reading goes.
pub const RECENT_MS: i64 = 30 * 86_400_000;
/// Files bigger than this aren't fetched to read them: a 25 MB attachment is Gmail's limit.
pub const MAX_BYTES: u64 = 25 * 1024 * 1024;
/// Files read on demand per thread, so a thread of many attachments doesn't hold Ask Moss up.
pub const THREAD_FILES: usize = 6;

const PER_ROUND: usize = 10;

impl Shared {
    fn reader(&self) -> Option<Arc<dyn Reader>> {
        crate::lock(&self.reader).clone()
    }

    /// Reads every file waiting in the background, newest first. Stops when none are left,
    /// reading is turned off, or the reader says to wait.
    pub(crate) async fn read_waiting(self: &Arc<Self>, reader: &Arc<dyn Reader>) {
        // Files the server wouldn't give this time; tried again next time.
        let mut failed: Vec<(String, u32)> = Vec::new();
        loop {
            if !reader.may_run() || !self.store().reading_attachments().unwrap_or(false) {
                return;
            }
            let since = self.now() - RECENT_MS;
            let reads = |name: &str, content_type: &str| reader.reads(name, content_type);
            let Ok(batch) = self.store().to_read(since, MAX_BYTES, &reads, &failed, PER_ROUND) else { return };
            if batch.is_empty() {
                return;
            }
            for file in batch {
                if !reader.may_run() {
                    return;
                }
                match self.read_file(reader, &file.message_id, file.index, &file.name, &file.content_type).await {
                    Ok(_) => self.meaning_wake.notify_one(),
                    Err(_) => failed.push((file.message_id, file.index)),
                }
            }
        }
    }

    /// Fetches a file, reads it and keeps its text: empty when it has no words or can't be
    /// read, so it isn't read again.
    async fn read_file(&self, reader: &Arc<dyn Reader>, message_id: &str, index: u32, name: &str, content_type: &str) -> Result<Option<String>> {
        let bytes = self.attachment_bytes(message_id, index).await?;
        let slot = self.attachment_read_slots.clone().try_acquire_owned()
            .map_err(|_| crate::Error::Unavailable("Attachment reading is busy. Try again later.".into()))?;
        let token = {
            let mut store = self.store();
            let Some(token) = store.begin_attachment_read(message_id, index)? else {
                return Ok(store.attachment_text(message_id, index)?);
            };
            token
        };
        let (reader, name, content_type) = (reader.clone(), name.to_string(), content_type.to_string());
        let task = tokio::task::spawn_blocking(move || {
            // A timed-out task retains its slot until native code really returns.
            let _slot = slot;
            reader.read(&bytes, &name, &content_type)
        });
        let text = match tokio::time::timeout(std::time::Duration::from_secs(30), task).await {
            Ok(Ok(Some(text))) => text,
            _ => return Ok(None),
        };
        Ok(self.store().finish_attachment_read(message_id, index, &token, &text)?.then_some(text))
    }
}

impl Engine {
    /// Lets the engine read the text inside attachments, from now on: in the background,
    /// and when Ask Moss needs a file.
    pub fn set_reader(&self, reader: Arc<dyn Reader>) {
        *crate::lock(&self.shared.reader) = Some(reader.clone());
        let shared = self.shared.clone();
        crate::keep_running(move || {
            let (shared, reader) = (shared.clone(), reader.clone());
            async move {
                loop {
                    shared.read_waiting(&reader).await;
                    shared.reading_wake.notified().await;
                }
            }
        });
    }

    /// Reads in the background again now, e.g. once the Mac is back on power.
    pub fn wake_reader(&self) {
        self.shared.reading_wake.notify_one();
    }

    /// Whether Moss reads the text inside attachments (Settings → Storage). On by default.
    #[cfg(test)]
    pub fn reading_attachments(&self) -> Result<bool> {
        Ok(self.shared.store().reading_attachments()?)
    }

    /// Turns reading attachments on (it starts again in the background) or off (the text
    /// kept is forgotten).
    pub fn set_reading_attachments(&self, on: bool) -> Result<()> {
        self.shared.store().set_reading_attachments(on)?;
        if on {
            self.wake_reader();
        }
        Ok(())
    }

    /// The text in attachment `index`, reading it now if it hasn't been: `None` when reading
    /// is off, there's no reader, Moss doesn't read files like it, it's too big, or the
    /// server can't give it.
    pub async fn file_text(&self, message_id: &str, index: u32) -> Result<Option<String>> {
        let unavailable = || crate::Error::Unavailable(format!("attachment {index} of message {message_id}"));
        let file = self.shared.store().attachment(message_id, index)?.ok_or_else(unavailable)?;
        if let Some(text) = self.shared.store().attachment_text(message_id, index)? {
            return Ok(Some(text));
        }
        let Some(reader) = self.shared.reader() else { return Ok(None) };
        if !self.shared.store().reading_attachments()? || file.bytes > MAX_BYTES || !reader.reads(&file.name, &file.content_type) {
            return Ok(None);
        }
        let text = self.shared.read_file(&reader, message_id, index, &file.name, &file.content_type).await?;
        self.shared.meaning_wake.notify_one();
        Ok(text)
    }

    /// A thread with the text of its files (see `moss_core::Attachment::text`), for Ask Moss:
    /// what's been read, and up to [`THREAD_FILES`] more read now. A file the server can't
    /// give stays without text.
    pub async fn thread_with_files(&self, id: &str) -> Result<Thread> {
        let mut thread = self.thread(id)?;
        self.shared.store().add_file_texts(&mut thread)?;
        let mut read = 0;
        for message in &mut thread.messages {
            for file in message.attachments.iter_mut().filter(|a| !a.inline && a.text.is_none()) {
                if read == THREAD_FILES {
                    return Ok(thread);
                }
                if let Ok(Some(text)) = self.file_text(&message.id, file.index).await {
                    file.text = Some(text);
                    read += 1;
                }
            }
        }
        Ok(thread)
    }
}
