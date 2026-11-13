//! File-level burst stacking.
//!
//! A burst is a contiguous run of album files in capture-time order that share
//! camera, orientation, and a close DCT pHash. The pHash is the same value the
//! duplicate detector writes under `cache/ai/{owner}/albums/{cache_id}/phash/`;
//! this module never owns a second hash cache.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::backend::ai::{
    AlbumAiCache, PHASH_CACHE_VERSION, PhashCacheFile, build_photo_content_signature,
    build_photo_stable_identity, compute_photo_phash_preferring_preview, phash_cache_key,
};
use crate::backend::album::helpers::{
    album_index_path_for_dir, albums_root, ensure_album_cache_id, load_existing_album_index,
    read_album_meta,
};
use crate::backend::album::media::preview_thumbnail_cache_path;
use crate::backend::album::types::{AlbumIndex, AlbumPhotoItem};
use crate::backend::cache_areas;
use crate::backend::common::{AppError, AppState, into_host_path_safely};
use crate::backend::i18n::trf;
use crate::backend::tasks::{Task, TaskBase, TaskStatus};

pub(crate) const BURST_ALGORITHM: &str = "burst-v1";
pub(crate) const BURST_MAX_GAP_MS: i64 = 2_000;
pub(crate) const BURST_MAX_SPAN_MS: i64 = 15_000;
pub(crate) const BURST_MAX_TIME_WINDOW_SECONDS: u32 = 86_400;
pub(crate) const BURST_MAX_HAMMING: u32 = 10;
pub(crate) const BURST_MAX_HAMMING_LIMIT: u32 = 32;
pub(crate) const BURST_MIN_MEMBERS: usize = 2;
pub(crate) const BURST_MAX_MEMBERS: usize = 20;
const BURST_CACHE_VERSION: u32 = 1;
pub(crate) const BURST_EMBEDDING_THRESHOLD: u32 = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BurstAlgorithm {
    Dct,
    Embedding,
}

