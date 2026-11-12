//! Dictionary catalog + HTTP download (ADR-0011).

use std::io::Write;
use std::path::Path;

use serde::Serialize;

use crate::{Error, Result};

#[derive(Debug, Clone, Serialize)]
pub struct CatalogEntry {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: &'static str,
    pub description: &'static str,
    pub url: &'static str,
    /// Approximate download size in MB (for the UI).
    pub size_mb: u32,
    /// Part of the one-click recommended setup.
    pub recommended: bool,
}

/// Curated, stable download links (GitHub "latest release" URLs where possible).
pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "jitendex",
        name: "Jitendex",
        kind: "Definitions (English)",
        description: "Modern, richly formatted JMdict-based dictionary with examples. Best general choice.",
        url: "https://github.com/stephenmk/stephenmk.github.io/releases/latest/download/jitendex-yomitan.zip",
        size_mb: 45,
        recommended: true,
    },
    CatalogEntry {
        id: "jpdb-freq",
        name: "JPDB frequency",
        kind: "Frequency",
        description: "Word frequency ranks from a large corpus of anime, dramas, novels and games.",
        url: "https://raw.githubusercontent.com/Kuuuube/yomitan-dictionaries/main/dictionaries/JPDB_v2.2_Frequency_Kana_2024-10-13.zip",
        size_mb: 6,
        recommended: true,
    },
    CatalogEntry {
        id: "kanjium-pitch",
        name: "Kanjium pitch accents",
        kind: "Pitch accent",
        description: "Pitch accent patterns for ~120k words.",
        url: "https://github.com/toasted-nutbread/yomichan-pitch-accent-dictionary/releases/latest/download/kanjium_pitch_accents.zip",
        size_mb: 1,
        recommended: true,
    },
    CatalogEntry {
        id: "jmdict-en",
        name: "JMdict (English, no names)",
        kind: "Definitions (English)",
        description: "The classic JMdict, plain glossaries. Lighter alternative to Jitendex.",
        url: "https://github.com/yomidevs/jmdict-yomitan/releases/latest/download/JMdict_english_without_proper_names.zip",
        size_mb: 15,
        recommended: false,
    },
    CatalogEntry {
        id: "jmdict-en-names",
        name: "JMdict (English, with names)",
        kind: "Definitions (English)",
        description: "JMdict including proper names (people, places).",
        url: "https://github.com/yomidevs/jmdict-yomitan/releases/latest/download/JMdict_english.zip",
        size_mb: 39,
        recommended: false,
    },
    CatalogEntry {
        id: "bccwj-freq",
        name: "BCCWJ frequency",
        kind: "Frequency",
        description: "Frequency from the Balanced Corpus of Contemporary Written Japanese.",
        url: "https://raw.githubusercontent.com/Kuuuube/yomitan-dictionaries/main/dictionaries/BCCWJ_SUW_LUW_combined.zip",
        size_mb: 19,
        recommended: false,
    },
];

/// Download `url` to `dest`, calling `progress(done, total)`.
pub async fn download(url: &str, dest: &Path, mut progress: impl FnMut(u64, u64)) -> Result<()> {
    let client = crate::http_client()
        .user_agent("Yomu/0.1")
        .connect_timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| Error::Dict(e.to_string()))?;
    let mut resp = client.get(url).send().await.map_err(|e| Error::Dict(format!("download failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(Error::Dict(format!("download failed: HTTP {}", resp.status())));
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file = std::io::BufWriter::new(std::fs::File::create(dest)?);
    let mut done = 0u64;
    let mut last = 0u64;
    while let Some(chunk) = resp.chunk().await.map_err(|e| Error::Dict(format!("download interrupted: {e}")))? {
        file.write_all(&chunk)?;
        done += chunk.len() as u64;
        if done - last > 256 * 1024 {
            last = done;
            progress(done, total);
        }
    }
    file.flush()?;
    progress(done, total.max(done));
    Ok(())
}
