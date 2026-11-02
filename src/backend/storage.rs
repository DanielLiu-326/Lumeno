//! Storage report: originals in a user's data root plus the thumbnail,
//! darktable-preview and video-transcode caches generated for them.
//!
//! Cache files are named by the key that produced them, so a scan reverses the
//! mapping: recompute keys for every source file and intersect with the files on
//! disk. Anything left over belongs to a deleted, renamed or edited source — it
//! is safe to delete and will be rebuilt on the next view.
//!
//! Ownership follows who the work was done *for*: thumbnails and transcodes live
//! under the album/video owner's directory, darktable previews under the
//! requesting user's key. A scan only ever walks the caller's own data root, so a
//! cache entry that cannot be matched here is still ours to delete.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use axum::Extension;
use axum::Json;
use axum::extract::Query;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::backend::album::api::collect_accessible_album_sources;
use crate::backend::album::helpers::{
    album_display_name, albums_root, is_safe_cache_key, is_safe_cache_owner,
    is_user_visible_album_dir, load_existing_album_index, read_album_meta, sanitize_album_name,
};
use crate::backend::album::media::{
    ThumbnailGenerator, cache_relative_path, darktable_preview_cache_key, find_xmp_sidecar,
    is_image_path, is_video_path, thumbnail_cache_key_parts, xmp_modified_secs,
};
use crate::backend::album::video_encode::{
    encode_file, encode_root, is_encode_codec, probe_file, source_cache_key,
};
use crate::backend::cache_areas::{self, CacheArea, CacheGc, CacheLayout};
use crate::backend::common::{ApiResult, AppError, AppState, Ctx};
use crate::backend::constants::{THUMBNAIL_CACHE_SIZES, THUMBNAIL_SIZE_MAX, THUMBNAIL_SIZE_MIN};
use crate::backend::i18n::{tr, trf};
use crate::backend::tasks::{Task, TaskBase, TaskStatus, format_subtask};

pub const REPORT_VERSION: u32 = 1;
const REPORT_STALE_SECS: u64 = 24 * 3600;
/// A failed scan must not be re-queued by every report poll.
const SCAN_RETRY_AFTER_SECS: u64 = 60;
const THUMB_EXTENSIONS: [&str; 3] = ["webp", "avif", "jpg"];
/// Hashing is the expensive half of a scan; keep the parallel batch small so a
/// large library never holds millions of key strings at once.
const MEDIA_CHUNK: usize = 256;

// ── Report types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageKindTotals {
    pub files: u64,
    pub bytes: u64,
    #[serde(default)]
    pub unattributed_files: u64,
    #[serde(default)]
    pub unattributed_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageTotals {
    pub files: u64,
    pub bytes: u64,
    pub albums: StorageKindTotals,
    pub resources: StorageKindTotals,
    pub outside: StorageKindTotals,
    pub temp: StorageKindTotals,
    pub thumbnails: StorageKindTotals,
    pub darktable_previews: StorageKindTotals,
    pub transcodes: StorageKindTotals,
    pub transcode_probes: StorageKindTotals,
    pub ai: StorageKindTotals,
    /// Rebuildable caches that have no per-photo attribution.
    pub cache_other: StorageKindTotals,
}

