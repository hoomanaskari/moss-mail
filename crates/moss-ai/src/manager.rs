//! The AI on this Mac: the writing model you chose, getting the files, and running both models.
//!
//! Files live in one folder (`<app data>/ai`):
//! - `settings.json`: the writing model you picked.
//! - `models/`: the decision model and writing models, checked by SHA-256.
//! - `runtime/`: llama.cpp, unpacked.
//! - `run/`: each running model's key.
//!
//! Nothing downloads until [`Ai::download`] is called: from the Download button, or the first
//! time you use an AI feature. Each model starts the first time a job needs it.
//!
//! On a Mac with Apple Intelligence, it can write instead of a Gemma model: built in, so
//! nothing downloads for it (see `apple`).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::apple::{self, Apple};
use crate::catalog::{self, Asset, ModelDownload, Writer, WriterInfo};
use crate::decide::{self, Answer, Question};
use crate::download::{self, DownloadError, Request};
use crate::server::{Endpoint, Server, ServerError};
use crate::{prompt, AiError, Machine};
use moss_core::Thread;

/// Which job a model does. They start with different settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Decider,
    Writer,
    Embedder,
}

impl Role {
    /// The decision model reads the whole prompt in one batch, and keeps it cached between
    /// questions about the same email.
    fn flags(self) -> &'static [&'static str] {
        match self {
            Role::Decider => &[
                "-np",
                "1",
                "-c",
                "8192",
                "-b",
                "2048",
                "-ub",
                "2048",
                "--cache-ram",
                "256",
            ],
            Role::Writer => &["-np", "1", "-c", "32768"],
            Role::Embedder => &["--embeddings", "-np", "1", "-c", "2048", "-b", "2048", "-ub", "2048"],
        }
    }

    fn name(self) -> &'static str {
        match self {
            Role::Decider => "decider",
            Role::Writer => "writer",
            Role::Embedder => "embedder",
        }
    }
}

/// A model that is running and answering.
pub trait Running: Send + Sync {
    fn endpoint(&self) -> Endpoint;
}

impl Running for Server {
    fn endpoint(&self) -> Endpoint {
        Server::endpoint(self).clone()
    }
}

/// Starts a model. The app uses [`Llama`]; tests pass a fake.
#[async_trait]
pub trait Launch: Send + Sync {
    async fn launch(
        &self,
        binary: &Path,
        model: &Path,
        role: Role,
        key_file: &Path,
    ) -> Result<Box<dyn Running>, ServerError>;
}

/// Runs models in llama.cpp's `llama-server`.
pub struct Llama;

/// Loading the biggest model from a slow disk can take a while.
const LOAD_WAIT: Duration = Duration::from_secs(180);

#[async_trait]
impl Launch for Llama {
    async fn launch(
        &self,
        binary: &Path,
        model: &Path,
        role: Role,
        key_file: &Path,
    ) -> Result<Box<dyn Running>, ServerError> {
        Ok(Box::new(
            Server::start(binary, model, role.flags(), key_file, LOAD_WAIT).await?,
        ))
    }
}

/// The files Moss can download. The app uses [`Catalog::pinned`]; tests point it elsewhere.
#[derive(Debug, Clone)]
pub struct Catalog {
    pub runtime: Asset,
    /// The folder the runtime archive unpacks into.
    pub runtime_dir: &'static str,
    pub embedder: Asset,
    pub decider: Asset,
    pub writers: Vec<Writer>,
}

impl Catalog {
    pub fn pinned() -> Catalog {
        Catalog {
            runtime: catalog::RUNTIME,
            runtime_dir: catalog::RUNTIME_DIR,
            embedder: catalog::EMBEDDER,
            decider: catalog::DECIDER,
            writers: catalog::WRITERS.to_vec(),
        }
    }
}

/// Where the download stands.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Download {
    /// Not (all) downloaded. `done` is what's on disk already, so a download resumes there.
    #[serde(rename_all = "camelCase")]
    Missing { done: u64, total: u64 },
    #[serde(rename_all = "camelCase")]
    Downloading { done: u64, total: u64 },
    #[serde(rename_all = "camelCase")]
    Failed {
        done: u64,
        total: u64,
        message: String,
    },
    /// Every file is in and checked.
    Ready,
}

/// What to get: everything this Mac runs well, the small models only, or nothing for now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Plan {
    /// Search by meaning, sorting, and the writing model for summaries and drafts.
    Recommended,
    /// Search by meaning and sorting.
    Essentials,
    /// Nothing yet. Settings → AI has it later.
    Skip,
}

/// A plan as the first-run step offers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanInfo {
    pub plan: Plan,
    pub bytes: u64,
    /// e.g. "About 6.4 GB".
    pub size: String,
}

/// What the AI can do now: each part works as soon as its model is in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Features {
    /// Search by meaning (the embedding model).
    pub search: bool,
    /// Sorting and reading mail (the decision model).
    pub sorting: bool,
    /// Summaries, the ask and drafts (the writing model).
    pub writing: bool,
}

pub use crate::power::Pause;

/// Everything Settings → AI and the first-run step show.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// e.g. "M4 Max, 36 GB memory".
    pub machine: String,
    pub writers: Vec<WriterInfo>,
    /// The chosen writing model's id.
    pub writer: String,
    /// The decision model's download size, e.g. "About 1.1 GB".
    pub decider_size: String,
    /// The embedding model's download size, e.g. "About 334 MB".
    pub embedder_size: String,
    /// What was picked; `None` until someone has been asked.
    pub plan: Option<Plan>,
    /// The choices, with their real sizes on this Mac.
    pub plans: Vec<PlanInfo>,
    pub features: Features,
    /// Bytes of models and runtime on disk, for Settings → Storage.
    pub on_disk: u64,
    /// The models with any of their file on disk, so each can be deleted: `embedder`,
    /// `decider` and writing models' ids, in that order.
    pub downloaded: Vec<String>,
    /// The embedding model's own file.
    pub embedder_download: ModelDownload,
    /// The decision model's own file.
    pub decider_download: ModelDownload,
    /// Why a running download is waiting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused: Option<Pause>,
    /// The writing model that started writing by itself when its download finished, until
    /// your next choice: Settings says so.
    pub took_over: Option<String>,
    pub download: Download,
}