impl BurstAlgorithm {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Dct => "dct",
            Self::Embedding => "embedding",
        }
    }

    fn cache_name(self) -> String {
        format!("{BURST_ALGORITHM}:{}", self.as_str())
    }

    fn from_meta(meta: &crate::backend::album::types::AlbumMeta) -> Self {
        match meta.burst_algorithm() {
            "embedding" => Self::Embedding,
            _ => Self::Dct,
        }
    }

    fn threshold(self, meta: &crate::backend::album::types::AlbumMeta) -> u32 {
        match self {
            Self::Dct => meta.burst_threshold(),
            Self::Embedding => meta.burst_embedding_threshold(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub(crate) struct BurstSourceIndex {
    pub(crate) entries: usize,
    pub(crate) modified_unix_ms: u64,
    pub(crate) len: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BurstStack {
    pub(crate) id: String,
    pub(crate) cover: String,
    pub(crate) count: usize,
    pub(crate) members: Vec<String>,
    pub(crate) start_datetime: String,
    pub(crate) end_datetime: String,
    pub(crate) max_hamming_distance: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BurstCacheFile {
    pub(crate) schema_version: u32,
    pub(crate) algorithm: String,
    pub(crate) source_index: BurstSourceIndex,
    pub(crate) created_at_unix_ms: u128,
    #[serde(default = "default_burst_max_hamming")]
    pub(crate) max_hamming: u32,
    #[serde(default)]
    pub(crate) threshold: u32,
    #[serde(default = "default_time_window_seconds")]
    pub(crate) time_window_seconds: u32,
    pub(crate) stacks: Vec<BurstStack>,
}

fn default_burst_max_hamming() -> u32 {
    BURST_MAX_HAMMING
}

fn default_time_window_seconds() -> u32 {
    2
}

impl BurstCacheFile {
    fn is_valid_for(
        &self,
        source_index: &BurstSourceIndex,
        algorithm: BurstAlgorithm,
        threshold: u32,
        time_window_seconds: u32,
    ) -> bool {
        let legacy_dct = algorithm == BurstAlgorithm::Dct && self.algorithm == BURST_ALGORITHM;
        let cache_threshold = if self.threshold == 0 {
            self.max_hamming
        } else {
            self.threshold
        };
        self.schema_version == BURST_CACHE_VERSION
            && (legacy_dct || self.algorithm == algorithm.cache_name())
            && cache_threshold == threshold
            && self.time_window_seconds == time_window_seconds
            && &self.source_index == source_index
    }
}

#[derive(Debug)]
pub(crate) struct BurstLookup {
    pub(crate) state: &'static str,
    pub(crate) reason: Option<&'static str>,
    pub(crate) task_key: Option<String>,
    pub(crate) algorithm: BurstAlgorithm,
    pub(crate) threshold: u32,
    pub(crate) time_window_seconds: u32,
    pub(crate) cache: Option<BurstCacheFile>,
}

pub(crate) enum BurstDisplay<'a> {
    Photo(&'a AlbumPhotoItem),
    Stack {
        stack: &'a BurstStack,
        cover: &'a AlbumPhotoItem,
    },
}

struct BurstCandidate<'a> {
    entry: &'a AlbumPhotoItem,
    rel: String,
    time: NaiveDateTime,
    phash: u64,
    embedding: Option<&'a [f32]>,
}

pub(crate) fn album_relative_path(album: &str, path: &str) -> Option<String> {
    path.strip_prefix(&format!("albums/{album}/"))
        .map(|rel| rel.replace('\\', "/"))
        .filter(|rel| !rel.is_empty())
}

fn parse_photo_time(value: &str) -> Option<NaiveDateTime> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y:%m:%d %H:%M:%S"))
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S"))
        .ok()
}

fn camera_matches(left: &AlbumPhotoItem, right: &AlbumPhotoItem) -> bool {
    let left = format!("{} {}", left.camera_make, left.camera_model)
        .trim()
        .to_ascii_lowercase();
    let right = format!("{} {}", right.camera_make, right.camera_model)
        .trim()
        .to_ascii_lowercase();
    left.is_empty() || right.is_empty() || left == right
}

fn orientation_matches(left: &AlbumPhotoItem, right: &AlbumPhotoItem) -> bool {
    if left.width == 0 || left.height == 0 || right.width == 0 || right.height == 0 {
        return true;
    }
    (left.width >= left.height) == (right.width >= right.height)
}

fn hamming(left: u64, right: u64) -> u32 {
    (left ^ right).count_ones()
}

fn stack_id(members: &[String]) -> String {
    let mut sorted = members.to_vec();
    sorted.sort();
    let mut hasher = Sha256::new();
    hasher.update(format!("{BURST_ALGORITHM}|").as_bytes());
    for member in sorted {
        hasher.update(member.as_bytes());
        hasher.update(b"\n");
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize())
}

fn choose_cover(candidates: &[BurstCandidate<'_>]) -> usize {
    let middle = (candidates.len().saturating_sub(1)) as i64;
    let mut best = 0usize;
    for (index, candidate) in candidates.iter().enumerate() {
        let current = &candidates[best];
        let candidate_area = u64::from(candidate.entry.width) * u64::from(candidate.entry.height);
        let best_area = u64::from(current.entry.width) * u64::from(current.entry.height);
        let candidate_middle = (2 * index as i64 - middle).abs();
        let best_middle = (2 * best as i64 - middle).abs();
        let better = candidate.entry.rating > current.entry.rating
            || (candidate.entry.rating == current.entry.rating && candidate_area > best_area)
            || (candidate.entry.rating == current.entry.rating
                && candidate_area == best_area
                && candidate_middle < best_middle);
        if better {
            best = index;
        }
    }
    best
}

pub(crate) fn build_stacks(
    entries: &[AlbumPhotoItem],
    album_name: &str,
    phashes: &HashMap<String, u64>,
    max_hamming: u32,
    time_window_seconds: u32,
) -> Vec<BurstStack> {
    let max_gap_ms = i64::from(time_window_seconds.clamp(1, BURST_MAX_TIME_WINDOW_SECONDS)) * 1000;
    let max_span_ms = BURST_MAX_SPAN_MS.max(max_gap_ms);
    let mut candidates: Vec<BurstCandidate<'_>> = entries
        .iter()
        .filter(|entry| entry.duration_ms == 0)
        .filter_map(|entry| {
            let rel = album_relative_path(album_name, &entry.path)?;
            let time = parse_photo_time(&entry.datetime)?;
            let phash = *phashes.get(&rel)?;
            Some(BurstCandidate {
                entry,
                rel,
                time,
                phash,
                embedding: None,
            })
        })
        .collect();
    candidates.sort_by(|left, right| {
        left.time
            .cmp(&right.time)
            .then_with(|| left.entry.name.cmp(&right.entry.name))
    });

    let mut stacks = Vec::new();
    let mut index = 0usize;
    while index < candidates.len() {
        let mut group = vec![index];
        let mut next = index + 1;
        while next < candidates.len() && group.len() < BURST_MAX_MEMBERS {
            let candidate = &candidates[next];
            let last = &candidates[*group.last().unwrap()];
            let first = &candidates[group[0]];
            if candidate
                .time
                .signed_duration_since(last.time)
                .num_milliseconds()
                > max_gap_ms
            {
                break;
            }
            if candidate
                .time
                .signed_duration_since(first.time)
                .num_milliseconds()
                > max_span_ms
            {
                break;
            }
            if !camera_matches(first.entry, candidate.entry)
                || !orientation_matches(first.entry, candidate.entry)
            {
                break;
            }
            if group
                .iter()
                .any(|member| hamming(candidates[*member].phash, candidate.phash) > max_hamming)
            {
                break;
            }
            group.push(next);
            next += 1;
        }

        if group.len() >= BURST_MIN_MEMBERS {
            let members: Vec<String> = group
                .iter()
                .map(|member| candidates[*member].rel.clone())
                .collect();
            let mut max_distance = 0u32;
            for left in 0..group.len() {
                for right in (left + 1)..group.len() {
                    max_distance = max_distance.max(hamming(
                        candidates[group[left]].phash,
                        candidates[group[right]].phash,
                    ));
                }
            }
            let cover_index = choose_cover(
                &group
                    .iter()
                    .map(|member| BurstCandidate {
                        entry: candidates[*member].entry,
                        rel: candidates[*member].rel.clone(),
                        time: candidates[*member].time,
                        phash: candidates[*member].phash,
                        embedding: None,
                    })
                    .collect::<Vec<_>>(),
            );
            let cover = candidates[group[cover_index]].rel.clone();
            let first = &candidates[group[0]];
            let last = &candidates[*group.last().unwrap()];
            stacks.push(BurstStack {
                id: stack_id(&members),
                cover,
                count: members.len(),
                members,
                start_datetime: first.entry.datetime.clone(),
                end_datetime: last.entry.datetime.clone(),
                max_hamming_distance: max_distance,
            });
            index = next;
        } else {
            index += 1;
        }
    }
    stacks
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut left_norm = 0.0f32;
    let mut right_norm = 0.0f32;
    for (a, b) in left.iter().zip(right.iter()) {
        dot += a * b;
        left_norm += a * a;
        right_norm += b * b;
    }
    if left_norm <= 0.0 || right_norm <= 0.0 {
        return 0.0;
    }
    dot / (left_norm.sqrt() * right_norm.sqrt())
}

pub(crate) fn build_stacks_embeddings(
    entries: &[AlbumPhotoItem],
    album_name: &str,
    embeddings: &HashMap<String, Vec<f32>>,
    min_cosine: f32,
    time_window_seconds: u32,
) -> Vec<BurstStack> {
    let max_gap_ms = i64::from(time_window_seconds.clamp(1, BURST_MAX_TIME_WINDOW_SECONDS)) * 1000;
    let max_span_ms = BURST_MAX_SPAN_MS.max(max_gap_ms);
    let mut candidates: Vec<BurstCandidate<'_>> = entries
        .iter()
        .filter(|entry| entry.duration_ms == 0)
        .filter_map(|entry| {
            let rel = album_relative_path(album_name, &entry.path)?;
            let time = parse_photo_time(&entry.datetime)?;
            let vector = embeddings.get(&rel)?;
            Some(BurstCandidate {
                entry,
                rel,
                time,
                phash: 0,
                embedding: Some(vector.as_slice()),
            })
        })
        .collect();
    candidates.sort_by(|left, right| {
        left.time
            .cmp(&right.time)
            .then_with(|| left.entry.name.cmp(&right.entry.name))
    });

    let mut stacks = Vec::new();
    let mut index = 0usize;
    while index < candidates.len() {
        let mut group = vec![index];
        let mut next = index + 1;
        while next < candidates.len() && group.len() < BURST_MAX_MEMBERS {
            let candidate = &candidates[next];
            let last = &candidates[*group.last().unwrap()];
            let first = &candidates[group[0]];
            if candidate
                .time
                .signed_duration_since(last.time)
                .num_milliseconds()
                > max_gap_ms
            {
                break;
            }
            if candidate
                .time
                .signed_duration_since(first.time)
                .num_milliseconds()
                > max_span_ms
            {
                break;
            }
            if !camera_matches(first.entry, candidate.entry)
                || !orientation_matches(first.entry, candidate.entry)
            {
                break;
            }
            let Some(candidate_vector) = candidate.embedding else {
                break;
            };
            if group.iter().any(|member| {
                let Some(vector) = candidates[*member].embedding else {
                    return true;
                };
                cosine_similarity(vector, candidate_vector) < min_cosine
            }) {
                break;
            }
            group.push(next);
            next += 1;
        }

        if group.len() >= BURST_MIN_MEMBERS {
            let members: Vec<String> = group
                .iter()
                .map(|member| candidates[*member].rel.clone())
                .collect();
            let cover_index = choose_cover(
                &group
                    .iter()
                    .map(|member| BurstCandidate {
                        entry: candidates[*member].entry,
                        rel: candidates[*member].rel.clone(),
                        time: candidates[*member].time,
                        phash: 0,
                        embedding: candidates[*member].embedding,
                    })
                    .collect::<Vec<_>>(),
            );
            let cover = candidates[group[cover_index]].rel.clone();
            let first = &candidates[group[0]];
            let last = &candidates[*group.last().unwrap()];
            stacks.push(BurstStack {
                id: stack_id(&members),
                cover,
                count: members.len(),
                members,
                start_datetime: first.entry.datetime.clone(),
                end_datetime: last.entry.datetime.clone(),
                max_hamming_distance: 0,
            });
            index = next;
        } else {
            index += 1;
        }
    }
    stacks
}

pub(crate) fn display_items<'a>(
    index: &'a AlbumIndex,
    cache: &'a BurstCacheFile,
    album_name: &str,
) -> Vec<BurstDisplay<'a>> {
    let by_rel: HashMap<String, &AlbumPhotoItem> = index
        .entries
        .iter()
        .filter_map(|entry| album_relative_path(album_name, &entry.path).map(|rel| (rel, entry)))
        .collect();
    let mut stacked = HashSet::new();
    let mut display = Vec::new();
    for stack in &cache.stacks {
        for member in &stack.members {
            stacked.insert(member.clone());
        }
        let Some(cover) = by_rel.get(&stack.cover).copied().or_else(|| {
            stack
                .members
                .iter()
                .find_map(|member| by_rel.get(member).copied())
        }) else {
            continue;
        };
        display.push(BurstDisplay::Stack { stack, cover });
    }
    for entry in &index.entries {
        let Some(rel) = album_relative_path(album_name, &entry.path) else {
            continue;
        };
        if !stacked.contains(&rel) {
            display.push(BurstDisplay::Photo(entry));
        }
    }
    display
}

fn cache_path(data_path: &Path, owner: &str, cache_id: &str) -> PathBuf {
    cache_areas::area("bursts")
        .expect("registered cache area")
        .root(data_path)
        .join(owner)
        .join(format!("{cache_id}.json"))
}

async fn source_index(
    data_path: &Path,
    owner: &str,
    album_dir: &Path,
    entries: usize,
) -> Result<BurstSourceIndex, AppError> {
    let index_path = album_index_path_for_dir(data_path, owner, album_dir).await?;
    let metadata = tokio::fs::metadata(&index_path)
        .await
        .map_err(|error| AppError::Unknown(format!("read burst source index failed: {error}")))?;
    let modified_unix_ms = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0);
    Ok(BurstSourceIndex {
        entries,
        modified_unix_ms,
        len: metadata.len(),
    })
}

pub(crate) async fn read_cache(
    data_path: &Path,
    owner: &str,
    cache_id: &str,
) -> Option<BurstCacheFile> {
    let raw = tokio::fs::read_to_string(cache_path(data_path, owner, cache_id))
        .await
        .ok()?;
    serde_json::from_str(&raw).ok()
}

pub(crate) async fn write_cache(
    data_path: &Path,
    owner: &str,
    cache_id: &str,
    cache: &BurstCacheFile,
) -> Result<(), String> {
    let path = cache_path(data_path, owner, cache_id);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| format!("create burst cache dir failed: {error}"))?;
    }
    let bytes = serde_json::to_vec_pretty(cache)
        .map_err(|error| format!("serialize burst cache: {error}"))?;
    let temp = path.with_extension("json.tmp");
    tokio::fs::write(&temp, &bytes)
        .await
        .map_err(|error| format!("write burst cache failed: {error}"))?;
    tokio::fs::rename(&temp, &path)
        .await
        .map_err(|error| format!("replace burst cache failed: {error}"))
}