impl StorageTotals {
    fn recompute(&mut self) {
        self.files = self.albums.files
            + self.resources.files
            + self.outside.files
            + self.temp.files
            + self.thumbnails.files
            + self.darktable_previews.files
            + self.transcodes.files
            + self.transcode_probes.files
            + self.ai.files
            + self.cache_other.files;
        self.bytes = self.albums.bytes
            + self.resources.bytes
            + self.outside.bytes
            + self.temp.bytes
            + self.thumbnails.bytes
            + self.darktable_previews.bytes
            + self.transcodes.bytes
            + self.transcode_probes.bytes
            + self.ai.bytes
            + self.cache_other.bytes;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageFormatRow {
    pub ext: String,
    pub files: u64,
    pub bytes: u64,
    pub cache_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageAlbumRow {
    pub album: String,
    pub name: String,
    pub files: u64,
    pub bytes: u64,
    /// Thumbnails plus darktable previews generated from this album's photos.
    pub thumbnails_bytes: u64,
    pub transcodes_bytes: u64,
    pub thumbnails_files: u64,
    pub transcodes_files: u64,
    pub indexed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageOutsideRow {
    /// First directory under the data root. Empty for files at the root itself.
    pub top: String,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageMonthRow {
    pub month: String,
    pub files: u64,
    pub bytes: u64,
    pub thumbnails_bytes: u64,
    pub transcodes_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageVideoRow {
    pub codec: String,
    pub key: String,
    pub name: String,
    pub album: Option<String>,
    pub bytes: u64,
    pub duration_ms: u64,
    pub unattributed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageSizeRow {
    pub size: u32,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StorageReport {
    pub version: u32,
    pub generated_at: u64,
    pub totals: StorageTotals,
    pub formats: Vec<StorageFormatRow>,
    pub albums: Vec<StorageAlbumRow>,
    pub outside: Vec<StorageOutsideRow>,
    pub months: Vec<StorageMonthRow>,
    pub unindexed_albums: u64,
    pub shared_album_count: u64,
    pub videos: Vec<StorageVideoRow>,
    pub thumbnail_sizes: Vec<StorageSizeRow>,
    /// One row per registered cache area (`cache_areas.rs`), including areas
    /// that are not attributed per photo.
    #[serde(default)]
    pub cache_areas: Vec<StorageCacheAreaRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageCacheAreaRow {
    pub name: String,
    pub files: u64,
    pub bytes: u64,
    /// Whether the retention GC may delete from this area.
    pub gc: bool,
}

// ── Cache attribution ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheKind {
    Thumbnail,
    Darktable,
    Transcode,
    Probe,
}

#[derive(Debug, Clone)]
pub(crate) struct CacheEntry {
    pub kind: CacheKind,
    pub key: String,
    pub path: PathBuf,
    pub bytes: u64,
    /// Own album directory name, when the source file could be attributed.
    pub album: Option<String>,
    pub month: Option<String>,
    /// Thumbnail edge length, for grouping the smaller caches.
    pub size: Option<u32>,
    pub codec: Option<String>,
    /// Source video file name (basename only), shown in the transcode list.
    pub source_name: Option<String>,
    /// False when no current source file produces this key.
    pub attributed: bool,
}

#[derive(Debug, Clone)]
struct CacheFile {
    path: PathBuf,
    bytes: u64,
    codec: Option<String>,
}

#[derive(Default)]
struct CacheIndex {
    /// Key → cached files. A key can exist in more than one encoder format.
    thumbnails: HashMap<String, Vec<CacheFile>>,
    darktable: HashMap<String, Vec<CacheFile>>,
    transcodes: HashMap<String, Vec<CacheFile>>,
    probes: HashMap<String, Vec<CacheFile>>,
}

impl CacheIndex {
    async fn load(data_path: &Path, username: &str) -> Self {
        let mut index = CacheIndex::default();
        let thumbs_dir = cache_area("thumbnails").owner_dir(data_path, username);
        for (name, path, bytes) in list_files(&thumbs_dir).await {
            let Some((key, ext)) = name.rsplit_once('.') else {
                continue;
            };
            if !THUMB_EXTENSIONS.contains(&ext) {
                continue;
            }
            index
                .thumbnails
                .entry(key.to_string())
                .or_default()
                .push(CacheFile {
                    path,
                    bytes,
                    codec: None,
                });
        }
        let darktable_dir = cache_area("darktable").owner_dir(data_path, username);
        for (name, path, bytes) in list_files(&darktable_dir).await {
            let Some((key, ext)) = name.rsplit_once('.') else {
                continue;
            };
            if ext != "jpg" {
                continue;
            }
            index
                .darktable
                .entry(key.to_string())
                .or_default()
                .push(CacheFile {
                    path,
                    bytes,
                    codec: None,
                });
        }
        if let Ok(root) = encode_root(data_path, username) {
            if let Ok(mut dir) = tokio::fs::read_dir(&root).await {
                while let Ok(Some(entry)) = dir.next_entry().await {
                    let is_dir = entry
                        .file_type()
                        .await
                        .map(|kind| kind.is_dir())
                        .unwrap_or(false);
                    if !is_dir {
                        continue;
                    }
                    let codec = entry.file_name().to_string_lossy().to_string();
                    if !is_encode_codec(&codec) {
                        continue;
                    }
                    for (name, path, bytes) in list_files(&entry.path()).await {
                        let Some((key, ext)) = name.rsplit_once('.') else {
                            continue;
                        };
                        if ext != "mp4" {
                            continue;
                        }
                        index
                            .transcodes
                            .entry(key.to_string())
                            .or_default()
                            .push(CacheFile {
                                path,
                                bytes,
                                codec: Some(codec.clone()),
                            });
                    }
                }
            }
            for (name, path, bytes) in list_files(&root.join("probe")).await {
                let Some((key, ext)) = name.rsplit_once('.') else {
                    continue;
                };
                if ext != "json" {
                    continue;
                }
                index
                    .probes
                    .entry(key.to_string())
                    .or_default()
                    .push(CacheFile {
                        path,
                        bytes,
                        codec: None,
                    });
            }
        }
        index
    }
}

async fn list_files(dir: &Path) -> Vec<(String, PathBuf, u64)> {
    let mut out = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return out;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(file_type) = entry.file_type().await else {
            continue;
        };
        if file_type.is_symlink() || !file_type.is_file() {
            continue;
        }
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        out.push((
            entry.file_name().to_string_lossy().to_string(),
            entry.path(),
            meta.len(),
        ));
    }
    out
}

// ── Scan ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) struct ScanInputs {
    pub data_path: PathBuf,
    pub user_root: PathBuf,
    pub username: String,
    pub shared_album_count: u64,
}

pub(crate) struct ScanProgress<'a> {
    pub base: &'a TaskBase,
    pub cancel: &'a AtomicBool,
}

impl ScanProgress<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    async fn advance(&self, phase: &str, file_name: &str) {
        let done = self.base.processed_count.load(Ordering::Relaxed);
        let total = self.base.total_count.load(Ordering::Relaxed).max(1);
        self.base.add_processed_count(1);
        self.base
            .set_cur_task(format_subtask(phase, file_name, done + 1, total))
            .await;
    }
}

pub(crate) enum ScanOutcome {
    Done(Box<StorageScan>),
    Cancelled,
}

pub(crate) struct StorageScan {
    pub report: StorageReport,
    pub cache: Vec<CacheEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Albums,
    Resources,
    Outside,
    Temp,
    Skip,
}

struct SourceFile {
    path: PathBuf,
    rel: String,
    name: String,
    len: u64,
    modified: u64,
    ext: String,
    category: Category,
    group: Option<String>,
    month: Option<String>,
    media: bool,
    video: bool,
    xmp_modified: Option<u64>,
}

struct MediaHit {
    thumbnails: Vec<(u32, String)>,
    darktable: Vec<(u32, String)>,
    video: Option<String>,
}

#[derive(Default)]
struct AlbumAgg {
    files: u64,
    bytes: u64,
    thumbnails_bytes: u64,
    transcodes_bytes: u64,
    thumbnails_files: u64,
    transcodes_files: u64,
}

#[derive(Default)]
struct MonthAgg {
    files: u64,
    bytes: u64,
    thumbnails_bytes: u64,
    transcodes_bytes: u64,
}

#[derive(Default)]
struct FormatAgg {
    files: u64,
    bytes: u64,
    cache_bytes: u64,
}

#[derive(Default)]
struct OutsideAgg {
    files: u64,
    bytes: u64,
}

#[derive(Default)]
struct Agg {
    albums: BTreeMap<String, AlbumAgg>,
    months: BTreeMap<String, MonthAgg>,
    formats: BTreeMap<String, FormatAgg>,
    outside: BTreeMap<String, OutsideAgg>,
    sizes: BTreeMap<u32, (u64, u64)>,
    totals: StorageTotals,
}

impl Agg {
    fn add_original(&mut self, src: &SourceFile) {
        self.totals.files += 1;
        self.totals.bytes += src.len;
        match src.category {
            Category::Albums => {
                self.totals.albums.files += 1;
                self.totals.albums.bytes += src.len;
                if let Some(album) = &src.group {
                    let row = self.albums.entry(album.clone()).or_default();
                    row.files += 1;
                    row.bytes += src.len;
                }
            }
            Category::Resources => {
                self.totals.resources.files += 1;
                self.totals.resources.bytes += src.len;
            }
            Category::Outside => {
                self.totals.outside.files += 1;
                self.totals.outside.bytes += src.len;
                let row = self
                    .outside
                    .entry(src.group.clone().unwrap_or_default())
                    .or_default();
                row.files += 1;
                row.bytes += src.len;
            }
            Category::Temp => {
                self.totals.temp.files += 1;
                self.totals.temp.bytes += src.len;
            }
            Category::Skip => {}
        }
        let format = self.formats.entry(src.ext.clone()).or_default();
        format.files += 1;
        format.bytes += src.len;
        if let (Category::Albums, Some(month)) = (src.category, src.month.as_ref()) {
            let row = self.months.entry(month.clone()).or_default();
            row.files += 1;
            row.bytes += src.len;
        }
    }

    fn add_cache(&mut self, kind: CacheKind, src: &SourceFile, bytes: u64, size: Option<u32>) {
        let totals = match kind {
            CacheKind::Thumbnail => &mut self.totals.thumbnails,
            CacheKind::Darktable => &mut self.totals.darktable_previews,
            CacheKind::Transcode => &mut self.totals.transcodes,
            CacheKind::Probe => &mut self.totals.transcode_probes,
        };
        totals.files += 1;
        totals.bytes += bytes;

        self.formats.entry(src.ext.clone()).or_default().cache_bytes += bytes;

        if matches!(kind, CacheKind::Thumbnail | CacheKind::Darktable)
            && let Some(size) = size
        {
            let row = self.sizes.entry(size).or_default();
            row.0 += 1;
            row.1 += bytes;
        }
        if src.category != Category::Albums {
            return;
        }
        if let Some(album) = &src.group {
            let row = self.albums.entry(album.clone()).or_default();
            match kind {
                CacheKind::Thumbnail | CacheKind::Darktable => {
                    row.thumbnails_bytes += bytes;
                    row.thumbnails_files += 1;
                }
                CacheKind::Transcode | CacheKind::Probe => {
                    row.transcodes_bytes += bytes;
                    row.transcodes_files += 1;
                }
            }
        }
        if let Some(month) = &src.month {
            let row = self.months.entry(month.clone()).or_default();
            match kind {
                CacheKind::Thumbnail | CacheKind::Darktable => row.thumbnails_bytes += bytes,
                CacheKind::Transcode | CacheKind::Probe => row.transcodes_bytes += bytes,
            }
        }
    }
}

static SCAN_FAILURES: std::sync::OnceLock<std::sync::Mutex<HashMap<String, u64>>> =
    std::sync::OnceLock::new();

fn scan_failures() -> &'static std::sync::Mutex<HashMap<String, u64>> {
    SCAN_FAILURES.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn record_scan_failure(owner: &str) {
    if let Ok(mut failures) = scan_failures().lock() {
        failures.insert(owner.to_string(), now_unix());
    }
}

fn clear_scan_failure(owner: &str) {
    if let Ok(mut failures) = scan_failures().lock() {
        failures.remove(owner);
    }
}

fn scan_recently_failed(owner: &str) -> bool {
    scan_failures()
        .lock()
        .ok()
        .and_then(|failures| failures.get(owner).copied())
        .is_some_and(|failed_at| now_unix().saturating_sub(failed_at) < SCAN_RETRY_AFTER_SECS)
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn modified_secs(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn ext_of(name: &str) -> String {
    let ext = name
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext.is_empty() {
        "(none)".to_string()
    } else if ext == "jpeg" {
        "jpg".to_string()
    } else {
        ext
    }
}

fn month_of(date: &str) -> Option<String> {
    if date.len() < 7 || date == "1970-01-01" {
        return None;
    }
    let bytes = date.as_bytes();
    let well_formed = bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit);
    if !well_formed {
        return None;
    }
    let month: u32 = date[5..7].parse().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    Some(date[..7].to_string())
}

fn categorize(rel: &str) -> (Category, Option<String>) {
    let mut parts = rel.split('/');
    match parts.next() {
        Some("albums") => match parts.next() {
            Some(segment) if segment.starts_with("__") => {
                if segment == "__resource__" {
                    (Category::Resources, None)
                } else {
                    (Category::Skip, None)
                }
            }
            Some(segment) => (Category::Albums, Some(segment.to_string())),
            // A file sitting directly in `albums/` is not part of any album.
            None => (Category::Skip, None),
        },
        Some("temp") => (Category::Temp, None),
        Some(first) => {
            let top = if rel.contains('/') {
                first.to_string()
            } else {
                String::new()
            };
            (Category::Outside, Some(top))
        }
        None => (Category::Outside, None),
    }
}

async fn collect_sources(
    inputs: &ScanInputs,
    months: &HashMap<String, String>,
    progress: Option<&ScanProgress<'_>>,
) -> Vec<SourceFile> {
    let mut sources = Vec::new();
    let mut stack = vec![inputs.user_root.clone()];
    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if progress.is_some_and(|progress| progress.cancelled()) {
                return sources;
            }
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            let path = entry.path();
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if entry.file_name().to_string_lossy() == "__uploading" {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if !file_type.is_file() {
                continue;
            }
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            let Ok(rel_path) = path.strip_prefix(&inputs.user_root) else {
                continue;
            };
            let rel = rel_path.to_string_lossy().replace('\\', "/");
            let (category, group) = categorize(&rel);
            if category == Category::Skip {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let media = is_image_path(&path) || is_video_path(&path);
            let video = is_video_path(&path);
            let xmp_modified = if media && !video {
                let xmp_path = find_xmp_sidecar(&path);
                xmp_modified_secs(xmp_path.as_deref()).await
            } else {
                None
            };
            sources.push(SourceFile {
                ext: ext_of(&name),
                path,
                rel: rel.clone(),
                name,
                len: meta.len(),
                modified: modified_secs(&meta),
                category,
                group,
                month: months.get(&rel).cloned(),
                media,
                video,
                xmp_modified,
            });
        }
    }
    sources
}

fn compute_hit(src: &SourceFile, data_path: &Path, username: &str) -> MediaHit {
    let thumbnails = if src.media {
        THUMBNAIL_CACHE_SIZES
            .iter()
            .map(|&size| {
                let key = thumbnail_cache_key_parts(
                    username,
                    data_path,
                    &src.path,
                    size,
                    src.len,
                    src.modified,
                    ThumbnailGenerator::Lumeno,
                );
                (size, key)
            })
            .collect()
    } else {
        Vec::new()
    };
    let darktable = if src.media && !src.video {
        THUMBNAIL_CACHE_SIZES
            .iter()
            .map(|&size| {
                let key = darktable_preview_cache_key(
                    data_path,
                    username,
                    &src.path,
                    size,
                    src.len,
                    src.modified,
                    src.xmp_modified,
                );
                (size, key)
            })
            .collect()
    } else {
        Vec::new()
    };
    let video = if src.video {
        let rel = cache_relative_path(data_path, &src.path);
        Some(source_cache_key(&rel, src.len, src.modified))
    } else {
        None
    };
    MediaHit {
        thumbnails,
        darktable,
        video,
    }
}

fn match_hit(
    src: &SourceFile,
    hit: MediaHit,
    index: &mut CacheIndex,
    agg: &mut Agg,
    entries: &mut Vec<CacheEntry>,
) {
    for (size, key) in hit.thumbnails {
        let Some(files) = index.thumbnails.remove(&key) else {
            continue;
        };
        for file in files {
            agg.add_cache(CacheKind::Thumbnail, src, file.bytes, Some(size));
            entries.push(CacheEntry {
                kind: CacheKind::Thumbnail,
                key: key.clone(),
                path: file.path,
                bytes: file.bytes,
                album: album_of(src),
                month: src.month.clone(),
                size: Some(size),
                codec: None,
                source_name: None,
                attributed: true,
            });
        }
    }
    for (size, key) in hit.darktable {
        let Some(files) = index.darktable.remove(&key) else {
            continue;
        };
        for file in files {
            agg.add_cache(CacheKind::Darktable, src, file.bytes, Some(size));
            entries.push(CacheEntry {
                kind: CacheKind::Darktable,
                key: key.clone(),
                path: file.path,
                bytes: file.bytes,
                album: album_of(src),
                month: src.month.clone(),
                size: Some(size),
                codec: None,
                source_name: None,
                attributed: true,
            });
        }
    }
    if let Some(key) = hit.video {
        if let Some(files) = index.transcodes.remove(&key) {
            for file in files {
                agg.add_cache(CacheKind::Transcode, src, file.bytes, None);
                entries.push(CacheEntry {
                    kind: CacheKind::Transcode,
                    key: key.clone(),
                    path: file.path,
                    bytes: file.bytes,
                    album: album_of(src),
                    month: src.month.clone(),
                    size: None,
                    codec: file.codec,
                    source_name: Some(src.name.clone()),
                    attributed: true,
                });
            }
        }
        if let Some(files) = index.probes.remove(&key) {
            for file in files {
                agg.add_cache(CacheKind::Probe, src, file.bytes, None);
                entries.push(CacheEntry {
                    kind: CacheKind::Probe,
                    key: key.clone(),
                    path: file.path,
                    bytes: file.bytes,
                    album: album_of(src),
                    month: src.month.clone(),
                    size: None,
                    codec: None,
                    source_name: Some(src.name.clone()),
                    attributed: true,
                });
            }
        }
    }
}

fn album_of(src: &SourceFile) -> Option<String> {
    if src.category == Category::Albums {
        src.group.clone()
    } else {
        None
    }
}

fn drain_unattributed(
    key: String,
    files: Vec<CacheFile>,
    kind: CacheKind,
    agg: &mut Agg,
    entries: &mut Vec<CacheEntry>,
) {
    let totals = match kind {
        CacheKind::Thumbnail => &mut agg.totals.thumbnails,
        CacheKind::Darktable => &mut agg.totals.darktable_previews,
        CacheKind::Transcode => &mut agg.totals.transcodes,
        CacheKind::Probe => &mut agg.totals.transcode_probes,
    };
    for file in files {
        totals.files += 1;
        totals.bytes += file.bytes;
        totals.unattributed_files += 1;
        totals.unattributed_bytes += file.bytes;
        entries.push(CacheEntry {
            kind,
            key: key.clone(),
            path: file.path,
            bytes: file.bytes,
            album: None,
            month: None,
            size: None,
            codec: file.codec,
            source_name: None,
            attributed: false,
        });
    }
}

/// `cache/ai/{owner}` is read-only in this version; count it so the totals do
/// not silently omit it.
async fn ai_cache_totals(data_path: &Path, username: &str) -> (u64, u64) {
    let root = cache_area("ai").owner_dir(data_path, username);
    let mut stack = vec![root];
    let (mut files, mut bytes) = (0u64, 0u64);
    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                stack.push(entry.path());
                continue;
            }
            if let Ok(meta) = entry.metadata().await {
                files += 1;
                bytes += meta.len();
            }
        }
    }
    (files, bytes)
}

pub(crate) async fn scan_storage(
    inputs: &ScanInputs,
    progress: Option<&ScanProgress<'_>>,
) -> Result<ScanOutcome, AppError> {
    if !is_safe_cache_owner(&inputs.username) {
        return Err(AppError::BadRequest("invalid cache owner".to_owned()));
    }

    // Album index: display names, whether an index exists, and the photo dates
    // the year/month view needs.
    let mut album_info: HashMap<String, (String, bool)> = HashMap::new();
    let mut months: HashMap<String, String> = HashMap::new();
    let mut unindexed_albums = 0u64;
    if let Ok(mut entries) = tokio::fs::read_dir(albums_root(&inputs.user_root)).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().to_string();
            if !is_user_visible_album_dir(&dir_name) {
                continue;
            }
            let meta = read_album_meta(&entry.path()).await;
            let display = album_display_name(&meta, &dir_name);
            let index = load_existing_album_index(
                &inputs.data_path,
                &inputs.user_root,
                &inputs.username,
                &dir_name,
            )
            .await?;
            let indexed = index.is_some();
            if !indexed {
                unindexed_albums += 1;
            }
            if let Some(index) = index {
                for entry in index.entries {
                    if let Some(month) = month_of(&entry.date) {
                        months.insert(entry.path, month);
                    }
                }
            }
            album_info.insert(dir_name, (display, indexed));
        }
    }