/// What the AI runs on. The app passes the real ones; tests pass fakes.
pub struct Parts {
    /// `<app data>/ai`.
    pub dir: PathBuf,
    pub machine: Machine,
    pub catalog: Catalog,
    pub launcher: Arc<dyn Launch>,
    /// Gets the new status whenever it changes.
    pub events: Arc<dyn Fn(Status) + Send + Sync>,
    /// Free bytes on the disk that holds `dir`, or `None` if unknown.
    pub free_space: fn(&Path) -> Option<u64>,
    /// A `llama-server` to use instead of downloading one (debug builds only).
    pub runtime: Option<PathBuf>,
    /// Whether a download should wait now (see `power.rs`); tests pass fakes.
    pub conditions: fn() -> Option<Pause>,
    /// How often a download checks whether it should wait, or may go on.
    pub recheck: Duration,
    /// Apple Intelligence, when this Mac has it on.
    pub apple: Option<Apple>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Settings {
    writer: Option<String>,
    #[serde(default)]
    plan: Option<Plan>,
}

#[derive(Default)]
struct State {
    /// A download is running: it takes files from `queue` until none are left.
    running: bool,
    /// Files waiting their turn, in order.
    queue: Vec<Item>,
    /// The file being fetched, and how much of it is in.
    current: Option<(PathBuf, u64)>,
    failed: Option<String>,
    /// The file a download failed on, and why.
    failed_file: Option<(PathBuf, String)>,
    paused: Option<Pause>,
    /// The writing model you last asked to download, until you choose something else.
    asked_for: Option<String>,
    /// The writing model that started writing by itself when its download finished, until your next choice.
    took_over: Option<String>,
}

/// Whether a writing model whose download just finished should write from now on: when you
/// asked for it and haven't chosen anything else since, or when nothing on this Mac can write
/// (the model that did was deleted).
pub fn takes_over(finished: &str, asked_for: Option<&str>, chosen_ready: bool) -> bool {
    asked_for == Some(finished) || !chosen_ready
}

impl State {
    /// Whether `dest` is being fetched or waits its turn.
    fn wants(&self, dest: &Path) -> bool {
        self.current.as_ref().is_some_and(|(d, _)| d == dest) || self.queue.iter().any(|i| i.dest == dest)
    }
}

/// A model nobody has asked anything for this long is stopped, to give its memory back (the
/// writing model holds several GB). The next request starts it again.
pub const IDLE_UNLOAD: Duration = Duration::from_secs(10 * 60);
/// How often Moss looks for models left alone that long.
pub const IDLE_CHECK: Duration = Duration::from_secs(30);

/// How much one model is used: requests running now, and when the last one ended.
struct Use {
    busy: AtomicUsize,
    last: Mutex<tokio::time::Instant>,
}

impl Use {
    fn new() -> Self {
        Use { busy: AtomicUsize::new(0), last: Mutex::new(tokio::time::Instant::now()) }
    }

    /// Whether, at `now`, nothing is running on it and nothing has for [`IDLE_UNLOAD`].
    fn idle(&self, now: tokio::time::Instant) -> bool {
        self.busy.load(Ordering::SeqCst) == 0 && now.saturating_duration_since(*self.last.lock().unwrap()) >= IDLE_UNLOAD
    }
}

/// A request running on a model: while it lives, the model isn't stopped for being idle.
struct Busy<'a>(&'a Use);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        *self.0.last.lock().unwrap() = tokio::time::Instant::now();
        self.0.busy.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Inner {
    parts: Parts,
    /// For the models on this Mac.
    client: reqwest::Client,
    /// For downloads, built when the first one starts (see `http::internet`).
    downloads: tokio::sync::OnceCell<reqwest::Client>,
    writer: Mutex<String>,
    plan: Mutex<Option<Plan>>,
    state: Mutex<State>,
    /// Stops the file being fetched: someone cancelled, or the download must wait.
    cancel: AtomicBool,
    /// Someone cancelled: don't go on after a wait.
    stopped: AtomicBool,
    /// Only the file being fetched was stopped: go on with the next.
    skip: AtomicBool,
    embedder_server: tokio::sync::Mutex<Option<Box<dyn Running>>>,
    decider_server: tokio::sync::Mutex<Option<Box<dyn Running>>>,
    writer_server: tokio::sync::Mutex<Option<(String, Box<dyn Running>)>>,
    embedder_use: Use,
    decider_use: Use,
    writer_use: Use,
    /// Whether the task that stops idle models has started.
    reaper: AtomicBool,
    /// "The ask" already written, by writing model and message id.
    asks: Mutex<HashMap<(String, String), Option<String>>>,
}

/// The AI on this Mac. Cheap to clone.
#[derive(Clone)]
pub struct Ai(Arc<Inner>);

/// One file to get, and where it goes.
#[derive(Clone)]
struct Item {
    asset: Asset,
    dest: PathBuf,
    runtime: bool,
}

/// Room to spare beyond the files themselves, so the disk isn't left full.
const SPARE: u64 = 1 << 30;