async fn load_phash_map(
    cfg: &AppState,
    owner: &str,
    album_name: &str,
    album_dir: &Path,
    index: &AlbumIndex,
    cache_id: &str,
    base: Option<&TaskBase>,
) -> HashMap<String, u64> {
    let cache = AlbumAiCache::new(cfg.ai_service.store().root(), owner, cache_id);
    let mut hashes = HashMap::new();
    for (position, entry) in index.entries.iter().enumerate() {
        let Some(rel) = album_relative_path(album_name, &entry.path) else {
            continue;
        };
        let (stable_key, _) = build_photo_stable_identity(owner, album_name, &entry.path);
        let signature = build_photo_content_signature(entry.size, entry.modified);
        let key = phash_cache_key(&stable_key, &signature);
        if let Some(cached) = cache.read_phash_async(&key).await
            && cached.version == PHASH_CACHE_VERSION
            && cached.source == rel
            && cached.content_signature.as_deref() == Some(signature.as_str())
        {
            hashes.insert(rel, cached.phash_u64);
            if let Some(base) = base {
                base.set_processed_count(position + 1);
            }
            continue;
        }

        let Ok(original) = into_host_path_safely(album_dir, &rel) else {
            continue;
        };
        let Ok(metadata) = tokio::fs::metadata(&original).await else {
            continue;
        };
        let preview = preview_thumbnail_cache_path(&cfg.data_path, owner, &original, &metadata);
        let original_for_hash = original.clone();
        let preview_for_hash = preview.clone();
        let computed = tokio::task::spawn_blocking(move || {
            compute_photo_phash_preferring_preview(&original_for_hash, preview_for_hash.as_deref())
        })
        .await
        .ok()
        .and_then(Result::ok);
        if let Some(phash) = computed {
            let file = PhashCacheFile::new(&stable_key, &rel, phash, Some(&signature));
            let _ = cache.write_phash_async(&key, &file).await;
            hashes.insert(rel, phash);
        }
        if let Some(base) = base {
            base.set_processed_count(position + 1);
        }
    }
    hashes
}

