//! Album domain: indexes, photos, thumbnails, darktable, sharing, and filters.

pub mod api;
pub mod decoder;
pub mod filters;
pub mod helpers;
pub mod media;
pub mod types;

pub use api::*;
pub(crate) use api::{AiAlbumScanTarget, collect_accessible_ai_scan_targets};
pub(crate) use helpers::{migrate_json_sidecar, scan_json_sidecars};
pub use media::{
    api_albums_export, api_darktable_preview, api_has_xmp, api_thumbnail,
    queue_background_thumbnail_tasks, spawn_thumbnail_prewarmer,
};
pub(crate) use media::{
    collect_album_image_paths, ensure_cached_face_crop, purge_owner_thumbnail_cache,
    write_cached_face_crop_from_image,
};