    let mut cache = CacheIndex::load(&inputs.data_path, &inputs.username).await;

    if let Some(progress) = progress
        && progress.cancelled()
    {
        return Ok(ScanOutcome::Cancelled);
    }

    let mut sources = collect_sources(inputs, &months, progress).await;
    if let Some(progress) = progress
        && progress.cancelled()
    {
        return Ok(ScanOutcome::Cancelled);
    }
    let media_count = sources.iter().filter(|source| source.media).count();
    if let Some(progress) = progress {
        progress.base.set_total_count(sources.len() + media_count);
    }

    let mut agg = Agg::default();
    // Keep media indices; aggregation only needs the source facts.
    let mut media: Vec<usize> = Vec::with_capacity(media_count);
    for (position, source) in sources.iter().enumerate() {
        agg.add_original(source);
        if source.media {
            media.push(position);
        }
        if let Some(progress) = progress {
            progress.advance("storage", &source.name).await;
            if progress.cancelled() {
                return Ok(ScanOutcome::Cancelled);
            }
        }
    }

    let mut entries: Vec<CacheEntry> = Vec::new();
    for chunk in media.chunks(MEDIA_CHUNK) {
        if let Some(progress) = progress
            && progress.cancelled()
        {
            return Ok(ScanOutcome::Cancelled);
        }
        let hits: Vec<MediaHit> = chunk
            .par_iter()
            .map(|&position| compute_hit(&sources[position], &inputs.data_path, &inputs.username))
            .collect();
        for (&position, hit) in chunk.iter().zip(hits) {
            let source = &sources[position];
            match_hit(source, hit, &mut cache, &mut agg, &mut entries);
            if let Some(progress) = progress {
                progress.advance("storage", &source.name).await;
            }
        }
    }

    for (key, files) in std::mem::take(&mut cache.thumbnails) {
        drain_unattributed(key, files, CacheKind::Thumbnail, &mut agg, &mut entries);
    }
    // Previews live under the requesting account's directory, so a file that no
    // longer matches one of that account's sources belongs to a deleted or
    // edited photo and is ours to clean.
    for (key, files) in std::mem::take(&mut cache.darktable) {
        drain_unattributed(key, files, CacheKind::Darktable, &mut agg, &mut entries);
    }
    for (key, files) in std::mem::take(&mut cache.transcodes) {
        drain_unattributed(key, files, CacheKind::Transcode, &mut agg, &mut entries);
    }
    for (key, files) in std::mem::take(&mut cache.probes) {
        drain_unattributed(key, files, CacheKind::Probe, &mut agg, &mut entries);
    }

    let (ai_files, ai_bytes) = ai_cache_totals(&inputs.data_path, &inputs.username).await;
    agg.totals.ai.files = ai_files;
    agg.totals.ai.bytes = ai_bytes;
    agg.totals.recompute();

    sources.clear();
    let report = build_report(inputs, agg, album_info, unindexed_albums, &entries).await;
    Ok(ScanOutcome::Done(Box::new(StorageScan {
        report,
        cache: entries,
    })))
}