async fn load_embedding_map(
    cfg: &AppState,
    owner: &str,
    cache_id: &str,
) -> HashMap<String, Vec<f32>> {
    let cache = AlbumAiCache::new(cfg.ai_service.store().root(), owner, cache_id);
    let Ok(entries) = cache.list_embeddings_async().await else {
        return HashMap::new();
    };
    let mut vectors = HashMap::new();
    for (_, file) in entries {
        let Ok(vector) = file.decode_vector() else {
            continue;
        };
        if vector.is_empty() {
            continue;
        }
        vectors.insert(file.source.replace('\\', "/"), vector);
    }
    vectors
}

pub(crate) async fn build_cache(
    cfg: &AppState,
    owner: &str,
    album_name: &str,
    cache_id: &str,
    index: &AlbumIndex,
    algorithm: BurstAlgorithm,
    threshold: u32,
    time_window_seconds: u32,
    base: Option<&TaskBase>,
) -> Result<BurstCacheFile, String> {
    let album_dir = albums_root(&cfg.user_root_path(owner)).join(album_name);
    let source = source_index(&cfg.data_path, owner, &album_dir, index.entries.len())
        .await
        .map_err(|error| format!("{error}"))?;
    if let Some(base) = base {
        base.set_total_count(index.entries.len());
        base.set_status(TaskStatus::Running).await;
    }
    let stacks = match algorithm {
        BurstAlgorithm::Dct => {
            let hashes =
                load_phash_map(cfg, owner, album_name, &album_dir, index, cache_id, base).await;
            build_stacks(
                &index.entries,
                album_name,
                &hashes,
                threshold,
                time_window_seconds,
            )
        }
        BurstAlgorithm::Embedding => {
            let vectors = load_embedding_map(cfg, owner, cache_id).await;
            build_stacks_embeddings(
                &index.entries,
                album_name,
                &vectors,
                threshold as f32 / 100.0,
                time_window_seconds,
            )
        }
    };
    Ok(BurstCacheFile {
        schema_version: BURST_CACHE_VERSION,
        algorithm: algorithm.cache_name(),
        source_index: source,
        created_at_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_millis())
            .unwrap_or(0),
        max_hamming: if algorithm == BurstAlgorithm::Dct {
            threshold
        } else {
            0
        },
        threshold,
        time_window_seconds,
        stacks,
    })
}