impl Ai {
    pub fn new(parts: Parts) -> Ai {
        let settings: Settings = std::fs::read(parts.dir.join("settings.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let memory = parts.machine.memory_gb();
        // Apple Intelligence first, where there is one: it writes with nothing to download.
        let fallback = match parts.apple {
            Some(_) => apple::ID,
            None => catalog::recommended(&parts.catalog.writers, memory).id,
        }
        .to_string();
        // Someone who chose a writing model before plans existed wanted the lot.
        let plan = settings.plan.or(settings.writer.as_ref().map(|_| Plan::Recommended));
        let writer = settings
            .writer
            .filter(|id| (parts.apple.is_some() && id == apple::ID) || parts.catalog.writers.iter().any(|w| w.id == id))
            .unwrap_or(fallback);
        Ai(Arc::new(Inner {
            parts,
            client: crate::http::local(),
            downloads: tokio::sync::OnceCell::new(),
            writer: Mutex::new(writer),
            plan: Mutex::new(plan),
            state: Mutex::new(State::default()),
            cancel: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            skip: AtomicBool::new(false),
            embedder_server: tokio::sync::Mutex::new(None),
            decider_server: tokio::sync::Mutex::new(None),
            writer_server: tokio::sync::Mutex::new(None),
            embedder_use: Use::new(),
            decider_use: Use::new(),
            writer_use: Use::new(),
            reaper: AtomicBool::new(false),
            asks: Mutex::default(),
        }))
    }

    fn parts(&self) -> &Parts {
        &self.0.parts
    }

    /// The Gemma model that writes, or `None` when Apple Intelligence does.
    fn gguf(&self) -> Option<Writer> {
        let id = self.writer_key();
        self.parts().catalog.writers.iter().find(|w| w.id == id).copied()
    }

    /// Apple Intelligence, when it's the one that writes.
    fn apple(&self) -> Option<&Apple> {
        self.parts().apple.as_ref().filter(|_| self.writer_key() == apple::ID)
    }

    fn model_path(&self, asset: &Asset) -> PathBuf {
        self.parts().dir.join("models").join(asset.file)
    }

    fn runtime_marker(&self) -> PathBuf {
        self.parts()
            .dir
            .join("runtime")
            .join(format!("{}.ok", self.parts().catalog.runtime_dir))
    }

    /// The `llama-server` to run, once it's on disk and checked.
    fn runtime_binary(&self) -> Option<PathBuf> {
        if let Some(binary) = &self.parts().runtime {
            return Some(binary.clone()).filter(|b| b.is_file());
        }
        let catalog = &self.parts().catalog;
        let binary = self
            .parts()
            .dir
            .join("runtime")
            .join(catalog.runtime_dir)
            .join(catalog::SERVER_BINARY);
        let checked = std::fs::read_to_string(self.runtime_marker())
            .is_ok_and(|m| m.trim() == catalog.runtime.sha256);
        Some(binary).filter(|b| checked && b.is_file())
    }

    fn plan(&self) -> Option<Plan> {
        *self.0.plan.lock().unwrap()
    }

    /// The plan a download gets: the one picked, or everything when nothing (or nothing
    /// for now) was picked.
    fn wanted(&self) -> Plan {
        match self.plan() {
            Some(Plan::Essentials) => Plan::Essentials,
            _ => Plan::Recommended,
        }
    }

    /// Every file a plan needs, smallest first so each feature starts as early as it can:
    /// the runtime, the embedding model, the decision model, then the writing model.
    fn items_for(&self, plan: Plan) -> Vec<Item> {
        let catalog = &self.parts().catalog;
        let mut items = vec![];
        if plan == Plan::Skip {
            return items;
        }
        if self.parts().runtime.is_none() {
            let dest = self.parts().dir.join("runtime").join(catalog.runtime.file);
            items.push(Item {
                asset: catalog.runtime,
                dest,
                runtime: true,
            });
        }
        let mut models = vec![catalog.embedder, catalog.decider];
        if plan == Plan::Recommended {
            models.extend(self.gguf().map(|w| w.asset));
        }
        for asset in models {
            items.push(Item {
                asset,
                dest: self.model_path(&asset),
                runtime: false,
            });
        }
        items
    }

    fn items(&self) -> Vec<Item> {
        self.items_for(self.wanted())
    }

    fn item_ready(&self, item: &Item) -> bool {
        if item.runtime {
            self.runtime_binary().is_some()
        } else {
            download::is_ready(&item.dest, item.asset.sha256)
        }
    }

    fn on_disk(&self, item: &Item) -> u64 {
        if self.item_ready(item) {
            item.asset.bytes
        } else {
            download::on_disk(&item.dest, item.asset.sha256, item.asset.bytes)
        }
    }

    /// Bytes of an item on disk, or fetched so far while it downloads.
    fn progress(&self, state: &State, item: &Item) -> u64 {
        match &state.current {
            Some((dest, done)) if *dest == item.dest => *done,
            _ => self.on_disk(item),
        }
    }

    /// Where the download stands, worked out from the disk.
    fn download_state(&self) -> Download {
        let items = self.items();
        let total = items.iter().map(|i| i.asset.bytes).sum();
        let state = self.0.state.lock().unwrap();
        if state.running && items.iter().any(|i| state.wants(&i.dest)) {
            let done = items.iter().map(|i| self.progress(&state, i)).sum();
            return Download::Downloading { done, total };
        }
        if items.iter().all(|i| self.item_ready(i)) {
            return Download::Ready;
        }
        let done = items.iter().map(|i| self.on_disk(i)).sum();
        match &state.failed {
            Some(message) => Download::Failed {
                done,
                total,
                message: message.clone(),
            },
            None => Download::Missing { done, total },
        }
    }

    /// Where one model's own file stands.
    fn model_download(&self, asset: Asset) -> ModelDownload {
        let dest = self.model_path(&asset);
        if download::is_ready(&dest, asset.sha256) {
            return ModelDownload::Ready;
        }
        let item = Item { asset, dest, runtime: false };
        let state = self.0.state.lock().unwrap();
        let done = self.progress(&state, &item);
        match (&state.current, &state.failed_file) {
            (Some((d, _)), _) if *d == item.dest => ModelDownload::Downloading { done },
            _ if state.wants(&item.dest) => ModelDownload::Queued { done },
            (_, Some((d, message))) if *d == item.dest => ModelDownload::Failed { done, message: message.clone() },
            _ => ModelDownload::Missing { done },
        }
    }

    pub fn status(&self) -> Status {
        let memory = self.parts().machine.memory_gb();
        let plans = [Plan::Recommended, Plan::Essentials, Plan::Skip]
            .into_iter()
            .map(|plan| {
                let bytes = self.items_for(plan).iter().map(|i| i.asset.bytes).sum();
                PlanInfo { plan, bytes, size: catalog::size_label(bytes) }
            })
            .collect();
        let catalog = &self.parts().catalog;
        let on_disk: Vec<(&str, u64)> = self
            .models()
            .map(|(id, a)| (id, download::on_disk(&self.model_path(&a), a.sha256, a.bytes)))
            .collect();
        let models: u64 = on_disk.iter().map(|(_, bytes)| bytes).sum();
        let runtime = self.runtime_binary().map_or(0, |_| catalog.runtime.bytes);
        let (paused, took_over) = {
            let state = self.0.state.lock().unwrap();
            (state.paused, state.took_over.clone())
        };
        let apple = self.parts().apple.is_some();
        let gemma = catalog::writers_for(&catalog.writers, memory)
            .into_iter()
            .zip(&catalog.writers)
            .map(|(info, w)| WriterInfo { download: self.model_download(w.asset), recommended: info.recommended && !apple, ..info });
        Status {
            machine: self.parts().machine.label(),
            writers: apple.then(|| apple::info(true)).into_iter().chain(gemma).collect(),
            writer: self.writer_key(),
            decider_size: catalog::size_label(catalog.decider.bytes),
            embedder_size: catalog::size_label(catalog.embedder.bytes),
            plan: self.plan(),
            plans,
            features: Features {
                search: self.embedder_ready(),
                sorting: self.decider_ready(),
                writing: self.writer_ready(),
            },
            on_disk: models + runtime,
            downloaded: on_disk
                .iter()
                .filter(|(_, bytes)| *bytes > 0)
                .map(|(id, _)| id.to_string())
                .collect(),
            embedder_download: self.model_download(catalog.embedder),
            decider_download: self.model_download(catalog.decider),
            paused,
            took_over,
            download: self.download_state(),
        }
    }

    /// Names what writes now, to keep what it wrote by: the writing model's id.
    pub fn writer_key(&self) -> String {
        self.0.writer.lock().unwrap().clone()
    }

    /// Tokens the writing model reads and writes at once.
    pub fn context_tokens(&self) -> usize {
        self.apple().map_or(crate::agent::CONTEXT_TOKENS, |a| a.context)
    }

    fn save_settings(&self) -> Result<(), AiError> {
        let settings = Settings {
            writer: Some(self.writer_key()),
            plan: self.plan(),
        };
        let saved = std::fs::create_dir_all(&self.parts().dir).and_then(|_| {
            std::fs::write(
                self.parts().dir.join("settings.json"),
                serde_json::to_vec(&settings).unwrap(),
            )
        });
        saved.map_err(|e| AiError::Disk(e.to_string()))
    }

    /// Picks what to get, and starts getting it in the background (nothing, for
    /// [`Plan::Skip`]).
    pub fn choose_plan(&self, plan: Plan) -> Result<Status, AiError> {
        self.cancel();
        *self.0.plan.lock().unwrap() = Some(plan);
        self.0.state.lock().unwrap().failed = None;
        self.save_settings()?;
        if plan != Plan::Skip {
            self.download();
        }
        self.emit();
        Ok(self.status())
    }

    /// Deletes the models and the runtime, and turns the AI off until it's downloaded
    /// again. Settings stay.
    pub async fn remove_models(&self) -> Result<Status, AiError> {
        self.cancel();
        self.0.embedder_server.lock().await.take();
        self.0.decider_server.lock().await.take();
        self.0.writer_server.lock().await.take();
        for folder in ["models", "runtime"] {
            match std::fs::remove_dir_all(self.parts().dir.join(folder)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(AiError::Disk(e.to_string())),
                _ => {}
            }
        }
        *self.0.plan.lock().unwrap() = Some(Plan::Skip);
        self.save_settings()?;
        self.0.asks.lock().unwrap().clear();
        self.emit();
        Ok(self.status())
    }

    /// Every model, by the id [`Ai::remove_model`] takes.
    fn models(&self) -> impl Iterator<Item = (&'static str, Asset)> + '_ {
        let catalog = &self.parts().catalog;
        [("embedder", catalog.embedder), ("decider", catalog.decider)]
            .into_iter()
            .chain(catalog.writers.iter().map(|w| (w.id, w.asset)))
    }

    /// Deletes one model (`embedder`, `decider` or a writing model's id), with whatever part
    /// of it is in. Its job stops until it's downloaded again. A download getting it stops
    /// (what else is in is kept); one that isn't goes on.
    pub async fn remove_model(&self, id: &str) -> Result<Status, AiError> {
        let (_, asset) = self
            .models()
            .find(|(m, _)| *m == id)
            .ok_or_else(|| AiError::Choice(id.into()))?;
        let dest = self.model_path(&asset);
        self.leave_queue(&dest);
        match id {
            "embedder" => drop(self.0.embedder_server.lock().await.take()),
            "decider" => drop(self.0.decider_server.lock().await.take()),
            // Only the chosen writing model ever runs.
            _ if id == self.writer_key() => drop(self.0.writer_server.lock().await.take()),
            _ => {}
        }
        for file in [download::marker_path(&dest), dest.clone(), download::part_path(&dest)] {
            match std::fs::remove_file(file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(AiError::Disk(e.to_string())),
                _ => {}
            }
        }
        let mut state = self.0.state.lock().unwrap();
        if state.failed_file.as_ref().is_some_and(|(d, _)| *d == dest) {
            state.failed_file = None;
        }
        drop(state);
        self.emit();
        Ok(self.status())
    }

    fn emit(&self) {
        (self.parts().events)(self.status());
    }

    /// You chose who writes: a model you asked for no longer takes over, and the note that one did goes.
    fn chose(&self) {
        let mut state = self.0.state.lock().unwrap();
        state.asked_for = None;
        state.took_over = None;
    }

    /// After a writing model's file is in: it writes from now on if [`takes_over`] says so.
    async fn model_in(&self, item: &Item) {
        let memory = self.parts().machine.memory_gb();
        let Some(w) = self.parts().catalog.writers.iter().find(|w| self.model_path(&w.asset) == item.dest && catalog::fits(w, memory)) else {
            return;
        };
        let chosen_ready = self.gguf().is_none_or(|c| download::is_ready(&self.model_path(&c.asset), c.asset.sha256));
        let asked = self.0.state.lock().unwrap().asked_for.clone();
        if w.id == self.writer_key() || !takes_over(w.id, asked.as_deref(), chosen_ready) {
            return;
        }
        *self.0.writer.lock().unwrap() = w.id.to_string();
        let _ = self.save_settings();
        self.0.writer_server.lock().await.take();
        let mut state = self.0.state.lock().unwrap();
        state.asked_for = None;
        state.took_over = Some(w.id.to_string());
    }

    /// Picks the writing model; the old one stops running. A download in progress goes on.
    pub async fn choose_writer(&self, id: &str) -> Result<Status, AiError> {
        if id != apple::ID || self.parts().apple.is_none() {
            let writer = self
                .parts()
                .catalog
                .writers
                .iter()
                .find(|w| w.id == id)
                .ok_or_else(|| AiError::Choice(id.into()))?;
            if !catalog::fits(writer, self.parts().machine.memory_gb()) {
                return Err(AiError::Choice(id.into()));
            }
        }
        *self.0.writer.lock().unwrap() = id.to_string();
        self.0.state.lock().unwrap().failed = None;
        self.chose();
        self.save_settings()?;
        self.0.writer_server.lock().await.take();
        self.emit();
        Ok(self.status())
    }

    /// Starts getting whatever is missing, in the background. Returns the task, or `None` when
    /// a download is already running or everything is in. Nothing picked (or nothing for
    /// now) means everything: this is the Download button in Settings → AI.
    ///
    /// On battery below 20% or a phone's hotspot it waits, and goes on when that's over.
    pub fn download(&self) -> Option<tokio::task::JoinHandle<()>> {
        if self.items().iter().all(|i| self.item_ready(i) || self.0.state.lock().unwrap().wants(&i.dest)) {
            return None;
        }
        if self.plan() != Some(self.wanted()) {
            *self.0.plan.lock().unwrap() = Some(self.wanted());
            let _ = self.save_settings();
        }
        self.0.state.lock().unwrap().failed = None;
        self.enqueue(self.items())
    }

    /// Gets models on their own (`embedder`, `decider` or writing models' ids), with the
    /// runtime if it isn't in yet. Who writes stays as it is. A download already running
    /// takes them on; otherwise the new download's task comes back.
    pub fn download_models(&self, ids: &[String]) -> Result<Option<tokio::task::JoinHandle<()>>, AiError> {
        let mut items: Vec<Item> = self.items_for(Plan::Essentials).into_iter().filter(|i| i.runtime).collect();
        for id in ids {
            let (_, asset) = self.models().find(|(m, _)| m == id).ok_or_else(|| AiError::Choice(id.clone()))?;
            items.push(Item { asset, dest: self.model_path(&asset), runtime: false });
            // Downloading a writing model means: write with this one when it's in.
            if self.parts().catalog.writers.iter().any(|w| w.id == id) {
                self.0.state.lock().unwrap().asked_for = Some(id.clone());
            }
        }
        Ok(self.enqueue(items))
    }

    /// Stops getting one model, keeping what's in of it: off the queue if it waits its turn;
    /// the download moves on to the next file if it's the one being fetched.
    pub fn stop_model(&self, id: &str) -> Result<Status, AiError> {
        let (_, asset) = self.models().find(|(m, _)| *m == id).ok_or_else(|| AiError::Choice(id.into()))?;
        self.leave_queue(&self.model_path(&asset));
        self.emit();
        Ok(self.status())
    }

    /// Takes a file off the download: off the queue, or skipped if it's being fetched.
    fn leave_queue(&self, dest: &Path) {
        let mut state = self.0.state.lock().unwrap();
        state.queue.retain(|i| i.dest != dest);
        if state.current.as_ref().is_some_and(|(d, _)| d == dest) {
            self.0.skip.store(true, Ordering::Relaxed);
            self.0.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Adds the files not in yet to the queue, and starts a download if none is running.
    fn enqueue(&self, items: Vec<Item>) -> Option<tokio::task::JoinHandle<()>> {
        {
            let mut state = self.0.state.lock().unwrap();
            let new: Vec<Item> = items.into_iter().filter(|i| !self.item_ready(i) && !state.wants(&i.dest)).collect();
            if new.is_empty() {
                return None;
            }
            for item in &new {
                if state.failed_file.as_ref().is_some_and(|(d, _)| *d == item.dest) {
                    state.failed_file = None;
                }
            }
            state.queue.extend(new);
            if state.running {
                drop(state);
                self.emit();
                return None;
            }
            state.running = true;
            state.failed = None;
        }
        self.0.cancel.store(false, Ordering::Relaxed);
        self.0.stopped.store(false, Ordering::Relaxed);
        self.0.skip.store(false, Ordering::Relaxed);
        self.emit();
        let ai = self.clone();
        Some(tokio::spawn(async move {
            loop {
                let result = ai.fetch_when_allowed().await;
                let mut state = ai.0.state.lock().unwrap();
                match result {
                    // Someone asked for more as the last file finished.
                    Ok(()) if !state.queue.is_empty() => continue,
                    Ok(()) => {}
                    Err(DownloadError::Cancelled) => state.queue.clear(),
                    Err(e) => {
                        let text = message(&e);
                        state.failed_file = state.queue.first().map(|i| (i.dest.clone(), text.clone()));
                        state.failed = Some(text);
                        state.queue.clear();
                    }
                }
                state.running = false;
                state.paused = None;
                state.current = None;
                break;
            }
            ai.emit();
        }))
    }

    /// Fetches everything, waiting whenever the Mac is low on battery or on a hotspot.
    async fn fetch_when_allowed(&self) -> Result<(), DownloadError> {
        let conditions = self.parts().conditions;
        loop {
            if self.0.stopped.load(Ordering::Relaxed) {
                return Err(DownloadError::Cancelled);
            }
            if let Some(reason) = conditions() {
                self.pause(Some(reason));
                tokio::time::sleep(self.parts().recheck).await;
                continue;
            }
            self.pause(None);
            self.0.cancel.store(false, Ordering::Relaxed);
            // Stops the file being fetched as soon as the download should wait.
            let watch = {
                let ai = self.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(ai.parts().recheck).await;
                        if let Some(reason) = conditions() {
                            ai.0.state.lock().unwrap().paused = Some(reason);
                            ai.0.cancel.store(true, Ordering::Relaxed);
                            return;
                        }
                    }
                })
            };
            let result = self.fetch_all().await;
            watch.abort();
            let waiting = self.0.state.lock().unwrap().paused.is_some();
            match result {
                Err(DownloadError::Cancelled) if waiting && !self.0.stopped.load(Ordering::Relaxed) => {
                    self.emit();
                }
                other => return other,
            }
        }
    }

    fn pause(&self, reason: Option<Pause>) {
        let changed = std::mem::replace(&mut self.0.state.lock().unwrap().paused, reason) != reason;
        if changed {
            self.emit();
        }
    }

    /// Stops a running download. What's in is kept, so the next download resumes.
    pub fn cancel(&self) {
        self.0.stopped.store(true, Ordering::Relaxed);
        self.0.cancel.store(true, Ordering::Relaxed);
    }

    async fn fetch_all(&self) -> Result<(), DownloadError> {
        let missing: u64 = {
            let state = self.0.state.lock().unwrap();
            state.queue.iter().map(|i| i.asset.bytes - self.on_disk(i)).sum()
        };
        let free = (self.parts().free_space)(&self.parts().dir);
        if free.is_some_and(|free| free < missing + SPARE) {
            return Err(DownloadError::Disk(format!(
                "not enough free space. Moss needs {} more",
                catalog::size_label(missing + SPARE).trim_start_matches("About ")
            )));
        }
        loop {
            let item = {
                let mut state = self.0.state.lock().unwrap();
                state.current = None;
                if state.queue.is_empty() {
                    return Ok(());
                }
                let item = state.queue.remove(0);
                state.current = Some((item.dest.clone(), self.on_disk(&item)));
                item
            };
            // Only files not in yet are queued (see `enqueue`).
            std::fs::create_dir_all(item.dest.parent().expect("files live in a folder"))?;
            let mut last = Instant::now();
            let mut progress = |bytes: u64| {
                self.0.state.lock().unwrap().current = Some((item.dest.clone(), bytes));
                if last.elapsed() >= Duration::from_millis(200) || bytes == item.asset.bytes {
                    last = Instant::now();
                    self.emit();
                }
            };
            let request = Request {
                url: item.asset.url,
                dest: &item.dest,
                bytes: item.asset.bytes,
                sha256: item.asset.sha256,
            };
            match download::fetch(self.downloads().await, request, &self.0.cancel, &mut progress).await {
                // Only this file was stopped: on to the next.
                Err(DownloadError::Cancelled) if self.0.skip.swap(false, Ordering::Relaxed) => {
                    self.0.cancel.store(false, Ordering::Relaxed);
                    self.0.state.lock().unwrap().current = None;
                    self.emit();
                    continue;
                }
                Err(e) => {
                    // Back to the front, to pick up after a wait or to say it failed.
                    let mut state = self.0.state.lock().unwrap();
                    state.current = None;
                    state.queue.insert(0, item);
                    return Err(e);
                }
                Ok(()) => {}
            }
            if item.runtime {
                if let Err(e) = self.install_runtime(&item.dest) {
                    self.0.state.lock().unwrap().queue.insert(0, item);
                    return Err(e);
                }
            } else {
                self.model_in(&item).await;
            }
            // Its feature can start now.
            self.emit();
        }
    }

    /// The client downloads use, built on a blocking thread the first time one runs.
    async fn downloads(&self) -> &reqwest::Client {
        self.0
            .downloads
            .get_or_init(|| async { tokio::task::spawn_blocking(crate::http::internet).await.expect("building the HTTP client never panics") })
            .await
    }

    /// Marks a request on the model `role` as running, and makes sure idle models are
    /// looked for.
    fn busy(&self, role: Role) -> Busy<'_> {
        if !self.0.reaper.swap(true, Ordering::SeqCst) {
            tokio::spawn(reap(Arc::downgrade(&self.0)));
        }
        let using = self.uses(role);
        using.busy.fetch_add(1, Ordering::SeqCst);
        Busy(using)
    }

    fn uses(&self, role: Role) -> &Use {
        match role {
            Role::Embedder => &self.0.embedder_use,
            Role::Decider => &self.0.decider_use,
            Role::Writer => &self.0.writer_use,
        }
    }

    /// Stops every model that, at `now`, has been left alone for [`IDLE_UNLOAD`]. Each is
    /// checked holding its slot, which a request takes only after marking itself busy, so a
    /// model is never stopped while it answers.
    pub(crate) async fn unload_idle(&self, now: tokio::time::Instant) {
        let mut embedder = self.0.embedder_server.lock().await;
        if self.0.embedder_use.idle(now) {
            embedder.take();
        }
        drop(embedder);
        let mut decider = self.0.decider_server.lock().await;
        if self.0.decider_use.idle(now) {
            decider.take();
        }
        drop(decider);
        let mut writer = self.0.writer_server.lock().await;
        if self.0.writer_use.idle(now) {
            writer.take();
        }
    }

    /// Unpacks the checked runtime archive, marks it checked, and removes the archive.
    fn install_runtime(&self, archive: &Path) -> Result<(), DownloadError> {
        let runtime = self.parts().dir.join("runtime");
        crate::unpack::unpack(archive, &runtime)
            .map_err(|e| DownloadError::Disk(format!("couldn’t unpack the AI runtime: {e}")))?;
        let binary = runtime
            .join(self.parts().catalog.runtime_dir)
            .join(catalog::SERVER_BINARY);
        if !binary.is_file() {
            return Err(DownloadError::Damaged);
        }
        std::fs::write(self.runtime_marker(), self.parts().catalog.runtime.sha256)?;
        std::fs::remove_file(archive)?;
        let mut marker = archive.as_os_str().to_owned();
        marker.push(".ok");
        std::fs::remove_file(marker)?;
        Ok(())
    }

    /// Whether the embedding model can run now.
    pub fn embedder_ready(&self) -> bool {
        self.runtime_binary().is_some()
            && download::is_ready(
                &self.model_path(&self.parts().catalog.embedder),
                self.parts().catalog.embedder.sha256,
            )
    }

    /// Whether the decision model can run now.
    pub fn decider_ready(&self) -> bool {
        self.runtime_binary().is_some()
            && download::is_ready(
                &self.model_path(&self.parts().catalog.decider),
                self.parts().catalog.decider.sha256,
            )
    }

    /// Whether the writing model can run now.
    pub fn writer_ready(&self) -> bool {
        match self.gguf() {
            None => self.apple().is_some(),
            Some(w) => self.runtime_binary().is_some() && download::is_ready(&self.model_path(&w.asset), w.asset.sha256),
        }
    }

    /// Stops every model that is running. Moss calls this as it quits: the app exits without
    /// dropping anything, which would leave the models running and holding memory.
    pub async fn stop_all(&self) {
        self.0.embedder_server.lock().await.take();
        self.0.decider_server.lock().await.take();
        self.0.writer_server.lock().await.take();
    }

    /// Stops the models a run that crashed or was force-quit left running. Returns how many.
    pub fn stop_leftovers(&self) -> usize {
        let runtime = self.runtime_binary();
        let binaries: Vec<&Path> = runtime.as_deref().into_iter().chain(self.parts().apple.as_ref().map(|a| a.binary.as_path())).collect();
        crate::server::stop_leftovers(&self.parts().dir.join("run"), &binaries)
    }

    async fn launch(&self, model: &Path, role: Role) -> Result<Box<dyn Running>, AiError> {
        let binary = self.runtime_binary().ok_or(AiError::NotReady)?;
        self.launch_with(&binary, model, role).await
    }

    /// Starts `binary`: llama-server, or Apple's helper, which takes the same flags.
    async fn launch_with(&self, binary: &Path, model: &Path, role: Role) -> Result<Box<dyn Running>, AiError> {
        let run = self.parts().dir.join("run");
        std::fs::create_dir_all(&run).map_err(|e| AiError::Disk(e.to_string()))?;
        let key_file = run.join(format!("{}.key", role.name()));
        Ok(self
            .parts()
            .launcher
            .launch(binary, model, role, &key_file)
            .await?)
    }

    async fn embedder(&self) -> Result<Endpoint, AiError> {
        if !self.embedder_ready() {
            return Err(AiError::NotReady);
        }
        let mut slot = self.0.embedder_server.lock().await;
        if slot.is_none() {
            let model = self.model_path(&self.parts().catalog.embedder);
            *slot = Some(self.launch(&model, Role::Embedder).await?);
        }
        Ok(slot.as_ref().expect("just started").endpoint())
    }

    /// The embedding model's name, saved with each vector it makes.
    pub fn embedder_model(&self) -> &'static str {
        self.parts().catalog.embedder.id
    }

