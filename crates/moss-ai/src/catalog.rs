//! The exact files Moss downloads, pinned to a commit and a SHA-256, and which writing model
//! fits which Mac. Sizes and hashes come from Hugging Face and GitHub (2 November 2026).

use serde::Serialize;

/// One file to download.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    pub id: &'static str,
    pub url: &'static str,
    /// The name it's saved under.
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

/// llama.cpp b11960 (0.8.0), the official Apple Silicon build. Unpacked after download.
pub const RUNTIME: Asset = Asset {
    id: "runtime",
    url: "https://github.com/ggml-org/llama.cpp/releases/download/b11960/llama-b11960-bin-macos-arm64.tar.gz",
    file: "llama-b11960-bin-macos-arm64.tar.gz",
    bytes: 16_465_074,
    sha256: "58e5a5e1d7c2bf66cedd9c29e0323f3e570660a70a1e2156d32eed08f4cc1e3c",
};

/// The folder the runtime unpacks into, and the server inside it.
pub const RUNTIME_DIR: &str = "llama-b11960";
pub const SERVER_BINARY: &str = "llama-server";

/// The decision model, vLLM-SR d3-lite: sorts and decides.
pub const DECIDER: Asset = Asset {
    id: "d3-lite",
    url: "https://huggingface.co/ggml-org/vllm-d3-lite-GGUF/resolve/d4d6ebde7717eb37014a1c7401127fb48f26da42/vllm-d3-lite-Q8_0.gguf",
    file: "vllm-d3-lite-Q8_0.gguf",
    bytes: 1_082_017_344,
    sha256: "ffed04a9607034d641b95181d79abf36bec3a03ed647949bad8277d0e2984ddf",
};

/// The embedding model, EmbeddingGemma 300M: what each email means, for search by meaning.
/// The smallest model, so it comes first.
pub const EMBEDDER: Asset = Asset {
    id: "embeddinggemma-300m",
    url: "https://huggingface.co/ggml-org/embeddinggemma-300M-GGUF/resolve/0f741b5a6585bd53aeb15cd1372c56f2a0f65e12/embeddinggemma-300M-Q8_0.gguf",
    file: "embeddinggemma-300M-Q8_0.gguf",
    bytes: 333_590_944,
    sha256: "b5ce9d77a3fc4b3b39ccb5643c36777911cc4eb46a66962eadfa3f5f60490d63",
};

/// A writing model you can pick in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Writer {
    pub id: &'static str,
    pub name: &'static str,
    pub sub: &'static str,
    pub asset: Asset,
    /// Offered only with at least this much memory (GB).
    pub needs_gb: u64,
    /// Recommended from this much memory (GB) up to the next model's.
    pub recommended_from_gb: u64,
}

pub const WRITERS: [Writer; 3] = [
    Writer {
        id: "e2b",
        name: "Gemma 4 E2B",
        sub: "Fastest. Good for summaries and quick replies.",
        asset: Asset {
            id: "e2b",
            url: "https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF/resolve/0314792d7f1f7e229411f620751375812bb9faf2/gemma-4-E2B-it-Q4_K_M.gguf",
            file: "gemma-4-E2B-it-Q4_K_M.gguf",
            bytes: 3_106_738_272,
            sha256: "740185b21d22ceb83a11c3aa62ad5842ef32c70f6096d756bbee85a1e4ec34b8",
        },
        needs_gb: 0,
        recommended_from_gb: 0,
    },
    Writer {
        id: "e4b",
        name: "Gemma 4 E4B",
        sub: "Smarter replies, still quick. Needs 16 GB of memory or more.",
        asset: Asset {
            id: "e4b",
            url: "https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF/resolve/bfc15c382204943c3a8fff0c750b94ae2364d7a3/gemma-4-E4B-it-Q4_K_M.gguf",
            file: "gemma-4-E4B-it-Q4_K_M.gguf",
            bytes: 4_977_171_584,
            sha256: "85a896a047553e842f25297ee5b031d64ff30147d9c4af17b1e4b394cd1fab87",
        },
        needs_gb: 16,
        recommended_from_gb: 16,
    },
    Writer {
        id: "26b",
        name: "Gemma 4 26B",
        sub: "Smartest. Needs 32 GB of memory or more.",
        asset: Asset {
            id: "26b",
            url: "https://huggingface.co/unsloth/gemma-4-26B-A4B-it-GGUF/resolve/c099eb48e663fd284577b04978a94ffccb261841/gemma-4-26B-A4B-it-UD-Q4_K_M.gguf",
            file: "gemma-4-26B-A4B-it-UD-Q4_K_M.gguf",
            bytes: 16_947_541_728,
            sha256: "f2c28b3dc4776931ac6f879e11f203dec637ea0f14267a86ec8f6165f63f293f",
        },
        needs_gb: 32,
        recommended_from_gb: 48,
    },
];

/// The writing model with this id.
pub fn writer(id: &str) -> Option<&'static Writer> {
    WRITERS.iter().find(|w| w.id == id)
}

/// The biggest writing model this much memory runs comfortably, beside the decision model.
pub fn recommended(writers: &[Writer], memory_gb: u64) -> &Writer {
    writers
        .iter()
        .rev()
        .find(|w| memory_gb >= w.recommended_from_gb)
        .unwrap_or(&writers[0])
}