/// Ensure one album's burst cache is current. Used both by the AI pipeline's
/// final phase and by the album workflow fallback when AI is disabled.
pub(crate) async fn ensure_for_album(
    cfg: &AppState,
    owner: &str,
    album_name: &str,
    base: Option<&TaskBase>,
) -> Result<usize, String> {
    let root_path = cfg.user_root_path(owner);
    let album_dir = albums_root(&root_path).join(album_name);
    let index = load_existing_album_index(&cfg.data_path, &root_path, owner, album_name)
        .await
        .map_err(|error| format!("{error}"))?
        .ok_or_else(|| "album index not found".to_string())?;
    let meta = read_album_meta(&album_dir).await;
    let algorithm = BurstAlgorithm::from_meta(&meta);
    let threshold = algorithm.threshold(&meta);
    let time_window_seconds = meta.burst_time_window_seconds();
    let cache_id = ensure_album_cache_id(&album_dir)
        .await
        .map_err(|error| format!("{error}"))?;
    let source = source_index(&cfg.data_path, owner, &album_dir, index.entries.len())
        .await
        .map_err(|error| format!("{error}"))?;
    if let Some(cache) = read_cache(&cfg.data_path, owner, &cache_id).await
        && cache.is_valid_for(&source, algorithm, threshold, time_window_seconds)
    {
        return Ok(cache.stacks.len());
    }
    let cache = build_cache(
        cfg,
        owner,
        album_name,
        &cache_id,
        &index,
        algorithm,
        threshold,
        time_window_seconds,
        base,
    )
    .await?;
    write_cache(&cfg.data_path, owner, &cache_id, &cache).await?;
    Ok(cache.stacks.len())
}