    /// One vector per text from the embedding model. Texts are formatted for it already
    /// (see `prompt::document` and `prompt::query`). A failed run is stopped, so the next
    /// call starts it fresh.
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, AiError> {
        let _busy = self.busy(Role::Embedder);
        let endpoint = self.embedder().await?;
        let vectors = crate::embed::vectors(&self.0.client, &endpoint, texts).await;
        if vectors.is_err() {
            self.0.embedder_server.lock().await.take();
        }
        vectors
    }

    async fn decider(&self) -> Result<Endpoint, AiError> {
        if !self.decider_ready() {
            return Err(AiError::NotReady);
        }
        let mut slot = self.0.decider_server.lock().await;
        if slot.is_none() {
            *slot = Some(
                self.launch(
                    &self.model_path(&self.parts().catalog.decider),
                    Role::Decider,
                )
                .await?,
            );
        }
        Ok(slot.as_ref().expect("just started").endpoint())
    }

    pub(crate) async fn writer(&self) -> Result<Endpoint, AiError> {
        if !self.writer_ready() {
            return Err(AiError::NotReady);
        }
        let id = self.writer_key();
        let mut slot = self.0.writer_server.lock().await;
        if slot.as_ref().is_none_or(|(running, _)| *running != id) {
            slot.take();
            let server = match self.apple() {
                Some(apple) => self.launch_with(&apple.binary, Path::new(apple::ID), Role::Writer).await?,
                None => {
                    let w = self.gguf().expect("a writer that's ready is Apple's or a Gemma model");
                    self.launch(&self.model_path(&w.asset), Role::Writer).await?
                }
            };
            *slot = Some((id, server));
        }
        Ok(slot.as_ref().expect("just started").1.endpoint())
    }

