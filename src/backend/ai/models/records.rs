use serde::{Deserialize, Serialize};

use super::super::provider::AiCapabilities;
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

/// Per-photo AI progress, shared by scan, image embedding, pHash and faces.
///
/// Serialized as the snake_case strings the manifests and JSON used before, so
/// the wire format did not move; the comparisons are variants now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AiItemState {
    /// Scanned, work not decided yet.
    Discovered,
    Pending,
    Ready,
    /// Carried over from the previous generation.
    Reused,
    /// Feature was not requested for this run.
    Skipped,
    Error,
}

impl AiItemState {
    pub(crate) fn is_ready(self) -> bool {
        self == AiItemState::Ready
    }
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
    pub(crate) embedding_state: AiItemState,
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
    pub(crate) scan_state: AiItemState,
    #[serde(default = "default_ai_pending_state")]
    pub(crate) image_embedding_state: AiItemState,
    #[serde(default = "default_ai_pending_state")]
    pub(crate) duplicate_hash_state: AiItemState,
    #[serde(default)]
    pub(crate) duplicate_phash_u64: Option<u64>,
    #[serde(default)]
    pub(crate) duplicate_phash_valid: bool,
    #[serde(default = "default_ai_pending_state")]
    pub(crate) face_embedding_state: AiItemState,
    #[serde(default)]
    pub(crate) reused_from_generation: Option<String>,
    #[serde(default)]
    pub(crate) record_error: Option<String>,
    pub(crate) indexed_at_unix_ms: u128,
}

/// One index generation, written and read through this single type.
///
/// Every field is `#[serde(default)]` so a manifest written by an older build
/// still loads; the writer constructs this struct instead of a `json!` literal,
/// so the read side cannot drift from the write side.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredAiGenerationManifest {
    #[serde(default)]
    pub(crate) version: u32,
    #[serde(default)]
    pub(crate) owner: String,
    #[serde(default)]
    pub(crate) cache_id: String,
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
    pub(crate) files: StoredAiGenerationFiles,
    #[serde(default)]
    pub(crate) provider: StoredAiGenerationProvider,
    #[serde(default)]
    pub(crate) capabilities: Option<AiCapabilities>,
    #[serde(default)]
    pub(crate) stats: Option<AiGenerationStats>,
    #[serde(default)]
    pub(crate) face_detector_revision: String,
    #[serde(default)]
    pub(crate) image_embedding: StoredAiEmbeddingInfo,
    #[serde(default)]
    pub(crate) face_embedding: StoredAiEmbeddingInfo,
    #[serde(default)]
    pub(crate) note: String,
}

