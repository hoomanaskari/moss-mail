//! Search by meaning: the embedding model makes a vector for every message in the
//! background (newest first, older mail as soon as the model arrives), and search mixes
//! the nearest in with the keyword matches. The vectors stay on this device.

use std::sync::Arc;

use async_trait::async_trait;
use moss_core::SearchHit;
use moss_store::ToEmbed;

use crate::{Engine, Result, Shared};

/// The embedding model, as the engine sees it. The app passes its AI; tests pass a fake.
#[async_trait]
pub trait Embedder: Send + Sync {
    /// Which model makes the vectors; saved with each, so a new model makes them again.
    fn model(&self) -> String;
    /// One vector per message, or `None` while the model can't run.
    async fn documents(&self, emails: &[ToEmbed]) -> Option<Vec<Vec<f32>>>;
    /// The vector for what someone searched for, or `None` while the model can't run.
    async fn query(&self, text: &str) -> Option<Vec<f32>>;
}

const PER_ROUND: usize = 32;
/// Mail must mean at least this nearly what was searched for to count as a match.
pub const LEAST_ALIKE: f64 = 0.35;

async fn background_search(
    gate: Arc<tokio::sync::Semaphore>,
    search: impl FnOnce() -> moss_store::Result<Vec<SearchHit>> + Send + 'static,
) -> Result<Vec<SearchHit>> {
    let permit = gate.acquire_owned().await
        .map_err(|_| crate::Error::Invalid("Search is shutting down.".into()))?;
    Ok(tokio::task::spawn_blocking(move || {
        let _permit = permit;
        search()
    }).await
        .map_err(|_| crate::Error::Invalid("The search worker could not finish.".into()))??)
}

#[cfg(test)]
mod worker_tests {
    use super::*;

    #[tokio::test]
    async fn abandoning_a_search_keeps_its_capacity_reserved_until_the_worker_stops() {
        let gate = Arc::new(tokio::sync::Semaphore::new(1));
        let (started, running) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let task = tokio::spawn(background_search(gate.clone(), move || {
            started.send(()).unwrap();
            let _ = wait.recv();
            Ok(Vec::new())
        }));
        running.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(gate.available_permits(), 0, "cancelling an awaiter must not allow overlapping scans");
        release.send(()).unwrap();
        assert!(background_search(gate.clone(), || Ok(Vec::new())).await.unwrap().is_empty());
        assert_eq!(gate.available_permits(), 1);
    }

    #[tokio::test]
    async fn search_worker_failures_release_capacity_and_report_fixed_errors() {
        let gate = Arc::new(tokio::sync::Semaphore::new(1));
        let failed = background_search(gate.clone(), || panic!("private worker fixture")).await.unwrap_err();
        assert_eq!(failed.to_string(), "The search worker could not finish.");
        assert_eq!(gate.available_permits(), 1);
        assert!(background_search(gate.clone(), || Err(moss_store::Error::Invalid("query refused".into()))).await.is_err());
        assert_eq!(gate.available_permits(), 1);
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = called.clone();
        let work = move || { observed.store(true, std::sync::atomic::Ordering::SeqCst); Ok(Vec::new()) };
        assert!(background_search(gate.clone(), work.clone()).await.unwrap().is_empty());
        assert!(called.swap(false, std::sync::atomic::Ordering::SeqCst));
        gate.close();
        let failed = background_search(gate, work).await.unwrap_err();
        assert_eq!(failed.to_string(), "Search is shutting down.");
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
    }
}

impl Shared {
    pub(crate) fn with_meaning_reader<T>(&self, scan: impl FnOnce(Option<moss_store::MeaningReader>) -> moss_store::Result<T>) -> moss_store::Result<T> {
        let reader = self.store().meaning_reader()?;
        scan(reader)
    }