    /// Asks the decision model about `state`. A failed run is stopped, so the next question
    /// starts it fresh.
    pub async fn decide(
        &self,
        state: &str,
        questions: &[(&str, Question)],
    ) -> Result<BTreeMap<String, Answer>, AiError> {
        let _busy = self.busy(Role::Decider);
        let endpoint = self.decider().await?;
        let answers = decide::ask(&self.0.client, &endpoint, state, questions).await;
        if answers.is_err() {
            self.0.decider_server.lock().await.take();
        }
        answers
    }

    /// How sure the decision model is that `email` needs your reply, from 0 to 1.
    pub async fn needs_reply(&self, email: &str) -> Result<f64, AiError> {
        let answers = self
            .decide(email, &[("reply", prompt::needs_reply())])
            .await?;
        Ok(answers["reply"].yes())
    }

    /// Which split tab `email` belongs in (a label from [`prompt::SPLITS`]), and how likely.
    pub async fn split(&self, email: &str) -> Result<(String, f64), AiError> {
        let answers = self.decide(email, &[("split", prompt::split())]).await?;
        Ok(answers["split"].best().unwrap_or_default())
    }

    /// Like [`Ai::split`], for an email that came through a mailing list: one question,
    /// [`prompt::list_split`], that tells a newsletter from a notice about your own work.
    pub async fn list_split(&self, email: &str) -> Result<(String, f64), AiError> {
        let answers = self.decide(email, &[("split", prompt::list_split())]).await?;
        let (label, likely) = answers["split"].best().unwrap_or_default();
        Ok((prompt::list_tab(&label).to_string(), likely))
    }

