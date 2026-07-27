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

/// `/api/face-thumbnail` size bounds (default [`FACE_CROP_SIZE`]).
pub const FACE_THUMBNAIL_SIZE_MIN: u32 = 64;
pub const FACE_THUMBNAIL_SIZE_MAX: u32 = 512;

/// Padding around the face box when cropping avatars (matches frontend `faceCrop.js`).
pub const FACE_AVATAR_PAD: f32 = 0.15;

/// User profile avatar edge length (px).
pub const USER_AVATAR_SIZE: u32 = 256;

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

/// Min cosine similarity to suggest merging an unnamed cluster into a named person.
pub const DEFAULT_PERSON_MATCH_MIN_SCORE: f32 = 0.45;

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