async fn build_report(
    inputs: &ScanInputs,
    mut agg: Agg,
    album_info: HashMap<String, (String, bool)>,
    unindexed_albums: u64,
    entries: &[CacheEntry],
) -> StorageReport {
    let mut albums: Vec<StorageAlbumRow> = album_info
        .iter()
        .map(|(dir_name, (name, indexed))| {
            let row = agg.albums.remove(dir_name).unwrap_or_default();
            StorageAlbumRow {
                album: dir_name.clone(),
                name: name.clone(),
                files: row.files,
                bytes: row.bytes,
                thumbnails_bytes: row.thumbnails_bytes,
                transcodes_bytes: row.transcodes_bytes,
                thumbnails_files: row.thumbnails_files,
                transcodes_files: row.transcodes_files,
                indexed: *indexed,
            }
        })
        .collect();
    // Albums whose directory vanished mid-scan still had originals counted.
    for (dir_name, row) in std::mem::take(&mut agg.albums) {
        albums.push(StorageAlbumRow {
            name: dir_name.clone(),
            album: dir_name,
            files: row.files,
            bytes: row.bytes,
            thumbnails_bytes: row.thumbnails_bytes,
            transcodes_bytes: row.transcodes_bytes,
            thumbnails_files: row.thumbnails_files,
            transcodes_files: row.transcodes_files,
            indexed: false,
        });
    }
    albums.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.album.cmp(&b.album)));

    let mut formats: Vec<StorageFormatRow> = agg
        .formats
        .into_iter()
        .map(|(ext, row)| StorageFormatRow {
            ext,
            files: row.files,
            bytes: row.bytes,
            cache_bytes: row.cache_bytes,
        })
        .collect();
    formats.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.ext.cmp(&b.ext)));

    let mut outside: Vec<StorageOutsideRow> = agg
        .outside
        .into_iter()
        .map(|(top, row)| StorageOutsideRow {
            top,
            files: row.files,
            bytes: row.bytes,
        })
        .collect();
    outside.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.top.cmp(&b.top)));

    let mut months: Vec<StorageMonthRow> = agg
        .months
        .into_iter()
        .map(|(month, row)| StorageMonthRow {
            month,
            files: row.files,
            bytes: row.bytes,
            thumbnails_bytes: row.thumbnails_bytes,
            transcodes_bytes: row.transcodes_bytes,
        })
        .collect();
    months.sort_by(|a, b| b.month.cmp(&a.month));

    let mut thumbnail_sizes: Vec<StorageSizeRow> = std::mem::take(&mut agg.sizes)
        .into_iter()
        .map(|(size, (files, bytes))| StorageSizeRow { size, files, bytes })
        .collect();
    thumbnail_sizes.sort_by(|a, b| b.files.cmp(&a.files).then(a.size.cmp(&b.size)));

    let mut videos: Vec<StorageVideoRow> = Vec::new();
    for entry in entries
        .iter()
        .filter(|entry| entry.kind == CacheKind::Transcode)
    {
        let duration_ms =
            read_probe_duration(&inputs.data_path, &inputs.username, &entry.key).await;
        videos.push(StorageVideoRow {
            codec: entry.codec.clone().unwrap_or_else(|| "h264".to_string()),
            key: entry.key.clone(),
            name: entry
                .source_name
                .clone()
                .unwrap_or_else(|| entry.key.clone()),
            album: entry.album.clone(),
            bytes: entry.bytes,
            duration_ms,
            unattributed: !entry.attributed,
        });
    }
    videos.sort_by(|a, b| b.bytes.cmp(&a.bytes));

    let cache_areas = scan_cache_areas(&inputs.data_path).await;
    let mut totals = agg.totals;
    for row in &cache_areas {
        if UNATTRIBUTED_AREAS.contains(&row.name.as_str()) {
            totals.cache_other.files += row.files;
            totals.cache_other.bytes += row.bytes;
        }
    }
    totals.recompute();

    StorageReport {
        version: REPORT_VERSION,
        generated_at: now_unix(),
        totals,
        formats,
        albums,
        outside,
        months,
        unindexed_albums,
        shared_album_count: inputs.shared_album_count,
        videos,
        thumbnail_sizes,
        cache_areas,
    }
}

async fn read_probe_duration(data_path: &Path, username: &str, key: &str) -> u64 {
    let Ok(path) = probe_file(data_path, username, key) else {
        return 0;
    };
    let Ok(raw) = tokio::fs::read_to_string(&path).await else {
        return 0;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return 0;
    };
    value
        .get("source")
        .and_then(|source| source.get("duration_ms"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

// ── Report persistence ───────────────────────────────────────────────────────

fn report_path(data_path: &Path, owner: &str) -> Option<PathBuf> {
    if !is_safe_cache_owner(owner) {
        return None;
    }
    Some(
        data_path
            .join("cache")
            .join("storage-reports")
            .join(format!("{owner}.json")),
    )
}

pub(crate) async fn save_report(
    data_path: &Path,
    owner: &str,
    report: &StorageReport,
) -> Result<(), AppError> {
    let Some(path) = report_path(data_path, owner) else {
        return Err(AppError::BadRequest("invalid cache owner".to_owned()));
    };
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| AppError::Unknown(format!("create report dir failed: {error}")))?;
    }
    let bytes = serde_json::to_vec(report)
        .map_err(|error| AppError::Unknown(format!("serialize report failed: {error}")))?;
    tokio::fs::write(&path, bytes)
        .await
        .map_err(|error| AppError::Unknown(format!("write report failed: {error}")))
}

pub(crate) async fn load_report(data_path: &Path, owner: &str) -> Option<StorageReport> {
    let path = report_path(data_path, owner)?;
    let raw = tokio::fs::read_to_string(&path).await.ok()?;
    let report: StorageReport = serde_json::from_str(&raw).ok()?;
    if report.version != REPORT_VERSION {
        return None;
    }
    Some(report)
}

/// A cleanup changes the numbers, so the next report read rescans instead of
/// showing stale totals.
pub(crate) async fn invalidate_report(data_path: &Path, owner: &str) {
    if let Some(path) = report_path(data_path, owner) {
        let _ = tokio::fs::remove_file(&path).await;
    }
}

fn report_stale(generated_at: u64) -> bool {
    now_unix().saturating_sub(generated_at) > REPORT_STALE_SECS
}

// ── Cleanup spec ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupKind {
    Thumbnails,
    Transcodes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CleanupTarget {
    Unattributed,
    Album,
    Month,
    Size,
    Codec,
    Key,
    All,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CleanupRequest {
    /// Account whose caches are deleted. Absent = the caller; naming someone
    /// else requires an admin.
    #[serde(default)]
    pub owner: Option<String>,
    pub kind: CleanupKind,
    pub target: CleanupTarget,
    #[serde(default)]
    pub album: Option<String>,
    #[serde(default)]
    pub month: Option<String>,
    #[serde(default)]
    pub size: Option<u32>,
    #[serde(default)]
    pub codec: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub dry_run: bool,
}

fn valid_month(month: &str) -> bool {
    month.len() == 7
        && month.as_bytes()[..4].iter().all(u8::is_ascii_digit)
        && month.as_bytes()[4] == b'-'
        && month.as_bytes()[5..7].iter().all(u8::is_ascii_digit)
        && matches!(month[5..7].parse::<u32>(), Ok(1..=12))
}

fn validate_cleanup(locale: &str, spec: &CleanupRequest) -> Result<(), AppError> {
    let invalid = |key: &str| Err(AppError::BadRequest(tr(locale, key)));
    match spec.kind {
        CleanupKind::Thumbnails => {
            if spec.target == CleanupTarget::Codec {
                return invalid("error.storage_invalid_target");
            }
            if matches!(spec.target, CleanupTarget::Size)
                && !matches!(spec.size, Some(0))
                && !matches!(
                    spec.size,
                    Some(size) if (THUMBNAIL_SIZE_MIN..=THUMBNAIL_SIZE_MAX).contains(&size)
                )
            {
                return invalid("error.storage_size_invalid");
            }
        }
        CleanupKind::Transcodes => {
            if matches!(spec.target, CleanupTarget::Size) {
                return invalid("error.storage_invalid_target");
            }
            if spec.target == CleanupTarget::Codec
                && !spec.codec.as_deref().is_some_and(is_encode_codec)
            {
                return invalid("error.storage_invalid_target");
            }
        }
    }
    if spec.target == CleanupTarget::Album {
        let album = spec.album.clone().unwrap_or_default();
        if sanitize_album_name(&album).is_none() {
            return Err(AppError::BadRequest(trf(
                locale,
                "error.storage_album_not_found",
                &[("value", album)],
            )));
        }
    }
    if spec.target == CleanupTarget::Month && !spec.month.as_deref().is_some_and(valid_month) {
        return invalid("error.storage_invalid_target");
    }
    if spec.target == CleanupTarget::Key && !spec.key.as_deref().is_some_and(is_safe_cache_key) {
        return invalid("error.storage_invalid_key");
    }
    Ok(())
}

fn kind_accepts(kind: CleanupKind, entry: CacheKind) -> bool {
    match kind {
        CleanupKind::Thumbnails => {
            matches!(entry, CacheKind::Thumbnail | CacheKind::Darktable)
        }
        CleanupKind::Transcodes => {
            matches!(entry, CacheKind::Transcode | CacheKind::Probe)
        }
    }
}

/// A registered area by name. The name is a compile-time constant, so a miss is
/// a programming error rather than a request failure.
fn cache_area(name: &str) -> &'static CacheArea {
    cache_areas::area(name).expect("cache area is registered")
}

/// Recursive size of one cache area, skipping symlinks so a link cannot point
/// the walk at the user's albums.
async fn cache_area_totals(root: &Path) -> (u64, u64) {
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(kind) = entry.file_type().await else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                files += 1;
                if let Ok(meta) = entry.metadata().await {
                    bytes += meta.len();
                }
            }
        }
    }
    (files, bytes)
}

async fn scan_cache_areas(data_path: &Path) -> Vec<StorageCacheAreaRow> {
    let mut rows = Vec::with_capacity(cache_areas::CACHE_AREAS.len());
    for area in cache_areas::CACHE_AREAS {
        let (files, bytes) = cache_area_totals(&area.root(data_path)).await;
        rows.push(StorageCacheAreaRow {
            name: area.name.to_string(),
            files,
            bytes,
            gc: area.gc == CacheGc::FilesOlderThan,
        });
    }
    rows
}

/// Areas whose bytes are not part of any attributed category.
const UNATTRIBUTED_AREAS: &[&str] = &[
    "exif",
    "exports",
    "index",
    "thumbnail-manifests",
    "storage-reports",
];

fn allowed_root(data_path: &Path, username: &str, entry: CacheKind) -> Option<PathBuf> {
    match entry {
        CacheKind::Thumbnail => Some(cache_area("thumbnails").owner_dir(data_path, username)),
        CacheKind::Darktable => Some(cache_area("darktable").owner_dir(data_path, username)),
        CacheKind::Transcode | CacheKind::Probe => encode_root(data_path, username).ok(),
    }
}