    /// [`Ai::list_split`] for mail that came through a mailing list (`listed`), else
    /// [`Ai::split`].
    pub async fn split_for(&self, email: &str, listed: bool) -> Result<(String, f64), AiError> {
        if listed { self.list_split(email).await } else { self.split(email).await }
    }

    /// How likely `email` is a sales pitch ([`prompt::pitch`]), and how likely its sender
    /// needs something of the user's ([`prompt::wants`]), each from 0 to 1, in one request.
    pub async fn pitch(&self, email: &str) -> Result<(f64, f64), AiError> {
        let answers = self.decide(email, &[("pitch", prompt::pitch()), ("wants", prompt::wants())]).await?;
        Ok((answers["pitch"].yes(), answers["wants"].yes()))
    }

    /// How likely `email` sets the user a deadline, from 0 to 1.
    pub async fn deadline(&self, email: &str) -> Result<f64, AiError> {
        let answers = self.decide(email, &[("deadline", prompt::deadline())]).await?;
        Ok(answers["deadline"].yes())
    }

    /// How likely the user's own `email` asks its recipients for an answer, from 0 to 1.
    pub async fn expects_reply(&self, email: &str) -> Result<f64, AiError> {
        let answers = self.decide(email, &[("expects", prompt::expects_reply())]).await?;
        Ok(answers["expects"].yes())
    }

