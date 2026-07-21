//! Album domain: indexes, photos, thumbnails, darktable, sharing, and filters.

pub mod api;
pub mod decoder;
pub mod filters;
pub mod helpers;
pub mod media;
pub mod types;

pub use decoder::{
    create_decoder, find_xmp_sidecar, DecodeError, DecodeOptions, DecoderCreateOptions,
    ImageDecoder, ImageFormat, ImageFormatSet, is_raw_path,
};

pub use api::*;
pub use media::{
    api_albums_export, api_darktable_preview, api_face_thumbnail, api_has_xmp, api_thumbnail,
    is_image_path, queue_background_thumbnail_tasks, spawn_thumbnail_prewarmer, thumbnail_cache_key,
    ThumbnailGenerator,
};
pub use types::*;

pub(crate) use api::{collect_accessible_ai_scan_targets, AiAlbumScanTarget};
pub(crate) use helpers::{merge_xmp_meta, migrate_json_sidecar, scan_json_sidecars};
pub(crate) use media::collect_album_image_paths;