/// Whether this Mac has the memory for the model.
pub fn fits(writer: &Writer, memory_gb: u64) -> bool {
    memory_gb >= writer.needs_gb
}

/// "About 5 GB", "About 1.1 GB": sizes as Settings shows them.
pub fn size_label(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 10.0 {
        format!("About {} GB", gb.round())
    } else if gb >= 1.0 {
        format!("About {} GB", format!("{gb:.1}").trim_end_matches(".0"))
    } else {
        format!("About {} MB", (bytes as f64 / 1e6).round())
    }
}

/// A writing model as Settings lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterInfo {
    pub id: String,
    pub name: String,
    pub sub: String,
    pub size: String,
    pub bytes: u64,
    pub recommended: bool,
    pub too_big: bool,
    /// Where its own file stands: none of the files every model shares counts.
    pub download: ModelDownload,
}

/// Where one model's own file stands. `done` is the bytes of it on disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum ModelDownload {
    /// Not in; `done` above nothing means a part-finished download of it is kept.
    Missing { done: u64 },
    /// Waiting its turn behind another file.
    Queued { done: u64 },
    Downloading { done: u64 },
    Failed { done: u64, message: String },
    /// In and checked.
    Ready,
}

/// Every writing model, marked for this much memory.
pub fn writers_for(writers: &[Writer], memory_gb: u64) -> Vec<WriterInfo> {
    let best = recommended(writers, memory_gb).id;
    writers
        .iter()
        .map(|w| WriterInfo {
            id: w.id.into(),
            name: w.name.into(),
            sub: w.sub.into(),
            size: size_label(w.asset.bytes),
            bytes: w.asset.bytes,
            recommended: w.id == best,
            too_big: !fits(w, memory_gb),
            download: ModelDownload::Missing { done: 0 },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_file_is_pinned_to_a_commit_and_a_hash() {
        let all = [RUNTIME, EMBEDDER, DECIDER]
            .into_iter()
            .chain(WRITERS.iter().map(|w| w.asset));
        for a in all {
            assert_eq!(a.sha256.len(), 64, "{}", a.id);
            assert!(a.sha256.chars().all(|c| c.is_ascii_hexdigit()), "{}", a.id);
            assert!(a.url.starts_with("https://"), "{}", a.id);
            assert!(a.url.ends_with(a.file), "{}", a.id);
            assert!(
                !a.url.contains("/resolve/main/"),
                "{} must name a commit",
                a.id
            );
            assert!(a.bytes > 1_000_000, "{}", a.id);
        }
    }

    #[test]
    fn recommends_the_biggest_model_the_memory_runs_well() {
        assert_eq!(recommended(&WRITERS, 8).id, "e2b");
        assert_eq!(recommended(&WRITERS, 16).id, "e4b");
        assert_eq!(recommended(&WRITERS, 36).id, "e4b");
        assert_eq!(recommended(&WRITERS, 48).id, "26b");
        assert_eq!(recommended(&WRITERS, 128).id, "26b");
        assert_eq!(recommended(&WRITERS, 0).id, "e2b");
    }

    #[test]
    fn a_model_that_needs_more_memory_does_not_fit() {
        let [e2b, e4b, big] = &WRITERS;
        assert!(fits(e2b, 8));
        assert!(!fits(e4b, 8));
        assert!(fits(e4b, 16));
        assert!(!fits(big, 24));
        assert!(fits(big, 32));
    }

    #[test]
    fn finds_writers_by_id() {
        assert_eq!(writer("e4b").map(|w| w.name), Some("Gemma 4 E4B"));
        assert_eq!(writer("gpt"), None);
    }

    #[test]
    fn sizes_read_like_people_say_them() {
        assert_eq!(size_label(DECIDER.bytes), "About 1.1 GB");
        assert_eq!(size_label(EMBEDDER.bytes), "About 334 MB");
        // What the first-run step offers on a 16 GB Mac: the essentials, and with Gemma 4 E4B.
        let essentials = RUNTIME.bytes + EMBEDDER.bytes + DECIDER.bytes;
        assert_eq!(size_label(essentials), "About 1.4 GB");
        assert_eq!(size_label(essentials + WRITERS[1].asset.bytes), "About 6.4 GB");
        assert_eq!(size_label(3_106_738_272), "About 3.1 GB");
        assert_eq!(size_label(4_977_171_584), "About 5 GB");
        assert_eq!(size_label(16_947_541_728), "About 17 GB");
        assert_eq!(size_label(16_465_074), "About 16 MB");
    }

    #[test]
    fn lists_writers_for_this_mac() {
        let list = writers_for(&WRITERS, 16);
        assert_eq!(
            list.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(),
            ["e2b", "e4b", "26b"]
        );
        assert_eq!(
            list.iter()
                .filter(|w| w.recommended)
                .map(|w| w.id.as_str())
                .collect::<Vec<_>>(),
            ["e4b"]
        );
        assert_eq!(
            list.iter()
                .filter(|w| w.too_big)
                .map(|w| w.id.as_str())
                .collect::<Vec<_>>(),
            ["26b"]
        );
        assert_eq!(list[1].size, "About 5 GB");
        assert_eq!(list[1].bytes, WRITERS[1].asset.bytes);
        assert_eq!(list[0].name, "Gemma 4 E2B");
        assert!(list[0].sub.starts_with("Fastest"));
    }
}