#[derive(Debug)]
struct CleanupTargetFile {
    kind: CacheKind,
    path: PathBuf,
    bytes: u64,
}

enum CleanupOutcome {
    Ready(Vec<CleanupTargetFile>),
    Cancelled,
}

async fn direct_targets(
    inputs: &ScanInputs,
    spec: &CleanupRequest,
) -> Result<Vec<CleanupTargetFile>, AppError> {
    let key = spec
        .key
        .clone()
        .ok_or_else(|| AppError::BadRequest("missing cache key".to_owned()))?;
    let mut targets = Vec::new();
    match spec.kind {
        CleanupKind::Thumbnails => {
            for ext in THUMB_EXTENSIONS {
                let path = cache_area("thumbnails")
                    .owner_dir(&inputs.data_path, &inputs.username)
                    .join(format!("{key}.{ext}"));
                if let Ok(meta) = tokio::fs::metadata(&path).await
                    && meta.is_file()
                {
                    targets.push(CleanupTargetFile {
                        kind: CacheKind::Thumbnail,
                        path,
                        bytes: meta.len(),
                    });
                }
            }
        }
        CleanupKind::Transcodes => {
            let codec = spec
                .codec
                .clone()
                .ok_or_else(|| AppError::BadRequest("missing codec".to_owned()))?;
            let file = encode_file(&inputs.data_path, &inputs.username, &codec, &key)?;
            let probe = probe_file(&inputs.data_path, &inputs.username, &key)?;
            for (kind, path) in [(CacheKind::Transcode, file), (CacheKind::Probe, probe)] {
                if let Ok(meta) = tokio::fs::metadata(&path).await
                    && meta.is_file()
                {
                    targets.push(CleanupTargetFile {
                        kind,
                        path,
                        bytes: meta.len(),
                    });
                }
            }
        }
    }
    Ok(targets)
}

fn entry_matches(spec: &CleanupRequest, entry: &CacheEntry) -> bool {
    match spec.target {
        CleanupTarget::All => true,
        CleanupTarget::Unattributed => !entry.attributed,
        CleanupTarget::Album => entry.album.as_deref() == spec.album.as_deref(),
        CleanupTarget::Month => entry.month.as_deref() == spec.month.as_deref(),
        CleanupTarget::Size => entry.size == spec.size,
        CleanupTarget::Codec => entry.codec.as_deref() == spec.codec.as_deref(),
        CleanupTarget::Key => entry.key == spec.key.clone().unwrap_or_default(),
    }
}

async fn collect_cleanup_targets(
    inputs: &ScanInputs,
    spec: &CleanupRequest,
    progress: Option<&ScanProgress<'_>>,
) -> Result<CleanupOutcome, AppError> {
    if spec.target == CleanupTarget::Key {
        return Ok(CleanupOutcome::Ready(direct_targets(inputs, spec).await?));
    }
    let scan = match scan_storage(inputs, progress).await? {
        ScanOutcome::Done(scan) => scan,
        ScanOutcome::Cancelled => return Ok(CleanupOutcome::Cancelled),
    };
    let mut targets = Vec::new();
    for entry in scan.cache {
        if !kind_accepts(spec.kind, entry.kind) || !entry_matches(spec, &entry) {
            continue;
        }
        targets.push(CleanupTargetFile {
            kind: entry.kind,
            path: entry.path,
            bytes: entry.bytes,
        });
    }
    Ok(CleanupOutcome::Ready(targets))
}

/// Delete one cache file, refusing anything outside the directory that kind is
/// allowed to touch. Returns whether the file was removed (never true on a dry run).
async fn remove_cache_file(
    path: &Path,
    allowed: &Path,
    dry_run: bool,
) -> Result<bool, std::io::Error> {
    if !path.starts_with(allowed) {
        return Ok(false);
    }
    if dry_run {
        return Ok(true);
    }
    match tokio::fs::remove_file(path).await {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

// ── Tasks ────────────────────────────────────────────────────────────────────

pub struct StorageScanTask {
    /// Belongs to whoever queued it, so an admin sees a member's scan progress.
    base: TaskBase,
    inputs: ScanInputs,
    /// The account whose root is scanned; also the deduplication key.
    scan_owner: String,
    cancel: AtomicBool,
    locale: String,
}

impl StorageScanTask {
    fn new(
        name: String,
        inputs: ScanInputs,
        requester: &str,
        scan_owner: &str,
        locale: String,
    ) -> Self {
        Self {
            base: TaskBase::new(name, requester.to_string()),
            inputs,
            scan_owner: scan_owner.to_string(),
            cancel: AtomicBool::new(false),
            locale,
        }
    }
}

#[async_trait]
impl Task for StorageScanTask {
    fn base(&self) -> &TaskBase {
        &self.base
    }

    fn deduplication_key(&self) -> Option<String> {
        Some(format!("storage-scan:{}", self.scan_owner))
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Running).await;
        self.base
            .set_cur_task(tr(&self.locale, "task.storage_scanning"))
            .await;
        let progress = ScanProgress {
            base: &self.base,
            cancel: &self.cancel,
        };
        match scan_storage(&self.inputs, Some(&progress)).await {
            Ok(ScanOutcome::Done(scan)) => {
                if let Err(error) =
                    save_report(&self.inputs.data_path, &self.inputs.username, &scan.report).await
                {
                    record_scan_failure(&self.inputs.username);
                    self.base.set_error_msg(error.to_string()).await;
                    self.base.set_status(TaskStatus::Failed).await;
                    return;
                }
                clear_scan_failure(&self.inputs.username);
                self.base.set_status(TaskStatus::Finished).await;
                self.base
                    .set_cur_task(tr(&self.locale, "task.storage_scan_done"))
                    .await;
            }
            Ok(ScanOutcome::Cancelled) => {
                self.base.set_status(TaskStatus::Cancelled).await;
            }
            Err(error) => {
                record_scan_failure(&self.inputs.username);
                self.base.set_error_msg(error.to_string()).await;
                self.base.set_status(TaskStatus::Failed).await;
            }
        }
    }
}

pub struct StorageCleanupTask {
    base: TaskBase,
    inputs: ScanInputs,
    /// The account whose caches are deleted; also the deduplication key.
    target_owner: String,
    spec: CleanupRequest,
    cancel: AtomicBool,
    locale: String,
}

impl StorageCleanupTask {
    fn new(
        name: String,
        inputs: ScanInputs,
        target_owner: &str,
        requester: &str,
        spec: CleanupRequest,
        locale: String,
    ) -> Self {
        Self {
            base: TaskBase::new(name, requester.to_string()),
            inputs,
            target_owner: target_owner.to_string(),
            spec,
            cancel: AtomicBool::new(false),
            locale,
        }
    }
}

#[async_trait]
impl Task for StorageCleanupTask {
    fn base(&self) -> &TaskBase {
        &self.base
    }

    fn deduplication_key(&self) -> Option<String> {
        Some(format!("storage-cleanup:{}", self.target_owner))
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Running).await;
        self.base
            .set_cur_task(tr(&self.locale, "task.storage_cleanup_scanning"))
            .await;
        let progress = ScanProgress {
            base: &self.base,
            cancel: &self.cancel,
        };
        let targets = match collect_cleanup_targets(&self.inputs, &self.spec, Some(&progress)).await
        {
            Ok(CleanupOutcome::Ready(targets)) => targets,
            Ok(CleanupOutcome::Cancelled) => {
                self.base.set_status(TaskStatus::Cancelled).await;
                return;
            }
            Err(error) => {
                self.base.set_error_msg(error.to_string()).await;
                self.base.set_status(TaskStatus::Failed).await;
                return;
            }
        };

        let total_bytes: u64 = targets.iter().map(|target| target.bytes).sum();
        self.base.set_total_count(targets.len());
        self.base.set_processed_count(0);
        self.base.set_total_cost(total_bytes);
        self.base.set_processed_cost(0);

        let mut failures = 0usize;
        let mut removed = 0usize;
        for target in &targets {
            if self.cancel.load(Ordering::Relaxed) {
                self.base.set_status(TaskStatus::Cancelled).await;
                break;
            }
            let Some(allowed) =
                allowed_root(&self.inputs.data_path, &self.inputs.username, target.kind)
            else {
                failures += 1;
                continue;
            };
            let file_name = target
                .path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            self.base
                .set_cur_task(format_subtask(
                    &tr(&self.locale, "task.storage_removing"),
                    &file_name,
                    removed + failures + 1,
                    targets.len(),
                ))
                .await;
            match remove_cache_file(&target.path, &allowed, self.spec.dry_run).await {
                Ok(true) => {
                    removed += 1;
                    self.base.add_processed_count(1);
                    self.base.add_processed_cost(target.bytes);
                }
                Ok(false) => {
                    failures += 1;
                }
                Err(error) => {
                    failures += 1;
                    eprintln!("storage cleanup failed for a cache file: {error}");
                }
            }
        }

        if removed > 0 && !self.spec.dry_run {
            invalidate_report(&self.inputs.data_path, &self.inputs.username).await;
        }
        if failures > 0 {
            self.base
                .set_error_msg(trf(
                    &self.locale,
                    "error.storage_cleanup_partial",
                    &[("count", failures.to_string())],
                ))
                .await;
        }
        if !self.cancel.load(Ordering::Relaxed) {
            self.base.set_status(TaskStatus::Finished).await;
        }
    }
}

// ── HTTP API ─────────────────────────────────────────────────────────────────

fn shared_album_count(sources: &[crate::backend::album::api::AlbumSource], username: &str) -> u64 {
    sources
        .iter()
        .filter(|source| source.owner() != username)
        .count() as u64
}