    /// Makes vectors for every message waiting, newest first, then for the text read from
    /// files. Stops when none are left, or the model can't run.
    pub(crate) async fn embed_waiting(&self, embedder: &dyn Embedder) {
        let model = embedder.model();
        let mut cursor = None;
        loop {
            let (batch,next)=match self.store().unembedded_after(&model,PER_ROUND,cursor) {
                Ok(batch)=>batch,
                Err(_)=>{eprintln!("Moss: message indexing paused because its queue could not be read.");return;}
            };
            if batch.is_empty() {
                return self.embed_files(embedder, &model).await;
            }
            let Some(vectors) = embedder.documents(&batch).await else { return };
            if vectors.len() != batch.len() {
                return;
            }
            for (email, vector) in batch.iter().zip(&vectors) {
                // Can't save it: try again the next time mail changes.
                if self.store().set_embedding_if_current(email, &model, vector).is_err() {
                    return;
                }
            }
            cursor = next;
        }
    }
}

impl Shared {
    async fn embed_files(&self, embedder: &dyn Embedder, model: &str) {
        let mut cursor = None;
        loop {
            let (batch,next)=match self.store().unembedded_files_after(model,PER_ROUND,cursor) {
                Ok(batch)=>batch,
                Err(_)=>{eprintln!("Moss: attachment indexing paused because its queue could not be read.");return;}
            };
            if batch.is_empty() {
                return;
            }
            let emails: Vec<ToEmbed> = batch.iter().map(|f| f.email.clone()).collect();
            let Some(vectors) = embedder.documents(&emails).await else { return };
            if vectors.len() != batch.len() {
                return;
            }
            for (file, vector) in batch.iter().zip(&vectors) {
                if self.store().set_file_embedding_if_current(file, model, vector).is_err() {
                    return;
                }
            }
            cursor = next;
        }
    }
}

impl Engine {
    /// Lets the embedding model index mail by meaning, from now on, in the background; and
    /// search use it.
    pub fn set_embedder(&self, embedder: Arc<dyn Embedder>) {
        *crate::lock(&self.shared.embedder) = Some(embedder.clone());
        let shared = self.shared.clone();
        crate::keep_running(move || {
            let (shared, embedder) = (shared.clone(), embedder.clone());
            async move {
                loop {
                    shared.embed_waiting(embedder.as_ref()).await;
                    shared.meaning_wake.notified().await;
                }
            }
        });
    }

    /// Indexes by meaning again now, e.g. once the embedding model has downloaded.
    pub fn wake_embedder(&self) {
        self.shared.meaning_wake.notify_one();
    }

    /// Search by words, then by meaning when the embedding model can run: threads that match
    /// the words come first, then ones that mean what was searched for.
    pub async fn find(&self, query: &str, account: Option<&str>, limit: usize) -> Result<Vec<SearchHit>> {
        let mut hits = self.search(query, account, limit)?;
        let embedder = crate::lock(&self.shared.embedder).clone();
        let Some(embedder) = embedder.filter(|_| hits.len() < limit && !query.trim().is_empty()) else {
            return Ok(hits);
        };
        let Some(vector) = embedder.query(query).await else {
            return Ok(hits);
        };
        let model = embedder.model();
        let now = self.shared.now();
        let shared = self.shared.clone();
        let account = account.map(str::to_owned);
        let near = background_search(self.shared.meaning_search_slot.clone(), move || {
            // Opening/pinning is short; scanning and hydration never own the writer lock.
            // Acquire the job slot first so queued searches do not pin idle WAL snapshots.
            shared.with_meaning_reader(|reader| match reader {
                Some(reader) => reader.search(&model, &vector, account.as_deref(), LEAST_ALIKE, limit, now),
                None => shared.store().meaning_search(&model, &vector, account.as_deref(), LEAST_ALIKE, limit, now),
            })
        }).await?;
        for hit in near {
            if hits.len() < limit && !hits.iter().any(|h| h.thread.id == hit.thread.id) {
                hits.push(hit);
            }
        }
        Ok(hits)
    }
}