pub(crate) async fn queue_burst_index(
    cfg: Arc<AppState>,
    requester: String,
    owner: String,
    album_name: String,
    cache_id: String,
    algorithm: BurstAlgorithm,
    threshold: u32,
    time_window_seconds: u32,
    locale: String,
) -> String {
    let key = format!(
        "burst-index:{owner}:{album_name}:{}:{threshold}:{time_window_seconds}",
        algorithm.as_str()
    );
    let name = trf(
        &locale,
        "task.burst_index",
        &[("album", album_name.clone())],
    );
    let task = Arc::new(BurstIndexTask {
        base: TaskBase::new(name, requester),
        cfg,
        owner,
        album_name,
        cache_id,
        algorithm,
        threshold,
        time_window_seconds,
        locale,
    });
    let task_manager = task.cfg.task_mgr.clone();
    let _ = task_manager.add_task_if_absent(task).await;
    key
}

pub(crate) async fn lookup(
    cfg: &Arc<AppState>,
    requester: &str,
    owner: &str,
    album_name: &str,
    album_dir: &Path,
    index: &AlbumIndex,
    eligible: bool,
    locale: &str,
) -> BurstLookup {
    let meta = read_album_meta(album_dir).await;
    let algorithm = BurstAlgorithm::from_meta(&meta);
    let threshold = algorithm.threshold(&meta);
    let time_window_seconds = meta.burst_time_window_seconds();
    if !eligible {
        return BurstLookup {
            state: "disabled",
            reason: Some("filters_or_sort"),
            task_key: None,
            algorithm,
            threshold,
            time_window_seconds,
            cache: None,
        };
    }
    let Ok(cache_id) = ensure_album_cache_id(album_dir).await else {
        return BurstLookup {
            state: "error",
            reason: None,
            task_key: None,
            algorithm,
            threshold,
            time_window_seconds,
            cache: None,
        };
    };
    let Ok(source) = source_index(&cfg.data_path, owner, album_dir, index.entries.len()).await
    else {
        return BurstLookup {
            state: "error",
            reason: None,
            task_key: None,
            algorithm,
            threshold,
            time_window_seconds,
            cache: None,
        };
    };
    let cached = read_cache(&cfg.data_path, owner, &cache_id).await;
    if let Some(cache) = cached.as_ref()
        && cache.is_valid_for(&source, algorithm, threshold, time_window_seconds)
    {
        return BurstLookup {
            state: "ready",
            reason: None,
            task_key: None,
            algorithm,
            threshold,
            time_window_seconds,
            cache: cached,
        };
    }
    let key = queue_burst_index(
        cfg.clone(),
        requester.to_string(),
        owner.to_string(),
        album_name.to_string(),
        cache_id,
        algorithm,
        threshold,
        time_window_seconds,
        locale.to_string(),
    )
    .await;
    BurstLookup {
        state: if cached.is_some() { "stale" } else { "pending" },
        reason: None,
        task_key: Some(key),
        algorithm,
        threshold,
        time_window_seconds,
        cache: None,
    }
}

pub(crate) fn stack_json(stack: &BurstStack) -> Value {
    serde_json::json!({
        "id": stack.id,
        "count": stack.count,
        "start_datetime": stack.start_datetime,
        "end_datetime": stack.end_datetime,
        "max_hamming_distance": stack.max_hamming_distance,
    })
}

struct BurstIndexTask {
    base: TaskBase,
    cfg: Arc<AppState>,
    owner: String,
    album_name: String,
    cache_id: String,
    algorithm: BurstAlgorithm,
    threshold: u32,
    time_window_seconds: u32,
    locale: String,
}

#[async_trait::async_trait]
impl Task for BurstIndexTask {
    fn base(&self) -> &TaskBase {
        &self.base
    }

