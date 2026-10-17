//! Shared tunables used across album media, AI pipelines, and APIs.
//!
//! Prefer this module for cross-cutting sizes, limits, and costs. Keep
//! model-specific revisions / ONNX input shapes next to the model code.

// ── Thumbnails / previews / avatars ──────────────────────────────────────────

/// Album grid / background prewarm thumbnail edge length (px).
pub const THUMBNAIL_SIZE: u32 = 360;

/// Preview warm / mid-size thumbs — same as [`THUMBNAIL_SIZE`].
/// Large enough for AI scan pHash (needs only 32×32).
pub const PREVIEW_THUMBNAIL_SIZE: u32 = THUMBNAIL_SIZE;

/// `/api/thumbnail` size bounds when `size` is set and non-zero (`0` = full res).
pub const THUMBNAIL_SIZE_MIN: u32 = 64;
pub const THUMBNAIL_SIZE_MAX: u32 = 1024;

/// AI face-crop edge length written at detection save time (`face-crops/…webp`).
pub const FACE_CROP_SIZE: u32 = 256;

/// Padding around the face box when cropping avatars (matches frontend `faceCrop.js`).
pub const FACE_AVATAR_PAD: f32 = 0.15;

/// User profile avatar edge length (px).
pub const USER_AVATAR_SIZE: u32 = 256;

/// Long-edge cap for user wallpaper images (px).
pub const USER_WALLPAPER_MAX_EDGE: u32 = 2560;

/// Default / clamp for album grid and file-list page size (`photos_per_page` setting).
pub const DEFAULT_PHOTOS_PER_PAGE: u32 = 100;
pub const PHOTOS_PER_PAGE_MIN: u32 = 10;
pub const PHOTOS_PER_PAGE_MAX: u32 = 500;

/// Darktable RAW preview default / clamp (`size=0` = full res).
pub const DARKTABLE_PREVIEW_DEFAULT_SIZE: u32 = 1600;
pub const DARKTABLE_PREVIEW_SIZE_MIN: u32 = 256;
pub const DARKTABLE_PREVIEW_SIZE_MAX: u32 = 4096;

// ── Faces / people / duplicates ──────────────────────────────────────────────

/// Max faces kept per photo after detection (quality-ranked).
pub const MAX_FACES_PER_PHOTO: usize = 8;

/// Max face previews embedded per suggestion cluster in GET `/faces`.
/// Prefer not embedding previews; clients load crops by `face_id` lazily.
#[allow(dead_code)]
pub const FACE_SUGGESTION_PREVIEW_LIMIT: usize = 16;

/// DBSCAN cosine-distance eps for face clustering suggestions.
pub const DEFAULT_CLUSTER_EPS: f32 = 0.38;
/// DBSCAN min_samples for face clustering suggestions.
pub const DEFAULT_CLUSTER_MIN_SAMPLES: usize = 2;

/// Album-wise neighbor similarity for face clustering (`1 - eps`).
/// Higher means two faces must be more alike to join the same group.
pub const DEFAULT_CLUSTER_SIMILARITY: f32 = 1.0 - DEFAULT_CLUSTER_EPS;
pub const CLUSTER_SIMILARITY_MIN: f32 = 0.45;
pub const CLUSTER_SIMILARITY_MAX: f32 = 0.90;

/// Clamp a user-facing clustering similarity into the supported range.
pub fn clamp_cluster_similarity(value: f32) -> f32 {
    if !value.is_finite() {
        return DEFAULT_CLUSTER_SIMILARITY;
    }
    value.clamp(CLUSTER_SIMILARITY_MIN, CLUSTER_SIMILARITY_MAX)
}

/// Cosine distance eps used by DBSCAN for a similarity threshold.
pub fn cluster_eps_from_similarity(similarity: f32) -> f32 {
    1.0 - clamp_cluster_similarity(similarity)
}

/// Min cosine similarity to suggest merging an unnamed cluster into a named person.
pub const DEFAULT_PERSON_MATCH_MIN_SCORE: f32 = 0.45;

/// Min IoU between old and new face boxes when reconciling `faces.json` after re-detect.
pub const RECONCILE_FACE_IOU_MIN: f32 = 0.3;

/// Max named-person merge suggestions attached to one cluster / noise face.
pub const MAX_PERSON_MATCH_SUGGESTIONS: usize = 3;

/// Max Unicode scalars allowed in a person display name.
pub const MAX_PERSON_NAME_LEN: usize = 120;

/// Default Hamming distance for near-duplicate photo grouping.
pub const DEFAULT_NEAR_DUPLICATE_DISTANCE: u32 = 8;

/// Max Unicode scalars allowed in an AI text-search query.
pub const MAX_AI_TEXT_QUERY_CHARS: usize = 512;

// ── AI index task progress costs ─────────────────────────────────────────────

pub const AI_SCAN_COST: u64 = 1;
pub const AI_EMBED_COST: u64 = 4;
pub const AI_FACE_COST: u64 = 5;
pub const AI_WRITE_COST: u64 = 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_similarity_matches_historical_eps() {
        let eps = cluster_eps_from_similarity(DEFAULT_CLUSTER_SIMILARITY);
        assert!((eps - DEFAULT_CLUSTER_EPS).abs() < 1e-6);
    }

    #[test]
    fn similarity_clamps_before_converting_to_distance() {
        assert!((cluster_eps_from_similarity(0.99) - (1.0 - CLUSTER_SIMILARITY_MAX)).abs() < 1e-6);
        assert!((cluster_eps_from_similarity(0.10) - (1.0 - CLUSTER_SIMILARITY_MIN)).abs() < 1e-6);
        assert!((clamp_cluster_similarity(f32::NAN) - DEFAULT_CLUSTER_SIMILARITY).abs() < 1e-6);
    }
}
