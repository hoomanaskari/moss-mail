//! Text the writing model is writing, sent to the window as it grows, for any job that
//! streams (replies, quick replies, summaries, answers about your mail). Jobs that write in
//! parts (quick replies, summaries) send those parts as JSON text. The UI names each stream
//! with an id of its choosing, hears it as `moss://ai-text` events, and can stop it with `ai_stop`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Runtime, State};

/// The window event each new text is sent as: `{id, text}`.
pub const TEXT_EVENT: &str = "moss://ai-text";

/// One stream's text so far.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Text {
    pub id: String,
    pub text: String,
}

/// The streams running now, by id, each with its "stop" flag.
#[derive(Default)]
pub struct Writing(Mutex<HashMap<String, Arc<AtomicBool>>>);

impl Writing {
    /// Starts stream `id`; it ends when the [`Stream`] is dropped.
    pub fn begin<R: Runtime>(&self, app: &AppHandle<R>, id: &str) -> Stream<'_, R> {
        let stop = Arc::new(AtomicBool::new(false));
        self.0.lock().unwrap().insert(id.to_string(), stop.clone());
        Stream { writing: self, app: app.clone(), id: id.to_string(), stop }
    }

    /// Stops stream `id`. `false` when it isn't running.
    pub fn stop(&self, id: &str) -> bool {
        match self.0.lock().unwrap().get(id) {
            Some(stop) => {
                stop.store(true, Ordering::Relaxed);
                true
            }
            None => false,
        }
    }
}

/// A stream that's running. Give the job an `on_text` that calls [`Stream::send`].
pub struct Stream<'w, R: Runtime> {
    writing: &'w Writing,
    app: AppHandle<R>,
    id: String,
    stop: Arc<AtomicBool>,
}

impl<R: Runtime> Stream<'_, R> {
    /// Sends `text` to the window (send the whole text so far, so the UI can show it as it
    /// is). `false` once someone has stopped the stream.
    pub fn send(&self, text: &str) -> bool {
        let _ = self.app.emit(TEXT_EVENT, Text { id: self.id.clone(), text: text.to_string() });
        !self.stop.load(Ordering::Relaxed)
    }
}

impl<R: Runtime> Drop for Stream<'_, R> {
    fn drop(&mut self) {
        let mut running = self.writing.0.lock().unwrap();
        // A newer stream may have taken the id since.
        if running.get(&self.id).is_some_and(|s| Arc::ptr_eq(s, &self.stop)) {
            running.remove(&self.id);
        }
    }
}

/// Stops a stream; what it wrote so far stays.
#[tauri::command]
pub fn ai_stop(writing: State<'_, Writing>, id: String) -> bool {
    writing.stop(&id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::test::{mock_builder, mock_context, noop_assets};
    use tauri::Listener;

    #[test]
    fn text_reaches_the_window_until_the_stream_is_stopped() {
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        let heard = Arc::new(Mutex::new(Vec::<String>::new()));
        let h = heard.clone();
        app.listen(TEXT_EVENT, move |e| h.lock().unwrap().push(e.payload().to_string()));
        let writing = Writing::default();
        let stream = writing.begin(app.handle(), "r1");
        assert!(stream.send("Hi"));
        assert!(writing.stop("r1"));
        assert!(!stream.send("Hi Dana"));
        assert_eq!(
            *heard.lock().unwrap(),
            [r#"{"id":"r1","text":"Hi"}"#, r#"{"id":"r1","text":"Hi Dana"}"#]
        );
        // Done: nothing left to stop.
        drop(stream);
        assert!(!writing.stop("r1"));
    }

    #[test]
    fn a_newer_stream_with_the_same_id_keeps_running() {
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        let writing = Writing::default();
        let older = writing.begin(app.handle(), "r1");
        let newer = writing.begin(app.handle(), "r1");
        drop(older);
        assert!(writing.stop("r1"));
        assert!(!newer.send("x"));
    }
}
