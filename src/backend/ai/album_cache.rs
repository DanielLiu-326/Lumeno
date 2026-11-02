//! Per-album AI cache: one JSON file per photo embedding / face set / phash, plus indexes.
//!
//! Layout under `cache/ai/{owner}/albums/{cache_id}/`:
//! - `embeddings/{key}.json` (primary image vectors for search + index reuse)
//! - `faces/{key}.json`
//! - `face-crops/{face_id}.webp` (`face_id` = hash(stable_key|rounded_box))
//! - `phash/{key}.json` (per-photo DCT pHash; written at index time)
//! - `duplicates_index.json` (near-duplicate cliques; written at index time)
//! - `face_index.json` (DBSCAN suggestions; written once after face indexing)
//! - `face_hint.json` (named-person merge hints; refreshed on index / faces.json edits)
//! - `clusters.json` (legacy alias: read as a fallback, never written)

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha512};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::embedding::cosine_similarity;
use super::people::{ClusterFace, cluster_faces};
use crate::backend::album::helpers::is_safe_cache_key;

pub use crate::backend::constants::{
    DEFAULT_CLUSTER_EPS, DEFAULT_CLUSTER_MIN_SAMPLES, DEFAULT_NEAR_DUPLICATE_DISTANCE,
    DEFAULT_PERSON_MATCH_MIN_SCORE, MAX_PERSON_MATCH_SUGGESTIONS,
};

pub const EMBEDDING_CACHE_VERSION: u32 = 1;
pub const FACE_CACHE_VERSION: u32 = 2;
pub const CLUSTERS_CACHE_VERSION: u32 = 3;
pub const FACE_HINTS_VERSION: u32 = 1;
pub const PHASH_CACHE_VERSION: u32 = 1;
pub const DUPLICATES_INDEX_VERSION: u32 = 1;

fn b64_engine() -> &'static base64::engine::GeneralPurpose {
    &base64::engine::general_purpose::STANDARD
}

/// SHA512 → base64url (no pad), matching thumbnail-style keys.
pub fn content_cache_key(parts: &[&str]) -> String {
    let mut hasher = Sha512::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            hasher.update(b"|");
        }
        hasher.update(part.as_bytes());
    }
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize())
}

pub fn embedding_cache_key(
    stable_key: &str,
    content_signature: &str,
    provider_revision: &str,
) -> String {
    content_cache_key(&[stable_key, content_signature, provider_revision])
}

pub fn face_photo_cache_key(
    stable_key: &str,
    content_signature: &str,
    detector_revision: &str,
) -> String {
    content_cache_key(&[stable_key, content_signature, detector_revision])
}

pub fn phash_cache_key(stable_key: &str, content_signature: &str) -> String {
    content_cache_key(&[stable_key, content_signature, "dct-phash-v1"])
}

/// Canonical box string for cache keys, quantized to 1e-3 normalized coordinates.
pub fn format_box_xywh(box_xywh: &[f32; 4]) -> String {
    format!(
        "{:.3},{:.3},{:.3},{:.3}",
        box_xywh[0], box_xywh[1], box_xywh[2], box_xywh[3]
    )
}

/// Deterministic face id: `hash(stable_key|rounded_box)`.
pub fn face_id(stable_key: &str, box_xywh: &[f32; 4]) -> String {
    content_cache_key(&[stable_key, &format_box_xywh(box_xywh)])
}

/// Named-person appearance key in `faces.json`: `hash(source|box)`.
pub fn appearance_identity_key(source: &str, box_xywh: &[f32; 4]) -> String {
    let source = source
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string();
    content_cache_key(&[&source, &format_box_xywh(box_xywh)])
}

pub fn encode_f32_vector_b64(vector: &[f32]) -> String {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    b64_engine().encode(bytes)
}