async fn scan_inputs(cfg: &AppState, username: &str) -> ScanInputs {
    let sources = collect_accessible_album_sources(cfg, username).await;
    ScanInputs {
        data_path: cfg.data_path.clone(),
        user_root: cfg.user_root_path(username),
        username: username.to_string(),
        shared_album_count: shared_album_count(&sources, username),
    }
}

/// Inputs for work that never reports a shared-album count. Building the full
/// `scan_inputs` walks every account's album directory, which made a cleanup
/// click wait on that walk before the task was even queued.
fn cleanup_inputs(cfg: &AppState, username: &str) -> ScanInputs {
    ScanInputs {
        data_path: cfg.data_path.clone(),
        user_root: cfg.user_root_path(username),
        username: username.to_string(),
        shared_album_count: 0,
    }
}

async fn queue_scan(
    cfg: &Arc<AppState>,
    requester: &str,
    target_owner: &str,
    locale: &str,
) -> (uuid::Uuid, bool) {
    let inputs = scan_inputs(cfg, target_owner).await;
    let name = if target_owner == requester {
        tr(locale, "task.storage_scan")
    } else {
        trf(
            locale,
            "task.storage_scan_member",
            &[("owner", target_owner.to_string())],
        )
    };
    let task: Arc<dyn Task> = Arc::new(StorageScanTask::new(
        name,
        inputs,
        requester,
        target_owner,
        locale.to_string(),
    ));
    cfg.task_mgr.add_task_if_absent(task).await
}

/// Resolve `owner` from a request: absent means the caller. An admin may name
/// another account; everyone else may only name themselves.
async fn resolve_target_owner(
    ctx: &Ctx,
    cfg: &AppState,
    requested: Option<&str>,
    locale: &str,
) -> Result<String, AppError> {
    let Some(raw) = requested.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(ctx.username().to_string());
    };
    if raw == ctx.username() {
        return Ok(ctx.username().to_string());
    }
    if !ctx.is_admin() {
        return Err(AppError::Forbidden(tr(
            locale,
            "error.admin_only_member_storage",
        )));
    }
    let Some(owner) = cache_owner(raw) else {
        return Err(AppError::NotFound(trf(
            locale,
            "error.resource_not_found",
            &[("value", raw.to_string())],
        )));
    };
    if !cfg.user_manager.user_exists(owner.as_str()).await {
        return Err(AppError::NotFound(trf(
            locale,
            "error.resource_not_found",
            &[("value", raw.to_string())],
        )));
    }
    Ok(owner)
}

/// A strict single path component: the account name as it appears on disk.
fn cache_owner(raw: &str) -> Option<String> {
    let name = raw.trim();
    if name.is_empty() || name.len() > 64 || name == "." || name == ".." {
        return None;
    }
    if name.contains(['/', '\\', '\0']) || !is_safe_cache_owner(name) {
        return None;
    }
    Some(name.to_string())
}

#[derive(Debug, Deserialize)]
pub struct StorageQuery {
    #[serde(default)]
    pub owner: Option<String>,
}

/// `GET /api/storage/report[?owner=…]` — a member's report needs an admin.
pub async fn api_storage_report(
    ctx: Ctx,
    Extension(cfg): Extension<Arc<AppState>>,
    Query(query): Query<StorageQuery>,
) -> ApiResult {
    let locale = ctx.locale.as_str();
    let target = resolve_target_owner(&ctx, &cfg, query.owner.as_deref(), locale).await?;
    let report = load_report(&cfg.data_path, &target).await;
    let scan_key = format!("storage-scan:{target}");
    let scanning = cfg.task_mgr.has_active_key(&scan_key).await;
    let mut queued = false;
    if report.is_none() && !scanning && !scan_recently_failed(&target) {
        queue_scan(&cfg, ctx.username(), &target, locale).await;
        queued = true;
    }
    let stale = report
        .as_ref()
        .map(|report| report_stale(report.generated_at))
        .unwrap_or(true);
    Ok(ctx.ok(
        "response.success",
        Some(json!({
            "owner": target,
            "report": report,
            "stale": stale,
            "scanning": scanning || queued,
        })),
    ))
}

/// One row of `GET /api/storage/members`.
#[derive(Debug, Clone, Serialize)]
pub struct StorageMemberRow {
    pub owner: String,
    pub is_self: bool,
    pub report_present: bool,
    pub generated_at: u64,
    pub stale: bool,
    pub scanning: bool,
    pub totals: StorageMemberTotals,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct StorageMemberTotals {
    pub files: u64,
    pub bytes: u64,
    pub originals_bytes: u64,
    pub thumbnails_bytes: u64,
    pub darktable_bytes: u64,
    pub transcodes_bytes: u64,
    pub ai_bytes: u64,
    pub cache_other_bytes: u64,
    pub albums: u64,
}

/// A member row from the cached report only: opening the page must not scan
/// every account's data root.
fn member_row(
    owner: &str,
    self_owner: &str,
    report: Option<StorageReport>,
    scanning: bool,
) -> StorageMemberRow {
    match report {
        Some(report) => {
            let totals = &report.totals;
            let originals = totals.albums.bytes
                + totals.resources.bytes
                + totals.outside.bytes
                + totals.temp.bytes;
            StorageMemberRow {
                owner: owner.to_string(),
                is_self: owner == self_owner,
                report_present: true,
                generated_at: report.generated_at,
                stale: report_stale(report.generated_at),
                scanning,
                totals: StorageMemberTotals {
                    files: totals.files,
                    bytes: totals.bytes,
                    originals_bytes: originals,
                    thumbnails_bytes: totals.thumbnails.bytes,
                    darktable_bytes: totals.darktable_previews.bytes,
                    transcodes_bytes: totals.transcodes.bytes + totals.transcode_probes.bytes,
                    ai_bytes: totals.ai.bytes,
                    cache_other_bytes: totals.cache_other.bytes,
                    albums: report.albums.len() as u64,
                },
            }
        }
        None => StorageMemberRow {
            owner: owner.to_string(),
            is_self: owner == self_owner,
            report_present: false,
            generated_at: 0,
            stale: true,
            scanning,
            totals: StorageMemberTotals::default(),
        },
    }
}

/// Caches whose layout is flat, i.e. shared by every account. They are listed
/// once for the whole instance instead of being attributed to a member.
async fn shared_cache_totals(data_path: &Path) -> Value {
    let mut rows = Vec::new();
    for area in cache_areas::CACHE_AREAS {
        if area.layout != CacheLayout::Flat {
            continue;
        }
        let (files, bytes) = cache_area_totals(&area.root(data_path)).await;
        rows.push(json!({ "name": area.name, "files": files, "bytes": bytes }));
    }
    // Previews written before the per-owner layout cannot be attributed. They
    // are aged out by the retention GC; until then, show them instead of hiding
    // them from every account.
    let previews = cache_area("darktable").root(data_path);
    let (files, bytes) = direct_files_totals(&previews).await;
    if files > 0 {
        rows.push(json!({ "name": "darktable (legacy)", "files": files, "bytes": bytes }));
    }
    json!(rows)
}

/// Bytes of files directly inside `root`, ignoring subdirectories.
async fn direct_files_totals(root: &Path) -> (u64, u64) {
    let Ok(mut entries) = tokio::fs::read_dir(root).await else {
        return (0, 0);
    };
    let (mut files, mut bytes) = (0u64, 0u64);
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if meta.is_file() {
            files += 1;
            bytes += meta.len();
        }
    }
    (files, bytes)
}

/// `GET /api/storage/members` — admin only: every account plus the flat caches.
pub async fn api_storage_members(ctx: Ctx, Extension(cfg): Extension<Arc<AppState>>) -> ApiResult {
    if !ctx.is_admin() {
        return Err(AppError::Forbidden(tr(
            ctx.locale.as_str(),
            "error.admin_only_member_storage",
        )));
    }
    let mut owners = cfg.user_manager.list_usernames().await;
    owners.sort();
    let mut rows = Vec::with_capacity(owners.len());
    for owner in &owners {
        let report = load_report(&cfg.data_path, owner).await;
        let scanning = cfg
            .task_mgr
            .has_active_key(&format!("storage-scan:{owner}"))
            .await;
        rows.push(member_row(owner, ctx.username(), report, scanning));
    }
    let shared = shared_cache_totals(cfg.data_path.as_path()).await;
    Ok(ctx.ok(
        "response.success",
        Some(json!({
            "members": rows,
            "shared_cache": shared,
            "generated_at": now_unix(),
        })),
    ))
}

/// `POST /api/storage/scan[?owner=…|?all=1]`.
#[derive(Debug, Default, Deserialize)]
pub struct ScanQuery {
    #[serde(default)]
    pub owner: Option<String>,
    /// `?all=1` and `?all=true` both mean "queue one scan per account".
    #[serde(default, deserialize_with = "deserialize_flag")]
    pub all: bool,
}

fn deserialize_flag<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    Ok(matches!(raw.as_deref(), Some("1" | "true" | "yes")))
}

/// How many member scans one request may queue; each walks a whole data root.
const MAX_MEMBER_SCAN_BATCH: usize = 4;

pub async fn api_storage_scan(
    ctx: Ctx,
    Extension(cfg): Extension<Arc<AppState>>,
    Query(query): Query<ScanQuery>,
) -> ApiResult {
    let locale = ctx.locale.as_str();
    if query.all {
        if !ctx.is_admin() {
            return Err(AppError::Forbidden(tr(
                locale,
                "error.admin_only_member_storage",
            )));
        }
        let mut owners = cfg.user_manager.list_usernames().await;
        owners.sort();
        let mut tasks = Vec::new();
        for owner in owners {
            if tasks.len() >= MAX_MEMBER_SCAN_BATCH {
                break;
            }
            if cfg
                .task_mgr
                .has_active_key(&format!("storage-scan:{owner}"))
                .await
            {
                continue;
            }
            let (task_id, started) = queue_scan(&cfg, ctx.username(), &owner, locale).await;
            tasks.push(json!({ "owner": owner, "task_id": task_id, "started": started }));
        }
        return Ok(ctx.ok(
            "response.storage_scan_started",
            Some(json!({ "tasks": tasks })),
        ));
    }

    let target = resolve_target_owner(&ctx, &cfg, query.owner.as_deref(), locale).await?;
    let (task_id, started) = queue_scan(&cfg, ctx.username(), &target, locale).await;
    Ok(ctx.ok(
        "response.storage_scan_started",
        Some(json!({ "task_id": task_id, "started": started, "owner": target })),
    ))
}

