//! Album domain: indexes, photos, thumbnails, darktable, sharing, and filters.

pub mod api;
pub(crate) mod bursts;
pub mod decoder;
pub mod download;
pub mod filters;
pub mod helpers;
pub mod media;
pub mod types;
pub mod video_encode;
pub(crate) mod workflow;

pub use api::*;
pub(crate) use api::{AiAlbumScanTarget, collect_accessible_ai_scan_targets};
pub use download::{
    api_albums_download, api_albums_download_file, api_albums_export_batch,
    api_albums_export_batch_file,
};
pub(crate) use helpers::{migrate_json_sidecar, scan_json_sidecars};
pub use media::{
    api_albums_export, api_darktable_preview, api_has_xmp, api_thumbnail,
    queue_background_thumbnail_tasks, spawn_thumbnail_prewarmer,
};
pub(crate) use media::{
    collect_album_image_paths, ensure_cached_face_crop, purge_owner_thumbnail_cache,
    write_cached_face_crop_from_image,
};
pub use video_encode::{api_video_encode, api_video_file, api_video_info};