    fn deduplication_key(&self) -> Option<String> {
        Some(format!(
            "burst-index:{}:{}:{}:{}:{}",
            self.owner,
            self.album_name,
            self.algorithm.as_str(),
            self.threshold,
            self.time_window_seconds
        ))
    }

    async fn cancel(&self) {
        self.base.set_status(TaskStatus::Cancelled).await;
        self.base.set_cur_task("Cancelled".to_string()).await;
    }

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Running).await;
        let root = self.cfg.user_root_path(&self.owner);
        let Some(index) =
            load_existing_album_index(&self.cfg.data_path, &root, &self.owner, &self.album_name)
                .await
                .ok()
                .flatten()
        else {
            self.base
                .set_error_msg("album index not found".to_string())
                .await;
            self.base.set_status(TaskStatus::Failed).await;
            return;
        };
        match build_cache(
            &self.cfg,
            &self.owner,
            &self.album_name,
            &self.cache_id,
            &index,
            self.algorithm,
            self.threshold,
            self.time_window_seconds,
            Some(&self.base),
        )
        .await
        {
            Ok(cache) => {
                if let Err(error) =
                    write_cache(&self.cfg.data_path, &self.owner, &self.cache_id, &cache).await
                {
                    self.base.set_error_msg(error).await;
                    self.base.set_status(TaskStatus::Failed).await;
                    return;
                }
                self.base
                    .set_cur_task(trf(
                        &self.locale,
                        "task.burst_index_done",
                        &[("count", cache.stacks.len().to_string())],
                    ))
                    .await;
                self.base.set_status(TaskStatus::Finished).await;
            }
            Err(error) => {
                self.base.set_error_msg(error).await;
                self.base.set_status(TaskStatus::Failed).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BURST_ALGORITHM, BURST_MAX_HAMMING, build_stacks};
    use crate::backend::album::types::AlbumPhotoItem;
    use std::collections::HashMap;

    fn photo(name: &str, datetime: &str, camera: &str, _phash: u64) -> AlbumPhotoItem {
        AlbumPhotoItem {
            path: format!("albums/trip/{name}"),
            name: name.to_string(),
            date: datetime[..10].to_string(),
            datetime: datetime.to_string(),
            size: 1,
            modified: Some(1),
            camera_make: camera.to_string(),
            camera_model: String::new(),
            lens: String::new(),
            focal_length: String::new(),
            aperture: String::new(),
            shutter_speed: String::new(),
            iso: None,
            width: 100,
            height: 100,
            duration_ms: 0,
            rating: 0,
            color_label: Vec::new(),
            tags: Vec::new(),
        }
    }

    fn hashes(entries: &[AlbumPhotoItem]) -> HashMap<String, u64> {
        entries
            .iter()
            .map(|entry| {
                (
                    entry.path.strip_prefix("albums/trip/").unwrap().to_string(),
                    match entry.name.as_str() {
                        "a.jpg" => 0,
                        "b.jpg" => 1,
                        "c.jpg" => 2,
                        "d.jpg" => u64::MAX,
                        _ => 0,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn adjacent_similar_files_form_one_stack() {
        let entries = vec![
            photo("a.jpg", "2026-04-01 18:31:00", "Panasonic", 0),
            photo("b.jpg", "2026-04-01 18:31:01", "Panasonic", 1),
            photo("c.jpg", "2026-04-01 18:31:02", "Panasonic", 2),
        ];
        let stacks = build_stacks(&entries, "trip", &hashes(&entries), BURST_MAX_HAMMING, 2);
        assert_eq!(stacks.len(), 1);
        assert_eq!(stacks[0].count, 3);
        assert_eq!(stacks[0].members.len(), 3);
        assert_eq!(stacks[0].id.len(), 43);
    }

    #[test]
    fn embedding_algorithm_uses_cosine_threshold() {
        let entries = vec![
            photo("a.jpg", "2026-04-01 18:31:00", "Panasonic", 0),
            photo("b.jpg", "2026-04-01 18:31:01", "Panasonic", 1),
        ];
        let mut vectors = HashMap::new();
        vectors.insert("a.jpg".to_string(), vec![1.0, 0.0]);
        vectors.insert("b.jpg".to_string(), vec![0.98, 0.02]);
        assert_eq!(
            super::build_stacks_embeddings(&entries, "trip", &vectors, 0.90, 2).len(),
            1
        );
        vectors.insert("b.jpg".to_string(), vec![0.0, 1.0]);
        assert!(super::build_stacks_embeddings(&entries, "trip", &vectors, 0.90, 2).is_empty());
    }

    #[test]
    fn threshold_controls_grouping_distance() {
        let entries = vec![
            photo("a.jpg", "2026-04-01 18:31:00", "Panasonic", 0),
            photo("b.jpg", "2026-04-01 18:31:01", "Panasonic", 1),
        ];
        assert!(build_stacks(&entries, "trip", &hashes(&entries), 0, 2).is_empty());
        assert_eq!(
            build_stacks(&entries, "trip", &hashes(&entries), 1, 2).len(),
            1
        );
    }

    #[test]
    fn time_gap_breaks_the_group() {
        let entries = vec![
            photo("a.jpg", "2026-04-01 18:31:00", "Panasonic", 0),
            photo("b.jpg", "2026-04-01 18:31:05", "Panasonic", 1),
        ];
        assert!(build_stacks(&entries, "trip", &hashes(&entries), BURST_MAX_HAMMING, 2).is_empty());
    }

    #[test]
    fn time_window_can_span_the_configured_gap() {
        let entries = vec![
            photo("a.jpg", "2026-04-01 18:31:00", "Panasonic", 0),
            photo("b.jpg", "2026-04-01 18:31:05", "Panasonic", 1),
        ];
        assert!(build_stacks(&entries, "trip", &hashes(&entries), 10, 2).is_empty());
        assert_eq!(
            build_stacks(&entries, "trip", &hashes(&entries), 10, 10).len(),
            1
        );
    }

    #[test]
    fn fingerprint_mismatch_breaks_the_group() {
        let entries = vec![
            photo("a.jpg", "2026-04-01 18:31:00", "Panasonic", 0),
            photo("b.jpg", "2026-04-01 18:31:01", "Panasonic", 1),
            photo("d.jpg", "2026-04-01 18:31:02", "Panasonic", 1 << 20),
        ];
        let stacks = build_stacks(&entries, "trip", &hashes(&entries), BURST_MAX_HAMMING, 2);
        assert_eq!(stacks.len(), 1);
        assert_eq!(stacks[0].members, vec!["a.jpg", "b.jpg"]);
    }

    #[test]
    fn old_cache_without_threshold_uses_default() {
        let raw = r#"{
            "schema_version": 1,
            "algorithm": "burst-v1",
            "source_index": {"entries": 0, "modified_unix_ms": 0, "len": 0},
            "created_at_unix_ms": 0,
            "stacks": []
        }"#;
        let cache: super::BurstCacheFile = serde_json::from_str(raw).unwrap();
        assert_eq!(cache.max_hamming, super::BURST_MAX_HAMMING);
    }

    #[test]
    fn display_items_keep_singles_and_one_stack() {
        let entries = vec![
            photo("a.jpg", "2026-04-01 18:31:00", "Panasonic", 0),
            photo("b.jpg", "2026-04-01 18:31:01", "Panasonic", 1),
            photo("c.jpg", "2026-04-01 18:31:30", "Panasonic", 0),
        ];
        let cache = super::BurstCacheFile {
            schema_version: super::BURST_CACHE_VERSION,
            algorithm: super::BURST_ALGORITHM.to_string(),
            source_index: super::BurstSourceIndex::default(),
            created_at_unix_ms: 0,
            max_hamming: BURST_MAX_HAMMING,
            threshold: BURST_MAX_HAMMING,
            time_window_seconds: 2,
            stacks: vec![super::BurstStack {
                id: "s".to_string(),
                cover: "a.jpg".to_string(),
                count: 2,
                members: vec!["a.jpg".to_string(), "b.jpg".to_string()],
                start_datetime: "2026-04-01 18:31:00".to_string(),
                end_datetime: "2026-04-01 18:31:01".to_string(),
                max_hamming_distance: 1,
            }],
        };
        let index = crate::backend::album::types::AlbumIndex {
            version: 2,
            entries,
        };
        let items = super::display_items(&index, &cache, "trip");
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], super::BurstDisplay::Stack { .. }));
        assert!(matches!(items[1], super::BurstDisplay::Photo(_)));
    }

    #[test]
    fn stack_id_is_stable_and_algorithm_scoped() {
        let entries = vec![
            photo("a.jpg", "2026-04-01 18:31:00", "Panasonic", 0),
            photo("b.jpg", "2026-04-01 18:31:01", "Panasonic", 1),
        ];
        let first = build_stacks(&entries, "trip", &hashes(&entries), BURST_MAX_HAMMING, 2);
        let second = build_stacks(&entries, "trip", &hashes(&entries), BURST_MAX_HAMMING, 2);
        assert_eq!(first[0].id, second[0].id);
        assert_eq!(first[0].id.len(), 43);
        assert_eq!(BURST_ALGORITHM, "burst-v1");
    }
}