    /// One or two lines on a conversation, from the writing model.
    pub async fn summary(&self, conversation: &str) -> Result<Option<String>, AiError> {
        Ok(prompt::clean_summary(
            &self.write(prompt::SUMMARY_SYSTEM, conversation, 80).await?,
        ))
    }

    /// The HTTP client the models are reached with.
    pub(crate) fn http(&self) -> &reqwest::Client {
        &self.0.client
    }

    /// Marks a request on the writing model as running (see [`Ai::unload_idle`]).
    pub(crate) fn writing(&self) -> impl Drop + '_ {
        self.busy(Role::Writer)
    }

    /// The writing model failed: start it again next time.
    pub(crate) async fn writer_failed(&self) {
        self.0.writer_server.lock().await.take();
    }

    /// Asks the writing model (see [`crate::chat::ChatModel`] for `Ai`). A failed run stops
    /// it, so the next request starts it fresh.
    pub async fn write(
        &self,
        system: &str,
        user: &str,
        max_tokens: u32,
    ) -> Result<String, AiError> {
        let request = crate::chat::ChatRequest::ask(system, user, max_tokens);
        let reply = crate::chat::ChatModel::chat(self, &request, &mut |_| true).await?;
        Ok(reply.text.trim().to_string())
    }

    /// One line on what `email` asks of you, or `None` when it asks nothing.
    pub async fn ask_line(&self, email: &str) -> Result<Option<String>, AiError> {
        Ok(prompt::clean_ask(
            &self.write(prompt::ASK_SYSTEM, email, 60).await?,
        ))
    }

    /// "The ask" for a thread: what its latest message from the other person wants of you.
    /// Written once per message and writing model, then remembered until Moss quits.
    pub async fn ask_for(&self, thread: &Thread) -> Result<Option<String>, AiError> {
        let them = &thread.summary.from.email;
        let Some(message) =
            thread.messages.iter().rev().find(|m| &m.from.email == them).or(thread.messages.last())
        else {
            return Ok(None);
        };
        let key = (self.writer_key(), message.id.clone());
        if let Some(ask) = self.0.asks.lock().unwrap().get(&key) {
            return Ok(ask.clone());
        }
        let ask = self.ask_line(&prompt::message(message)).await?;
        self.0.asks.lock().unwrap().insert(key, ask.clone());
        Ok(ask)
    }
}