pub fn decode_f32_vector_b64(
    encoded: &str,
    expected_dim: Option<usize>,
) -> Result<Vec<f32>, String> {
    let bytes = b64_engine()
        .decode(encoded.trim())
        .map_err(|error| format!("decode vector base64: {error}"))?;
    if bytes.len() % 4 != 0 {
        return Err(format!(
            "vector byte length {} is not a multiple of 4",
            bytes.len()
        ));
    }
    let dim = bytes.len() / 4;
    if let Some(expected) = expected_dim
        && dim != expected
    {
        return Err(format!("vector dim {dim} != expected {expected}"));
    }
    let mut out = Vec::with_capacity(dim);
    for chunk in bytes.chunks_exact(4) {
        let arr: [u8; 4] = chunk.try_into().unwrap();
        out.push(f32::from_le_bytes(arr));
    }
    Ok(out)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingCacheFile {
    pub version: u32,
    pub stable_key: String,
    pub source: String,
    pub dimension: usize,
    pub dtype: String,
    pub vector_b64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

impl EmbeddingCacheFile {
    pub fn from_vector(
        stable_key: &str,
        source: &str,
        vector: &[f32],
        provider_revision: Option<&str>,
        content_signature: Option<&str>,
    ) -> Self {
        Self {
            version: EMBEDDING_CACHE_VERSION,
            stable_key: stable_key.to_string(),
            source: source.to_string(),
            dimension: vector.len(),
            dtype: "f32".to_string(),
            vector_b64: encode_f32_vector_b64(vector),
            provider_revision: provider_revision.map(str::to_string),
            content_signature: content_signature.map(str::to_string),
            width: None,
            height: None,
        }
    }

    pub fn decode_vector(&self) -> Result<Vec<f32>, String> {
        decode_f32_vector_b64(&self.vector_b64, Some(self.dimension))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceCacheEntry {
    pub face_id: String,
    pub box_xywh: [f32; 4],
    #[serde(default)]
    pub landmarks: [[f32; 2]; 5],
    #[serde(default)]
    pub detection_score: f32,
    #[serde(default)]
    pub quality_score: f32,
    pub dimension: usize,
    pub embedding_b64: String,
}

impl FaceCacheEntry {
    pub fn decode_embedding(&self) -> Result<Vec<f32>, String> {
        decode_f32_vector_b64(&self.embedding_b64, Some(self.dimension))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacePhotoCacheFile {
    pub version: u32,
    pub source: String,
    pub stable_key: String,
    /// Informational; validity is encoded in the filename via `face_photo_cache_key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_signature: Option<String>,
    /// Informational; validity is encoded in the filename via `face_photo_cache_key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detector_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    pub faces: Vec<FaceCacheEntry>,
}

impl FacePhotoCacheFile {
    pub fn new(
        source: &str,
        stable_key: &str,
        content_signature: &str,
        detector_revision: &str,
        faces: Vec<FaceCacheEntry>,
    ) -> Self {
        Self {
            version: FACE_CACHE_VERSION,
            source: source.to_string(),
            stable_key: stable_key.to_string(),
            content_signature: Some(content_signature.to_string()),
            detector_revision: Some(detector_revision.to_string()),
            width: None,
            height: None,
            faces,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PersonMatchSuggestion {
    pub person_id: String,
    pub person_name: String,
    pub score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NamedPersonCenter {
    pub person_id: String,
    pub person_name: String,
    pub center_id: String,
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterSuggestion {
    pub cluster_id: String,
    pub face_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representative_face_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center_b64: Option<String>,
    /// Named-person merge hints (computed vs `faces.json` centers).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub match_suggestions: Vec<PersonMatchSuggestion>,
}

/// Lightweight face row stored in `face_index.json` (no embedding blob).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceIndexEntry {
    pub face_id: String,
    pub stable_key: String,
    pub source: String,
    pub box_xywh: [f32; 4],
    #[serde(default)]
    pub quality_score: f32,
    /// Stem of `faces/{photo_key}.json` for embedding lookup on adopt/merge.
    #[serde(default)]
    pub photo_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClustersCacheFile {
    pub version: u32,
    pub algorithm: String,
    pub params: ClusterParams,
    pub clusters: Vec<ClusterSuggestion>,
    #[serde(default)]
    pub noise_face_ids: Vec<String>,
    /// Catalog of unassigned faces referenced by clusters / noise (v2+).
    #[serde(default)]
    pub faces: Vec<FaceIndexEntry>,
    /// Per noise face: merge hints into named people (`noise-{face_id}` groups).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub noise_match_suggestions: HashMap<String, Vec<PersonMatchSuggestion>>,
}

/// Named-person merge hints keyed by suggestion `cluster_id` (`c-N` or `noise-{face_id}`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FaceHintsFile {
    pub version: u32,
    #[serde(default)]
    pub group_hints: HashMap<String, Vec<PersonMatchSuggestion>>,
}

impl FaceHintsFile {
    pub fn empty() -> Self {
        Self {
            version: FACE_HINTS_VERSION,
            group_hints: HashMap::new(),
        }
    }
}

/// Extract merge hints from an in-memory clusters snapshot (after match computation).
pub fn hints_from_clusters(clusters: &ClustersCacheFile) -> FaceHintsFile {
    let mut group_hints = HashMap::new();
    for cluster in &clusters.clusters {
        if !cluster.match_suggestions.is_empty() {
            group_hints.insert(
                cluster.cluster_id.clone(),
                cluster.match_suggestions.clone(),
            );
        }
    }
    for (face_id, suggestions) in &clusters.noise_match_suggestions {
        if !suggestions.is_empty() {
            group_hints.insert(format!("noise-{face_id}"), suggestions.clone());
        }
    }
    FaceHintsFile {
        version: FACE_HINTS_VERSION,
        group_hints,
    }
}

/// Strip ephemeral hint fields before persisting `face_index.json`.
pub fn clusters_without_hints(clusters: &ClustersCacheFile) -> ClustersCacheFile {
    let mut out = clusters.clone();
    for cluster in &mut out.clusters {
        cluster.match_suggestions.clear();
    }
    out.noise_match_suggestions.clear();
    out
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterParams {
    pub eps: f32,
    pub min_samples: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhashCacheFile {
    pub version: u32,
    pub stable_key: String,
    pub source: String,
    pub phash_u64: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_signature: Option<String>,
}

impl PhashCacheFile {
    pub fn new(
        stable_key: &str,
        source: &str,
        phash_u64: u64,
        content_signature: Option<&str>,
    ) -> Self {
        Self {
            version: PHASH_CACHE_VERSION,
            stable_key: stable_key.to_string(),
            source: source.to_string(),
            phash_u64,
            content_signature: content_signature.map(str::to_string),
        }
    }
}

pub struct AlbumAiCache {
    pub(crate) root: PathBuf,
}

impl AlbumAiCache {
    pub fn new(ai_root: impl Into<PathBuf>, owner: &str, cache_id: &str) -> Self {
        Self {
            root: ai_root.into().join(owner).join("albums").join(cache_id),
        }
    }

    pub fn embeddings_dir(&self) -> PathBuf {
        self.root.join("embeddings")
    }

    pub fn faces_dir(&self) -> PathBuf {
        self.root.join("faces")
    }

    pub fn face_crops_dir(&self) -> PathBuf {
        self.root.join("face-crops")
    }

    pub fn phash_dir(&self) -> PathBuf {
        self.root.join("phash")
    }

    /// `None` when `face_id` is not a cache key. Face ids arrive from the client,
    /// so joining them unchecked would allow escaping `face-crops/`.
    pub fn face_crop_path_by_id(&self, face_id: &str) -> Option<PathBuf> {
        if !is_safe_cache_key(face_id) {
            return None;
        }
        Some(self.face_crops_dir().join(format!("{face_id}.webp")))
    }

    /// Cache path for named-person appearances (`faces.json`), keyed by source|box hash.
    pub fn face_crop_path_by_appearance(&self, source: &str, box_xywh: &[f32; 4]) -> PathBuf {
        let key = appearance_identity_key(source, box_xywh);
        self.face_crops_dir().join(format!("{key}.webp"))
    }

    pub async fn find_face_in_index_async(&self, face_id: &str) -> Option<FaceIndexEntry> {
        self.read_face_index_async()
            .await?
            .faces
            .into_iter()
            .find(|entry| entry.face_id == face_id)
    }

    pub fn phash_path(&self, key: &str) -> PathBuf {
        self.phash_dir().join(format!("{key}.json"))
    }

    pub fn duplicates_index_path(&self) -> PathBuf {
        self.root.join("duplicates_index.json")
    }

    pub fn face_index_path(&self) -> PathBuf {
        self.root.join("face_index.json")
    }

    pub fn face_hints_path(&self) -> PathBuf {
        self.root.join("face_hint.json")
    }

    pub fn clusters_path(&self) -> PathBuf {
        self.root.join("clusters.json")
    }

    pub fn embedding_path(&self, key: &str) -> PathBuf {
        self.embeddings_dir().join(format!("{key}.json"))
    }

    pub fn face_photo_path(&self, key: &str) -> PathBuf {
        self.faces_dir().join(format!("{key}.json"))
    }

    pub async fn ensure_dirs_async(&self) -> Result<(), String> {
        for dir in [
            self.embeddings_dir(),
            self.faces_dir(),
            self.face_crops_dir(),
            self.phash_dir(),
        ] {
            tokio::fs::create_dir_all(&dir)
                .await
                .map_err(|e| format!("create dir {}: {e}", dir.display()))?;
        }
        Ok(())
    }

    pub async fn write_embedding_async(
        &self,
        key: &str,
        file: &EmbeddingCacheFile,
    ) -> Result<(), String> {
        self.ensure_dirs_async().await?;
        let path = self.embedding_path(key);
        let bytes = serde_json::to_vec_pretty(file)
            .map_err(|e| format!("serialize embedding cache: {e}"))?;
        atomic_write_async(&path, &bytes).await
    }

    pub async fn read_embedding_async(&self, key: &str) -> Option<EmbeddingCacheFile> {
        let path = self.embedding_path(key);
        let raw = tokio::fs::read_to_string(path).await.ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Load all per-photo image embedding cache files under `embeddings/`.
    pub async fn list_embeddings_async(&self) -> Result<Vec<(String, EmbeddingCacheFile)>, String> {
        list_json_dir_async(&self.embeddings_dir()).await
    }

    pub async fn write_face_photo_async(
        &self,
        key: &str,
        file: &FacePhotoCacheFile,
    ) -> Result<(), String> {
        self.ensure_dirs_async().await?;
        let path = self.face_photo_path(key);
        let bytes =
            serde_json::to_vec_pretty(file).map_err(|e| format!("serialize face cache: {e}"))?;
        atomic_write_async(&path, &bytes).await
    }

    /// Validity is the filename key; existence alone means the entry matches that key.
    pub async fn has_face_photo_async(&self, key: &str) -> bool {
        tokio::fs::try_exists(self.face_photo_path(key))
            .await
            .unwrap_or(false)
    }

    pub async fn read_face_photo_async(&self, key: &str) -> Option<FacePhotoCacheFile> {
        let path = self.face_photo_path(key);
        let raw = tokio::fs::read_to_string(path).await.ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub async fn write_phash_async(&self, key: &str, file: &PhashCacheFile) -> Result<(), String> {
        self.ensure_dirs_async().await?;
        let path = self.phash_path(key);
        let bytes =
            serde_json::to_vec_pretty(file).map_err(|e| format!("serialize phash cache: {e}"))?;
        atomic_write_async(&path, &bytes).await
    }

    pub async fn read_phash_async(&self, key: &str) -> Option<PhashCacheFile> {
        let path = self.phash_path(key);
        let raw = tokio::fs::read_to_string(path).await.ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub async fn write_duplicates_index_json_async(
        &self,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|e| format!("create album ai root: {e}"))?;
        let bytes = serde_json::to_vec_pretty(value)
            .map_err(|e| format!("serialize duplicates_index: {e}"))?;
        atomic_write_async(&self.duplicates_index_path(), &bytes).await
    }

    pub fn read_duplicates_index_json(&self) -> Option<serde_json::Value> {
        let path = self.duplicates_index_path();
        std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
    }

    pub async fn read_duplicates_index_json_async(&self) -> Option<serde_json::Value> {
        let path = self.duplicates_index_path();
        let raw = tokio::fs::read_to_string(path).await.ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// Load all face photo cache files under `faces/`.
    pub async fn list_face_photos_async(
        &self,
    ) -> Result<Vec<(String, FacePhotoCacheFile)>, String> {
        list_json_dir_async(&self.faces_dir()).await
    }

    /// Persist DBSCAN suggestion clusters after face indexing completes.
    ///
    /// Only `face_index.json` is written; the legacy `clusters.json` name is read
    /// as a fallback for caches written by older versions, never produced.
    pub async fn write_face_index_async(&self, file: &ClustersCacheFile) -> Result<(), String> {
        let file = clusters_without_hints(file);
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|e| format!("create album ai root: {e}"))?;
        let bytes =
            serde_json::to_vec_pretty(&file).map_err(|e| format!("serialize face_index: {e}"))?;
        atomic_write_async(&self.face_index_path(), &bytes).await
    }

    pub async fn read_face_hints_async(&self) -> Option<FaceHintsFile> {
        let path = self.face_hints_path();
        let raw = tokio::fs::read_to_string(&path).await.ok()?;
        serde_json::from_str(&raw).ok()
    }

    pub async fn write_face_hints_async(&self, file: &FaceHintsFile) -> Result<(), String> {
        tokio::fs::create_dir_all(&self.root)
            .await
            .map_err(|e| format!("create album ai root: {e}"))?;
        let bytes =
            serde_json::to_vec_pretty(file).map_err(|e| format!("serialize face_hint: {e}"))?;
        atomic_write_async(&self.face_hints_path(), &bytes).await
    }

    /// Read precomputed DBSCAN suggestions. Prefers `face_index.json`, falls back to
    /// legacy `clusters.json`.
    pub async fn read_face_index_async(&self) -> Option<ClustersCacheFile> {
        for path in [self.face_index_path(), self.clusters_path()] {
            if let Ok(raw) = tokio::fs::read_to_string(&path).await
                && let Ok(file) = serde_json::from_str::<ClustersCacheFile>(&raw)
            {
                return Some(file);
            }
        }
        None
    }
}

async fn atomic_write_async(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("no parent for {}", path.display()))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| format!("create parent dir: {e}"))?;
    let tmp = parent.join(format!(
        ".{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("cache")
    ));
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|e| format!("write tmp {}: {e}", tmp.display()))?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|e| format!("rename to {}: {e}", path.display()))?;
    Ok(())
}

async fn list_json_dir_async<T: serde::de::DeserializeOwned>(
    dir: &Path,
) -> Result<Vec<(String, T)>, String> {
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(_) => return Ok(Vec::new()),
    };
    let mut out = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let key = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let modified = entry
            .metadata()
            .await
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .unwrap_or(UNIX_EPOCH);
        if let Ok(raw) = tokio::fs::read_to_string(&path).await
            && let Ok(file) = serde_json::from_str::<T>(&raw)
        {
            out.push((modified, key, file));
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    Ok(out.into_iter().map(|(_, key, file)| (key, file)).collect())
}

/// Build cluster suggestions from unassigned face embeddings.
///
/// `face_metas` should cover the same face ids as `faces` (source/box/photo_key for GET).
pub fn build_clusters_cache(
    faces: &[ClusterFace],
    face_metas: &[FaceIndexEntry],
    eps: f32,
    min_samples: usize,
) -> ClustersCacheFile {
    let assignments = cluster_faces(faces, eps, min_samples);
    let emb_by_id: HashMap<&str, &Vec<f32>> = faces
        .iter()
        .map(|f| (f.face_id.as_str(), &f.embedding))
        .collect();

    let mut by_person: HashMap<String, Vec<String>> = HashMap::new();
    for (face_id, person_id) in &assignments {
        by_person
            .entry(person_id.clone())
            .or_default()
            .push(face_id.clone());
    }

    let mut clusters = Vec::new();
    let mut cluster_index = 0usize;
    let mut keys: Vec<_> = by_person.keys().cloned().collect();
    keys.sort();
    for person_id in keys {
        let mut face_ids = by_person.remove(&person_id).unwrap_or_default();
        face_ids.sort();
        let representative = pick_representative(&face_ids, &emb_by_id);
        let center_b64 = representative
            .as_ref()
            .and_then(|id| emb_by_id.get(id.as_str()).map(|v| encode_f32_vector_b64(v)));
        clusters.push(ClusterSuggestion {
            cluster_id: format!("c-{cluster_index}"),
            face_ids,
            representative_face_id: representative,
            center_b64,
            match_suggestions: Vec::new(),
        });
        cluster_index += 1;
    }

    let assigned: std::collections::HashSet<_> = assignments.keys().cloned().collect();
    let mut noise_face_ids: Vec<_> = faces
        .iter()
        .filter(|f| !assigned.contains(&f.face_id))
        .map(|f| f.face_id.clone())
        .collect();
    noise_face_ids.sort();

    let face_ids: std::collections::HashSet<&str> =
        faces.iter().map(|f| f.face_id.as_str()).collect();
    let mut catalog: Vec<FaceIndexEntry> = face_metas
        .iter()
        .filter(|m| face_ids.contains(m.face_id.as_str()))
        .cloned()
        .collect();
    catalog.sort_by(|a, b| a.face_id.cmp(&b.face_id));

    ClustersCacheFile {
        version: CLUSTERS_CACHE_VERSION,
        algorithm: "dbscan".to_string(),
        params: ClusterParams { eps, min_samples },
        clusters,
        noise_face_ids,
        faces: catalog,
        noise_match_suggestions: HashMap::new(),
    }
}

pub fn build_clusters_cache_default(
    faces: &[ClusterFace],
    face_metas: &[FaceIndexEntry],
) -> ClustersCacheFile {
    build_clusters_cache_with_matches(faces, face_metas, &[])
}

pub fn build_clusters_cache_with_matches(
    faces: &[ClusterFace],
    face_metas: &[FaceIndexEntry],
    named_catalog: &[NamedPersonCenter],
) -> ClustersCacheFile {
    build_clusters_cache_with_matches_params(
        faces,
        face_metas,
        DEFAULT_CLUSTER_EPS,
        DEFAULT_CLUSTER_MIN_SAMPLES,
        named_catalog,
    )
}

pub fn build_clusters_cache_with_matches_params(
    faces: &[ClusterFace],
    face_metas: &[FaceIndexEntry],
    eps: f32,
    min_samples: usize,
    named_catalog: &[NamedPersonCenter],
) -> ClustersCacheFile {
    let mut file = build_clusters_cache(faces, face_metas, eps, min_samples);
    if named_catalog.is_empty() {
        return file;
    }
    let emb_by_id: HashMap<&str, &Vec<f32>> = faces
        .iter()
        .map(|face| (face.face_id.as_str(), &face.embedding))
        .collect();
    apply_person_match_suggestions(
        &mut file,
        named_catalog,
        |face_id| emb_by_id.get(face_id).map(|vector| (*vector).clone()),
        DEFAULT_PERSON_MATCH_MIN_SCORE,
        MAX_PERSON_MATCH_SUGGESTIONS,
    );
    file
}

/// Max cosine similarity between a query vector and any of a person's centers.
pub fn max_similarity_to_centers(query: &[f32], centers: &[Vec<f32>]) -> f32 {
    centers
        .iter()
        .map(|center| cosine_similarity(query, center))
        .fold(f32::NEG_INFINITY, f32::max)
}

pub fn suggest_person_matches(
    query: &[f32],
    catalog: &[NamedPersonCenter],
    min_score: f32,
    limit: usize,
) -> Vec<PersonMatchSuggestion> {
    if query.is_empty() || catalog.is_empty() || limit == 0 {
        return Vec::new();
    }
    let mut best_by_person: HashMap<String, PersonMatchSuggestion> = HashMap::new();
    for entry in catalog {
        let score = cosine_similarity(query, &entry.embedding);
        if score < min_score {
            continue;
        }
        best_by_person
            .entry(entry.person_id.clone())
            .and_modify(|existing| {
                if score > existing.score {
                    existing.score = score;
                    existing.center_id = Some(entry.center_id.clone());
                }
            })
            .or_insert(PersonMatchSuggestion {
                person_id: entry.person_id.clone(),
                person_name: entry.person_name.clone(),
                score,
                center_id: Some(entry.center_id.clone()),
            });
    }
    let mut out: Vec<_> = best_by_person.into_values().collect();
    out.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.person_name.cmp(&right.person_name))
    });
    out.truncate(limit);
    out
}

fn cluster_query_embedding(
    cluster: &ClusterSuggestion,
    resolve_embedding: &impl Fn(&str) -> Option<Vec<f32>>,
) -> Option<Vec<f32>> {
    if let Some(center_b64) = cluster.center_b64.as_deref() {
        if let Ok(vector) = decode_f32_vector_b64(center_b64, None) {
            if !vector.is_empty() {
                return Some(vector);
            }
        }
    }
    cluster
        .representative_face_id
        .as_deref()
        .and_then(resolve_embedding)
        .or_else(|| {
            cluster
                .face_ids
                .first()
                .and_then(|id| resolve_embedding(id.as_str()))
        })
}

pub fn apply_person_match_suggestions(
    file: &mut ClustersCacheFile,
    catalog: &[NamedPersonCenter],
    resolve_embedding: impl Fn(&str) -> Option<Vec<f32>>,
    min_score: f32,
    limit: usize,
) {
    if catalog.is_empty() {
        file.noise_match_suggestions.clear();
        for cluster in &mut file.clusters {
            cluster.match_suggestions.clear();
        }
        return;
    }
    for cluster in &mut file.clusters {
        cluster.match_suggestions = cluster_query_embedding(cluster, &resolve_embedding)
            .map(|query| suggest_person_matches(&query, catalog, min_score, limit))
            .unwrap_or_default();
    }
    file.noise_match_suggestions.clear();
    for noise_id in &file.noise_face_ids {
        let Some(query) = resolve_embedding(noise_id.as_str()) else {
            continue;
        };
        let suggestions = suggest_person_matches(&query, catalog, min_score, limit);
        if !suggestions.is_empty() {
            file.noise_match_suggestions
                .insert(noise_id.clone(), suggestions);
        }
    }
}

pub fn best_match_score(group: &Value) -> f32 {
    group
        .get("match_suggestions")
        .and_then(|value| value.as_array())
        .and_then(|items| items.first())
        .and_then(|item| item.get("score"))
        .and_then(|value| value.as_f64())
        .map(|value| value as f32)
        .unwrap_or(0.0)
}

pub fn sort_suggestion_groups(groups: &mut [Value]) {
    groups.sort_by(|left, right| {
        let left_has = best_match_score(left) > 0.0;
        let right_has = best_match_score(right) > 0.0;
        right_has
            .cmp(&left_has)
            .then_with(|| {
                best_match_score(right)
                    .partial_cmp(&best_match_score(left))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| {
                left.get("cluster_id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .cmp(
                        right
                            .get("cluster_id")
                            .and_then(|value| value.as_str())
                            .unwrap_or_default(),
                    )
            })
    });
}

impl ClustersCacheFile {
    pub fn empty() -> Self {
        Self {
            version: CLUSTERS_CACHE_VERSION,
            algorithm: "dbscan".to_string(),
            params: ClusterParams {
                eps: DEFAULT_CLUSTER_EPS,
                min_samples: DEFAULT_CLUSTER_MIN_SAMPLES,
            },
            clusters: Vec::new(),
            noise_face_ids: Vec::new(),
            faces: Vec::new(),
            noise_match_suggestions: HashMap::new(),
        }
    }
}

fn pick_representative(
    face_ids: &[String],
    emb_by_id: &HashMap<&str, &Vec<f32>>,
) -> Option<String> {
    if face_ids.is_empty() {
        return None;
    }
    if face_ids.len() == 1 {
        return face_ids.first().cloned();
    }
    // Medoid: face with highest average cosine similarity to others.
    let mut best_id = face_ids[0].clone();
    let mut best_score = f32::NEG_INFINITY;
    for candidate in face_ids {
        let Some(cand) = emb_by_id.get(candidate.as_str()) else {
            continue;
        };
        let mut sum = 0_f32;
        let mut count = 0_usize;
        for other in face_ids {
            if other == candidate {
                continue;
            }
            if let Some(o) = emb_by_id.get(other.as_str()) {
                sum += cosine_similarity(cand, o);
                count += 1;
            }
        }
        let score = if count == 0 { 0.0 } else { sum / count as f32 };
        if score > best_score {
            best_score = score;
            best_id = candidate.clone();
        }
    }
    Some(best_id)
}

#[cfg(test)]
mod tests {
    #[test]
    fn face_crop_paths_refuse_traversal_ids() {
        let cache = AlbumAiCache::new(std::path::Path::new("/data/cache/ai"), "alice", "album");
        for bad in [
            "",
            ".",
            "..",
            "../../../bob/albums/x/face-crops/abc",
            "abc.webp",
            "a/b",
            "a\\b",
            &"z".repeat(85),
            &"z".repeat(87),
        ] {
            assert!(
                cache.face_crop_path_by_id(bad).is_none(),
                "accepted unsafe face id {bad:?}"
            );
        }
        let good = "A1b2C3d4E5".to_string() + &"x".repeat(76);
        let path = cache.face_crop_path_by_id(&good).expect("valid cache key");
        assert!(path.starts_with(cache.face_crops_dir()));
        assert!(path.ends_with(format!("{good}.webp")));
    }

    use super::*;
    use uuid::Uuid;

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("album-ai-cache-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn vector_b64_roundtrip() {
        let vector = vec![0.1_f32, -0.25, 1.5, 0.0];
        let encoded = encode_f32_vector_b64(&vector);
        let decoded = decode_f32_vector_b64(&encoded, Some(4)).unwrap();
        assert_eq!(decoded, vector);
    }

    #[test]
    fn cache_key_stable() {
        let a = embedding_cache_key("raw/a.jpg", "sig1", "rev1");
        let b = embedding_cache_key("raw/a.jpg", "sig1", "rev1");
        let c = embedding_cache_key("raw/a.jpg", "sig2", "rev1");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[tokio::test]
    async fn embedding_file_roundtrip() {
        let dir = temp_dir();
        let cache = AlbumAiCache::new(&dir, "daniel", "raw");
        let file = EmbeddingCacheFile::from_vector(
            "raw/a.jpg",
            "a.jpg",
            &[1.0, 0.0, 0.0],
            Some("rev"),
            Some("sig"),
        );
        let key = embedding_cache_key("raw/a.jpg", "sig", "rev");
        cache.write_embedding_async(&key, &file).await.unwrap();
        let loaded = cache.read_embedding_async(&key).await.unwrap();
        assert_eq!(loaded.decode_vector().unwrap(), vec![1.0, 0.0, 0.0]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn face_index_reads_legacy_clusters_file_only_as_fallback() {
        let dir = temp_dir();
        let cache = AlbumAiCache::new(&dir, "daniel", "raw");
        let mut legacy = ClustersCacheFile::empty();
        legacy.algorithm = "legacy-dbscan".to_string();
        cache.ensure_dirs_async().await.unwrap();
        // A cache written by an older version: only `clusters.json` exists.
        std::fs::write(cache.clusters_path(), serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(!cache.face_index_path().exists());
        let loaded = cache
            .read_face_index_async()
            .await
            .expect("legacy clusters.json is still readable");
        assert_eq!(loaded.algorithm, "legacy-dbscan");

        // Once the canonical file exists it wins; nothing mirrors back.
        let canonical = ClustersCacheFile::empty();
        cache.write_face_index_async(&canonical).await.unwrap();
        assert!(cache.face_index_path().is_file());
        let loaded = cache.read_face_index_async().await.unwrap();
        assert_eq!(loaded.algorithm, "dbscan");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn face_photo_and_clusters_roundtrip() {
        let dir = temp_dir();
        let cache = AlbumAiCache::new(&dir, "daniel", "raw");
        let key = face_photo_cache_key("raw/a.jpg", "sig", "det");
        let face = FaceCacheEntry {
            face_id: "f1".to_string(),
            box_xywh: [0.1, 0.2, 0.3, 0.4],
            landmarks: [[0.0; 2]; 5],
            detection_score: 0.9,
            quality_score: 0.8,
            dimension: 2,
            embedding_b64: encode_f32_vector_b64(&[1.0, 0.0]),
        };
        let photo = FacePhotoCacheFile::new("a.jpg", "raw/a.jpg", "sig", "det", vec![face]);
        cache.write_face_photo_async(&key, &photo).await.unwrap();
        assert_eq!(
            cache.read_face_photo_async(&key).await.unwrap().faces.len(),
            1
        );

        let faces = vec![
            ClusterFace {
                face_id: "a".to_string(),
                embedding: unit(1.0, 0.0),
            },
            ClusterFace {
                face_id: "b".to_string(),
                embedding: unit(0.99, 0.01),
            },
            ClusterFace {
                face_id: "c".to_string(),
                embedding: unit(0.0, 1.0),
            },
        ];
        let clusters = build_clusters_cache(&faces, &[], 0.2, 2);
        cache.write_face_index_async(&clusters).await.unwrap();
        assert!(cache.face_index_path().is_file());
        let loaded = cache.read_face_index_async().await.unwrap();
        assert!(!loaded.clusters.is_empty());
        assert!(
            loaded
                .clusters
                .iter()
                .any(|c| c.face_ids.contains(&"a".to_string()))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn suggest_person_matches_picks_best_center_per_person() {
        let catalog = vec![
            NamedPersonCenter {
                person_id: "p1".to_string(),
                person_name: "Alice".to_string(),
                center_id: "c1".to_string(),
                embedding: unit(1.0, 0.0),
            },
            NamedPersonCenter {
                person_id: "p1".to_string(),
                person_name: "Alice".to_string(),
                center_id: "c2".to_string(),
                embedding: unit(0.0, 1.0),
            },
            NamedPersonCenter {
                person_id: "p2".to_string(),
                person_name: "Bob".to_string(),
                center_id: "c3".to_string(),
                embedding: unit(0.95, 0.05),
            },
        ];
        let query = unit(0.98, 0.02);
        let suggestions = suggest_person_matches(&query, &catalog, 0.4, 3);
        assert_eq!(suggestions.len(), 2);
        assert_eq!(suggestions[0].person_id, "p2");
        assert!(suggestions[0].score > 0.9);
    }

    #[test]
    fn max_sim_uses_best_center() {
        let query = unit(1.0, 0.0);
        let centers = vec![unit(0.0, 1.0), unit(0.95, 0.05)];
        let score = max_similarity_to_centers(&query, &centers);
        assert!(score > 0.9);
    }

    fn unit(a: f32, b: f32) -> Vec<f32> {
        let n = (a * a + b * b).sqrt();
        vec![a / n, b / n]
    }
}
