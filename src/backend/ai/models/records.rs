use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::super::people::{FaceAssignment, PersonRecord};

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
    pub(crate) photos_meta_present: bool,
    pub(crate) hashes_bin_present: bool,
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

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AiIndexedAlbumSummary {
    pub(crate) owner: String,
    pub(crate) album: String,
    pub(crate) source_key: String,
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
}

pub(crate) struct LoadedOwnerIndex {
    pub(crate) generation: String,
    pub(crate) photos: HashMap<String, AiIndexedPhotoRecord>,
    pub(crate) embeddings: HashMap<String, Vec<f32>>,
    pub(crate) faces: HashMap<String, AiFaceRecord>,
    pub(crate) faces_by_stable_key: HashMap<String, Vec<String>>,
    pub(crate) face_embeddings: HashMap<String, Vec<f32>>,
    pub(crate) people: HashMap<String, PersonRecord>,
    pub(crate) assignments: HashMap<String, FaceAssignment>,
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
    #[serde(default)]
    pub(crate) embedding_offset: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AiIndexedPhotoRecord {
    pub(crate) photo_id: usize,
    pub(crate) owner: String,
    pub(crate) album: String,
    pub(crate) source_key: String,
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
    #[serde(default)]
    pub(crate) image_embedding_offset: Option<u64>,
    #[serde(default = "default_ai_pending_state")]
    pub(crate) duplicate_hash_state: String,
    #[serde(default)]
    pub(crate) duplicate_sha256_hex: Option<String>,
    #[serde(default)]
    pub(crate) duplicate_sha256_offset: Option<u64>,
    #[serde(default)]
    pub(crate) duplicate_phash_u64: Option<u64>,
    #[serde(default)]
    pub(crate) duplicate_phash_offset: Option<u64>,
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