pub async fn api_storage_cleanup(
    ctx: Ctx,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(spec): Json<CleanupRequest>,
) -> ApiResult {
    let locale = ctx.locale.as_str();
    let target = resolve_target_owner(&ctx, &cfg, spec.owner.as_deref(), locale).await?;
    validate_cleanup(locale, &spec)?;
    let root = cfg.user_root_path(&target);

    if spec.target == CleanupTarget::Album {
        let album = spec.album.clone().unwrap_or_default();
        if !albums_root(&root).join(&album).is_dir() {
            return Err(AppError::NotFound(trf(
                locale,
                "error.storage_album_not_found",
                &[("value", album)],
            )));
        }
    }

    if cfg
        .task_mgr
        .has_active_key(&format!("storage-cleanup:{target}"))
        .await
    {
        return Err(AppError::BadRequest(tr(locale, "error.storage_conflict")));
    }
    if spec.kind == CleanupKind::Thumbnails
        && cfg
            .task_mgr
            .has_active_key(&format!("background-thumbnails:{target}"))
            .await
    {
        return Err(AppError::BadRequest(tr(locale, "error.storage_conflict")));
    }
    if spec.kind == CleanupKind::Transcodes
        && cfg.task_mgr.has_active_key_prefix("video-encode:").await
    {
        return Err(AppError::BadRequest(tr(locale, "error.storage_conflict")));
    }

    let inputs = cleanup_inputs(&cfg, &target);
    let name = match spec.kind {
        CleanupKind::Thumbnails => tr(locale, "task.storage_cleanup_thumbnails"),
        CleanupKind::Transcodes => tr(locale, "task.storage_cleanup_transcodes"),
    };
    let task: Arc<dyn Task> = Arc::new(StorageCleanupTask::new(
        name,
        inputs,
        &target,
        ctx.username(),
        spec.clone(),
        locale.to_string(),
    ));
    let (task_id, started) = cfg.task_mgr.add_task_if_absent(task).await;
    Ok(ctx.ok(
        "response.storage_cleanup_started",
        Some(json!({
            "task_id": task_id,
            "started": started,
            "dry_run": spec.dry_run,
            "owner": target,
        })),
    ))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_owner_names_are_single_path_components() {
        for bad in ["", " ", ".", "..", "a/b", "a\\b", "a\0b"] {
            assert!(super::cache_owner(bad).is_none(), "accepted {bad:?}");
        }
        assert_eq!(super::cache_owner("daniel").as_deref(), Some("daniel"));
        assert_eq!(super::cache_owner("  irene  ").as_deref(), Some("irene"));
    }

    #[test]
    fn member_rows_come_from_the_cached_report() {
        let missing = super::member_row("irene", "daniel", None, false);
        assert!(!missing.report_present);
        assert!(!missing.is_self);
        assert_eq!(missing.totals.bytes, 0);

        let mut report = super::StorageReport {
            version: super::REPORT_VERSION,
            generated_at: super::now_unix(),
            totals: Default::default(),
            formats: Vec::new(),
            albums: Vec::new(),
            outside: Vec::new(),
            months: Vec::new(),
            unindexed_albums: 0,
            shared_album_count: 0,
            videos: Vec::new(),
            thumbnail_sizes: Vec::new(),
            cache_areas: Vec::new(),
        };
        report.totals.albums.bytes = 1000;
        report.totals.thumbnails.bytes = 200;
        report.totals.ai.bytes = 30;
        report.totals.cache_other.bytes = 7;
        report.totals.recompute();
        let row = super::member_row("daniel", "daniel", Some(report), true);
        assert!(row.is_self && row.report_present && row.scanning);
        assert_eq!(row.totals.originals_bytes, 1000);
        assert_eq!(row.totals.thumbnails_bytes, 200);
        assert_eq!(row.totals.ai_bytes, 30);
        assert_eq!(row.totals.cache_other_bytes, 7);
        assert_eq!(row.totals.bytes, 1237);
    }

    #[test]
    fn scan_and_cleanup_deduplicate_on_the_target_account() {
        let inputs = super::ScanInputs {
            data_path: std::path::PathBuf::from("/data"),
            user_root: std::path::PathBuf::from("/data/irene"),
            username: "irene".to_string(),
            shared_album_count: 0,
        };
        let scan = super::StorageScanTask::new(
            "scan".to_string(),
            inputs.clone(),
            "daniel",
            "irene",
            "en".to_string(),
        );
        assert_eq!(
            super::Task::deduplication_key(&scan).as_deref(),
            Some("storage-scan:irene"),
            "the dedup key must not depend on who asked"
        );
        let cleanup = super::StorageCleanupTask::new(
            "cleanup".to_string(),
            inputs,
            "irene",
            "daniel",
            super::CleanupRequest {
                owner: None,
                kind: super::CleanupKind::Thumbnails,
                target: super::CleanupTarget::Unattributed,
                album: None,
                month: None,
                size: None,
                codec: None,
                key: None,
                dry_run: false,
            },
            "en".to_string(),
        );
        assert_eq!(
            super::Task::deduplication_key(&cleanup).as_deref(),
            Some("storage-cleanup:irene")
        );
    }

    /// The report must name every registered area, so a cache added to the
    /// registry cannot stay invisible in the UI.
    #[tokio::test]
    async fn report_covers_every_registered_cache_area() {
        let data = temp_data("areas");
        crate::backend::common::ensure_data_cache_layout(&data);
        for area in crate::backend::cache_areas::CACHE_AREAS {
            let dir = match area.layout {
                crate::backend::cache_areas::CacheLayout::Flat => area.root(&data),
                crate::backend::cache_areas::CacheLayout::PerOwner
                | crate::backend::cache_areas::CacheLayout::PerOwnerPerKind => {
                    area.owner_dir(&data, "daniel")
                }
            };
            tokio::fs::create_dir_all(&dir).await.unwrap();
            tokio::fs::write(dir.join("probe.bin"), b"12345")
                .await
                .unwrap();
        }
        let rows = scan_cache_areas(&data).await;
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        let expected: Vec<&str> = crate::backend::cache_areas::CACHE_AREAS
            .iter()
            .map(|area| area.name)
            .collect();
        assert_eq!(names, expected);
        for row in &rows {
            assert_eq!(row.files, 1, "{}", row.name);
            assert_eq!(row.bytes, 5, "{}", row.name);
        }
        // exif/exports/index/manifests/reports are the unattributed ones.
        assert!(
            rows.iter()
                .any(|row| row.name == "storage-reports" && row.gc)
        );
        let _ = std::fs::remove_dir_all(&data);
    }

    fn temp_data(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("lumeno-storage-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn inputs_for(data: &Path, user: &str) -> ScanInputs {
        ScanInputs {
            data_path: data.to_path_buf(),
            user_root: data.join(user),
            username: user.to_string(),
            shared_album_count: 0,
        }
    }

    async fn scan(inputs: &ScanInputs) -> StorageScan {
        match scan_storage(inputs, None).await.unwrap() {
            ScanOutcome::Done(scan) => *scan,
            ScanOutcome::Cancelled => panic!("scan was cancelled"),
        }
    }

    async fn write_thumbnail(data: &Path, user: &str, photo: &Path, size: u32) {
        let meta = tokio::fs::metadata(photo).await.unwrap();
        let key = thumbnail_cache_key_parts(
            user,
            data,
            photo,
            size,
            meta.len(),
            modified_secs(&meta),
            ThumbnailGenerator::Lumeno,
        );
        let dir = cache_area("thumbnails").owner_dir(&data, user);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join(format!("{key}.webp")), b"thumb")
            .await
            .unwrap();
    }

    #[test]
    fn normalizes_extensions_and_classifies_paths() {
        assert_eq!(ext_of("a.JPEG"), "jpg");
        assert_eq!(ext_of("a.CR2"), "cr2");
        assert_eq!(ext_of("noext"), "(none)");
        assert_eq!(
            categorize("albums/Trip/a.jpg"),
            (Category::Albums, Some("Trip".to_string()))
        );
        assert_eq!(
            categorize("albums/__resource__/lut/a.cube"),
            (Category::Resources, None)
        );
        assert_eq!(categorize("albums/__uploading/x"), (Category::Skip, None));
        assert_eq!(categorize("temp/a.bin"), (Category::Temp, None));
        assert_eq!(
            categorize("trans/a.jpg"),
            (Category::Outside, Some("trans".to_string()))
        );
        assert_eq!(
            categorize("lirui.zip"),
            (Category::Outside, Some(String::new()))
        );
    }

    #[test]
    fn month_bucket_rejects_unknown_dates() {
        assert_eq!(month_of("2026-04-10").as_deref(), Some("2026-04"));
        assert_eq!(month_of(""), None);
        assert_eq!(month_of("1970-01-01"), None);
        assert_eq!(month_of("2026-4"), None);
        assert_eq!(month_of("2026-13-01"), None);
    }

    #[tokio::test]
    async fn attributes_a_thumbnail_to_its_album() {
        let data = temp_data("attr");
        let user = "alice";
        let album_dir = data.join(user).join("albums").join("Trip");
        tokio::fs::create_dir_all(&album_dir).await.unwrap();
        let photo = album_dir.join("a.jpg");
        tokio::fs::write(&photo, b"jpeg-bytes").await.unwrap();
        write_thumbnail(&data, user, &photo, 360).await;

        let scan = scan(&inputs_for(&data, user)).await;
        assert_eq!(scan.report.totals.thumbnails.files, 1);
        assert_eq!(scan.report.totals.thumbnails.unattributed_files, 0);
        // The photo plus `album_meta.json`, which the index lookup materializes.
        assert_eq!(scan.report.totals.albums.files, 2);
        let trip = scan
            .report
            .albums
            .iter()
            .find(|row| row.album == "Trip")
            .expect("album row");
        assert_eq!(trip.files, 2);
        assert_eq!(trip.thumbnails_bytes, 5);
        assert_eq!(scan.report.unindexed_albums, 1);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[tokio::test]
    async fn darktable_previews_live_under_the_requesting_account() {
        let data = temp_data("darktable");
        let user = "alice";
        let album_dir = data.join(user).join("albums").join("Trip");
        tokio::fs::create_dir_all(&album_dir).await.unwrap();
        let photo = album_dir.join("a.cr2");
        tokio::fs::write(&photo, b"raw-bytes").await.unwrap();
        let meta = tokio::fs::metadata(&photo).await.unwrap();
        let dir = cache_area("darktable").owner_dir(&data, user);
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let key = darktable_preview_cache_key(
            &data,
            user,
            &photo,
            1600,
            meta.len(),
            modified_secs(&meta),
            None,
        );
        tokio::fs::write(dir.join(format!("{key}.jpg")), b"preview")
            .await
            .unwrap();
        // A preview of a source this account no longer has: in our own
        // directory, so it is unattributed but still ours to clean.
        tokio::fs::write(dir.join(format!("{}.jpg", "z".repeat(86))), b"stale")
            .await
            .unwrap();
        // Another account's previews never enter this account's report.
        let other = cache_area("darktable").owner_dir(&data, "bob");
        tokio::fs::create_dir_all(&other).await.unwrap();
        tokio::fs::write(other.join("b.jpg"), b"other")
            .await
            .unwrap();

        let scan = scan(&inputs_for(&data, user)).await;
        assert_eq!(scan.report.totals.darktable_previews.files, 2);
        assert_eq!(scan.report.totals.darktable_previews.bytes, 12);
        assert_eq!(
            scan.report.totals.darktable_previews.unattributed_files, 1,
            "the stale preview in our own directory is reported as unattributed"
        );
        let _ = std::fs::remove_dir_all(&data);
    }

    #[tokio::test]
    async fn edited_source_leaves_its_thumbnail_unattributed() {
        let data = temp_data("orphan");
        let user = "alice";
        let album_dir = data.join(user).join("albums").join("Trip");
        tokio::fs::create_dir_all(&album_dir).await.unwrap();
        let photo = album_dir.join("a.jpg");
        tokio::fs::write(&photo, b"first").await.unwrap();
        write_thumbnail(&data, user, &photo, 360).await;
        tokio::fs::write(&photo, b"second-with-other-length")
            .await
            .unwrap();

        let scan = scan(&inputs_for(&data, user)).await;
        assert_eq!(scan.report.totals.thumbnails.files, 1);
        assert_eq!(scan.report.totals.thumbnails.unattributed_files, 1);
        assert_eq!(scan.report.totals.thumbnails.unattributed_bytes, 5);
        assert_eq!(scan.report.albums[0].thumbnails_bytes, 0);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[tokio::test]
    async fn splits_albums_resources_outside_and_temp() {
        let data = temp_data("split");
        let user = "alice";
        let root = data.join(user);
        for path in [
            "albums/Trip/x.jpg",
            "albums/__resource__/lut/a.cube",
            "trans/y.JPEG",
            "temp/z.tmp",
        ] {
            let path = root.join(path);
            tokio::fs::create_dir_all(path.parent().unwrap())
                .await
                .unwrap();
            tokio::fs::write(&path, b"data").await.unwrap();
        }
        tokio::fs::write(root.join("lirui.zip"), b"zip")
            .await
            .unwrap();
        // Staging uploads are not counted at all.
        tokio::fs::create_dir_all(root.join("albums/__uploading"))
            .await
            .unwrap();
        tokio::fs::write(root.join("albums/__uploading/part.jpg"), b"part")
            .await
            .unwrap();

        let scan = scan(&inputs_for(&data, user)).await;
        // The photo plus the materialized `album_meta.json`.
        assert_eq!(scan.report.totals.albums.files, 2);
        assert_eq!(scan.report.totals.resources.files, 1);
        assert_eq!(scan.report.totals.temp.files, 1);
        // `trans/y.JPEG` and the root zip.
        assert_eq!(scan.report.totals.outside.files, 2);
        let jpg = scan
            .report
            .formats
            .iter()
            .find(|row| row.ext == "jpg")
            .expect("jpg row");
        assert_eq!(jpg.files, 2);
        assert!(
            scan.report
                .outside
                .iter()
                .any(|row| row.top.is_empty() && row.files == 1)
        );
        let _ = std::fs::remove_dir_all(&data);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn does_not_follow_symlinks_out_of_the_data_root() {
        let data = temp_data("symlink");
        let user = "alice";
        let root = data.join(user);
        tokio::fs::create_dir_all(root.join("albums"))
            .await
            .unwrap();
        let outside = data.join("elsewhere");
        tokio::fs::create_dir_all(&outside).await.unwrap();
        tokio::fs::write(outside.join("secret.jpg"), b"secret")
            .await
            .unwrap();
        std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();

        let scan = scan(&inputs_for(&data, user)).await;
        assert_eq!(scan.report.totals.files, 0);
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn cleanup_validation_rejects_bad_scopes() {
        let spec = |kind: CleanupKind, target: CleanupTarget| CleanupRequest {
            owner: None,
            kind,
            target,
            album: None,
            month: None,
            size: None,
            codec: None,
            key: None,
            dry_run: false,
        };

        let mut key_spec = spec(CleanupKind::Thumbnails, CleanupTarget::Key);
        key_spec.key = Some("../../etc".to_string());
        assert!(validate_cleanup("en", &key_spec).is_err());
        key_spec.key = Some("a".repeat(86));
        assert!(validate_cleanup("en", &key_spec).is_ok());

        let mut month_spec = spec(CleanupKind::Thumbnails, CleanupTarget::Month);
        month_spec.month = Some("2026-13".to_string());
        assert!(validate_cleanup("en", &month_spec).is_err());
        month_spec.month = Some("2026-04".to_string());
        assert!(validate_cleanup("en", &month_spec).is_ok());

        let mut size_spec = spec(CleanupKind::Thumbnails, CleanupTarget::Size);
        size_spec.size = Some(99999);
        assert!(validate_cleanup("en", &size_spec).is_err());
        size_spec.size = Some(360);
        assert!(validate_cleanup("en", &size_spec).is_ok());

        // Size only means something for thumbnails; codec only for transcodes.
        assert!(
            validate_cleanup("en", &spec(CleanupKind::Transcodes, CleanupTarget::Size)).is_err()
        );
        assert!(
            validate_cleanup("en", &spec(CleanupKind::Thumbnails, CleanupTarget::Codec)).is_err()
        );

        let mut codec_spec = spec(CleanupKind::Transcodes, CleanupTarget::Codec);
        codec_spec.codec = Some("mp4".to_string());
        assert!(validate_cleanup("en", &codec_spec).is_err());
        codec_spec.codec = Some("h264".to_string());
        assert!(validate_cleanup("en", &codec_spec).is_ok());

        let mut album_spec = spec(CleanupKind::Thumbnails, CleanupTarget::Album);
        album_spec.album = Some("../Trip".to_string());
        assert!(validate_cleanup("en", &album_spec).is_err());
        album_spec.album = Some("Trip".to_string());
        assert!(validate_cleanup("en", &album_spec).is_ok());
    }

    #[tokio::test]
    async fn cleanup_refuses_paths_outside_its_cache_root() {
        let data = temp_data("cleanup");
        let allowed = data.join("thumbnails");
        tokio::fs::create_dir_all(&allowed).await.unwrap();
        let inside = allowed.join("a.webp");
        tokio::fs::write(&inside, b"thumb").await.unwrap();

        assert!(remove_cache_file(&inside, &allowed, true).await.unwrap());
        assert!(inside.exists(), "dry run must not delete");
        assert!(remove_cache_file(&inside, &allowed, false).await.unwrap());
        assert!(!inside.exists());

        let outside = data.join("elsewhere.webp");
        tokio::fs::write(&outside, b"x").await.unwrap();
        assert!(!remove_cache_file(&outside, &allowed, false).await.unwrap());
        assert!(outside.exists());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[tokio::test]
    async fn report_roundtrip_and_version_gate() {
        let data = temp_data("report");
        let mut report = StorageReport {
            version: REPORT_VERSION,
            generated_at: now_unix(),
            ..Default::default()
        };
        save_report(&data, "alice", &report).await.unwrap();
        let loaded = load_report(&data, "alice").await.expect("report");
        assert_eq!(loaded.version, REPORT_VERSION);
        assert!(!report_stale(loaded.generated_at));

        report.version = REPORT_VERSION + 1;
        save_report(&data, "alice", &report).await.unwrap();
        assert!(load_report(&data, "alice").await.is_none());

        invalidate_report(&data, "alice").await;
        assert!(load_report(&data, "alice").await.is_none());
        let _ = std::fs::remove_dir_all(&data);
    }

    #[test]
    fn stale_reports_age_out() {
        assert!(!report_stale(now_unix()));
        assert!(report_stale(
            now_unix().saturating_sub(REPORT_STALE_SECS + 1)
        ));
    }
}