impl StoredAiGenerationManifest {
    pub(crate) const VERSION: u32 = 2;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct StoredAiGenerationFiles {
    #[serde(default)]
    pub(crate) embeddings_dir: String,
    #[serde(default)]
    pub(crate) faces_dir: String,
    #[serde(default)]
    pub(crate) phash_dir: String,
    #[serde(default)]
    pub(crate) duplicates_index: String,
    #[serde(default)]
    pub(crate) face_index: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredAiGenerationProvider {
    #[serde(default)]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) backend: String,
    #[serde(default)]
    pub(crate) healthy: bool,
    #[serde(default)]
    pub(crate) model_manifest_present: bool,
    #[serde(default)]
    pub(crate) model_root: String,
    #[serde(default)]
    pub(crate) revision: Option<String>,
}

impl Default for StoredAiGenerationProvider {
    fn default() -> Self {
        Self {
            kind: String::new(),
            backend: String::new(),
            healthy: false,
            model_manifest_present: false,
            model_root: String::new(),
            revision: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct StoredAiEmbeddingInfo {
    #[serde(default)]
    pub(crate) dimension: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

pub(crate) fn default_ai_scan_state() -> AiItemState {
    AiItemState::Discovered
}

pub(crate) fn default_ai_pending_state() -> AiItemState {
    AiItemState::Pending
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

#[cfg(test)]
mod state_tests {
    use super::AiItemState;

    /// The manifests and JSON payloads stored these as bare strings; the typed
    /// enum must not change what they contain.
    #[test]
    fn states_keep_their_wire_strings() {
        for (state, text) in [
            (AiItemState::Discovered, "\"discovered\""),
            (AiItemState::Pending, "\"pending\""),
            (AiItemState::Ready, "\"ready\""),
            (AiItemState::Reused, "\"reused\""),
            (AiItemState::Skipped, "\"skipped\""),
            (AiItemState::Error, "\"error\""),
        ] {
            assert_eq!(serde_json::to_string(&state).unwrap(), text);
            assert_eq!(serde_json::from_str::<AiItemState>(text).unwrap(), state);
        }
    }

    #[test]
    fn ready_is_the_only_ready_state() {
        assert!(AiItemState::Ready.is_ready());
        for state in [
            AiItemState::Discovered,
            AiItemState::Pending,
            AiItemState::Reused,
            AiItemState::Skipped,
            AiItemState::Error,
        ] {
            assert!(!state.is_ready(), "{state:?}");
        }
    }
}

#[cfg(test)]
mod manifest_tests {
    use super::{
        AiGenerationStats, StoredAiEmbeddingInfo, StoredAiGenerationFiles,
        StoredAiGenerationManifest, StoredAiGenerationProvider, StoredAiGenerationSource,
    };

    fn sample() -> StoredAiGenerationManifest {
        StoredAiGenerationManifest {
            version: StoredAiGenerationManifest::VERSION,
            owner: "daniel".to_string(),
            cache_id: "trip-1".to_string(),
            created_at_unix_ms: Some(1_700_000_000_000),
            state: Some("ready".to_string()),
            album: Some("Trip".to_string()),
            mode: Some("incremental".to_string()),
            features: vec!["image".to_string(), "faces".to_string()],
            sources: vec![StoredAiGenerationSource {
                owner: Some("daniel".to_string()),
                album: Some("Trip".to_string()),
                source_key: Some("trip".to_string()),
                photo_count: Some(12),
            }],
            files: StoredAiGenerationFiles {
                embeddings_dir: "embeddings".to_string(),
                faces_dir: "faces".to_string(),
                phash_dir: "phash".to_string(),
                duplicates_index: "duplicates_index.json".to_string(),
                face_index: "face_index.json".to_string(),
            },
            provider: StoredAiGenerationProvider {
                kind: "local".to_string(),
                backend: "burn".to_string(),
                healthy: true,
                model_manifest_present: true,
                model_root: "models".to_string(),
                revision: Some("rev-1".to_string()),
            },
            capabilities: None,
            stats: Some(AiGenerationStats {
                albums_total: 1,
                photos_total: 12,
                photos_indexed: 12,
                photos_reused: 0,
                duplicates: 2,
                faces: 5,
                image_embeddings_pending: 0,
                duplicate_hashes_pending: 0,
                face_embeddings_pending: 0,
            }),
            face_detector_revision: "det-1".to_string(),
            image_embedding: StoredAiEmbeddingInfo {
                dimension: Some(768),
            },
            face_embedding: StoredAiEmbeddingInfo {
                dimension: Some(512),
            },
            note: "note".to_string(),
        }
    }

    #[test]
    fn manifest_wire_shape_is_unchanged() {
        let value = serde_json::to_value(sample()).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "album",
                "cache_id",
                "capabilities",
                "created_at_unix_ms",
                "face_detector_revision",
                "face_embedding",
                "features",
                "files",
                "image_embedding",
                "mode",
                "note",
                "owner",
                "provider",
                "sources",
                "state",
                "stats",
                "version",
            ]
        );
    }

    #[test]
    fn manifest_reads_back_through_the_same_type() {
        let original = sample();
        let raw = serde_json::to_string(&original).unwrap();
        let loaded: StoredAiGenerationManifest = serde_json::from_str(&raw).unwrap();
        assert_eq!(loaded.version, 2);
        assert_eq!(loaded.owner, "daniel");
        assert_eq!(loaded.state.as_deref(), Some("ready"));
        assert_eq!(
            loaded.stats.as_ref().map(|stats| stats.photos_total),
            Some(12)
        );
        assert_eq!(
            loaded.provider.revision.as_deref(),
            Some("rev-1"),
            "provider snapshot must survive a round trip"
        );
        assert_eq!(loaded.sources.len(), 1);
        assert_eq!(loaded.image_embedding.dimension, Some(768));
    }

    #[test]
    fn an_older_manifest_still_loads() {
        // Only `state` and `stats` existed in the first iteration of the format.
        let raw = r#"{"state":"partial","stats":{"albums_total":1,"photos_total":2,"photos_indexed":1,"duplicates":0,"faces":0}}"#;
        let loaded: StoredAiGenerationManifest = serde_json::from_str(raw).unwrap();
        assert_eq!(loaded.state.as_deref(), Some("partial"));
        assert_eq!(loaded.version, 0);
        assert!(loaded.files.face_index.is_empty());
        assert!(loaded.capabilities.is_none());
    }
}