/// Every [`IDLE_CHECK`], stops the models left alone, until the AI is gone.
async fn reap(inner: std::sync::Weak<Inner>) {
    let mut every = tokio::time::interval(IDLE_CHECK);
    every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        every.tick().await;
        let Some(inner) = inner.upgrade() else { return };
        Ai(inner).unload_idle(tokio::time::Instant::now()).await;
    }
}

/// What Settings says when a download fails.
fn message(e: &DownloadError) -> String {
    match e {
        DownloadError::Network(_) => {
            "Can’t reach the download server. Check your connection and try again.".into()
        }
        DownloadError::Http(code) => {
            format!("The download server answered {code}. Try again later.")
        }
        DownloadError::Incomplete => {
            "The download stopped early. Try again to pick up where it stopped.".into()
        }
        DownloadError::Damaged => {
            "A downloaded file was damaged, so Moss deleted it. Try again.".into()
        }
        DownloadError::Disk(why) => {
            let mut why = why.clone();
            if let Some(first) = why.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            format!("{why}.")
        }
        DownloadError::Cancelled => "Stopped.".into(),
    }
}

/// Free bytes on the disk holding `path`: the disk with the longest mount point above it.
pub fn free_space(path: &Path) -> Option<u64> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .list()
        .iter()
        .filter(|d| path.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

#[cfg(test)]
mod tests;
