use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiGenerationStats {
    pub(crate) albums_total: usize,
    pub(crate) photos_total: usize,
    pub(crate) photos_indexed: usize,
    #[serde(default)]
    pub(crate) photos_reused: usize,
    pub(crate) duplicates: usize,
    pub(crate) faces: usize,
    #[serde(default)]
    pub(crate) image_embeddings_pending: usize,
    #[serde(default)]
    pub(crate) duplicate_hashes_pending: usize,
    #[serde(default)]
    pub(crate) face_embeddings_pending: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct OwnerIndexStatus {
    pub(crate) owner: String,
    pub(crate) root_exists: bool,
    pub(crate) current_generation: Option<String>,
    pub(crate) manifest_present: bool,
    pub(crate) embeddings_present: bool,
    pub(crate) duplicates_index_present: bool,
    pub(crate) state: Option<String>,
    pub(crate) created_at_unix_ms: Option<u128>,
    pub(crate) sources_total: Option<usize>,
    pub(crate) stats: Option<AiGenerationStats>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct NormalizedAiIndexRequest {
    pub(crate) album: Option<String>,
    pub(crate) mode: String,
    pub(crate) features: Vec<String>,
}

impl NormalizedAiIndexRequest {
    pub(crate) fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|value| value == feature)
    }

    pub(crate) fn default_incremental_features() -> Vec<String> {
        vec![
            "duplicates".to_string(),
            "faces".to_string(),
            "image".to_string(),
        ]
    }

    pub(crate) fn deduplication_key(&self, owner: &str) -> String {
        let album = self.album.as_deref().unwrap_or("all");
        let mut features = self.features.clone();
        features.sort();
        format!(
            "ai-index:{owner}:{album}:{}:{}",
            self.mode,
            features.join("+")
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AiIndexedAlbumSummary {
    pub(crate) owner: String,
    pub(crate) album: String,
    pub(crate) source_key: String,
    /// `AlbumMeta.cache` — AI embedding subdirectory key (stable across album renames).
    #[serde(default)]
    pub(crate) cache_id: String,
    pub(crate) photo_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AiSimilarSearchResult {
    pub(crate) owner: String,
    pub(crate) album: String,
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) date: String,
    pub(crate) size: u64,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) score: f32,
    pub(crate) reason: String,
    /// Album-index metadata, filled on the search response so the client does
    /// not need a second photos request. Empty when the index has no row.
    #[serde(default)]
    pub(crate) datetime: String,
    #[serde(default)]
    pub(crate) camera_make: String,
    #[serde(default)]
    pub(crate) camera_model: String,
    #[serde(default)]
    pub(crate) lens: String,
    #[serde(default)]
    pub(crate) focal_length: String,
    #[serde(default)]
    pub(crate) aperture: String,
    #[serde(default)]
    pub(crate) shutter_speed: String,
    #[serde(default)]
    pub(crate) iso: Option<u32>,
    #[serde(default)]
    pub(crate) rating: u8,
    #[serde(default)]
    pub(crate) color_label: Vec<String>,
    #[serde(default)]
    pub(crate) tags: Vec<String>,
}

impl AiSimilarSearchResult {
    /// Placeholder metadata fields for struct-update construction.
    pub(crate) fn empty_meta() -> Self {
        Self {
            owner: String::new(),
            album: String::new(),
            path: String::new(),
            name: String::new(),
            date: String::new(),
            size: 0,
            width: 0,
            height: 0,
            score: 0.0,
            reason: String::new(),
            datetime: String::new(),
            camera_make: String::new(),
            camera_model: String::new(),
            lens: String::new(),
            focal_length: String::new(),
            aperture: String::new(),
            shutter_speed: String::new(),
            iso: None,
            rating: 0,
            color_label: Vec::new(),
            tags: Vec::new(),
        }
    }
}

pub(crate) struct LoadedOwnerIndex {
    pub(crate) generation: String,
    pub(crate) photos: HashMap<String, AiIndexedPhotoRecord>,
    pub(crate) embeddings: HashMap<String, Vec<f32>>,
    pub(crate) faces: HashMap<String, AiFaceRecord>,
    pub(crate) faces_by_stable_key: HashMap<String, Vec<String>>,
    pub(crate) face_embeddings: HashMap<String, Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AiFaceRecord {
    pub(crate) face_id: String,
    pub(crate) photo_id: usize,
    pub(crate) stable_key: String,
    pub(crate) box_xywh: [f32; 4],
    pub(crate) landmarks: [[f32; 2]; 5],
    pub(crate) detection_score: f32,
    pub(crate) quality_score: f32,
    pub(crate) detector_revision: String,
    #[serde(default = "default_ai_pending_state")]
    pub(crate) embedding_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AiIndexedPhotoRecord {
    pub(crate) photo_id: usize,
    pub(crate) owner: String,
    pub(crate) album: String,
    pub(crate) source_key: String,
    /// `AlbumMeta.cache` — AI embedding subdirectory key (stable across album renames).
    #[serde(default)]
    pub(crate) cache_id: String,
    pub(crate) path: String,
    pub(crate) stable_key: String,
    pub(crate) stable_id: String,
    pub(crate) size_bytes: u64,
    pub(crate) modified_unix_seconds: Option<u64>,
    #[serde(default)]
    pub(crate) content_signature: String,
    #[serde(default)]
    pub(crate) width: Option<u32>,
    #[serde(default)]
    pub(crate) height: Option<u32>,
    #[serde(default)]
    pub(crate) provider_revision: Option<String>,
    #[serde(default = "default_ai_scan_state")]
    pub(crate) scan_state: String,
    #[serde(default = "default_ai_pending_state")]
    pub(crate) image_embedding_state: String,
    #[serde(default = "default_ai_pending_state")]
    pub(crate) duplicate_hash_state: String,
    #[serde(default)]
    pub(crate) duplicate_phash_u64: Option<u64>,
    #[serde(default)]
    pub(crate) duplicate_phash_valid: bool,
    #[serde(default = "default_ai_pending_state")]
    pub(crate) face_embedding_state: String,
    #[serde(default)]
    pub(crate) reused_from_generation: Option<String>,
    #[serde(default)]
    pub(crate) record_error: Option<String>,
    pub(crate) indexed_at_unix_ms: u128,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct StoredAiGenerationManifest {
    #[serde(default)]
    pub(crate) created_at_unix_ms: Option<u128>,
    #[serde(default)]
    pub(crate) state: Option<String>,
    #[serde(default)]
    pub(crate) album: Option<String>,
    #[serde(default)]
    pub(crate) mode: Option<String>,
    #[serde(default)]
    pub(crate) features: Vec<String>,
    #[serde(default)]
    pub(crate) sources: Vec<StoredAiGenerationSource>,
    #[serde(default)]
    pub(crate) stats: Option<AiGenerationStats>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct StoredAiGenerationSource {
    #[allow(dead_code)]
    pub(crate) owner: Option<String>,
    #[allow(dead_code)]
    pub(crate) album: Option<String>,
    #[allow(dead_code)]
    pub(crate) source_key: Option<String>,
    #[allow(dead_code)]
    pub(crate) photo_count: Option<usize>,
}

pub(crate) fn default_ai_scan_state() -> String {
    "discovered".to_string()
}

pub(crate) fn default_ai_pending_state() -> String {
    "pending".to_string()
}

impl AiIndexedPhotoRecord {
    /// Path relative to the album root (forward slashes).
    pub(crate) fn album_relative_path(&self) -> String {
        let prefix = format!("albums/{}/", self.album);
        self.path
            .strip_prefix(&prefix)
            .unwrap_or(self.path.as_str())
            .replace('\\', "/")
    }

    /// Stable embedding map key: `{cache_id}/{album_relative_path}`.
    pub(crate) fn embedding_key(&self) -> String {
        format!("{}/{}", self.cache_id, self.album_relative_path())
    }
}
