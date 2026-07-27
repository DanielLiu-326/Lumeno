use async_recursion::async_recursion;
use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{Path, Query};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::{Extension, Json};
use base64::Engine;
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Utc};
use function_name::named;
use futures::future::BoxFuture;
use image::GenericImageView;
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};
use std::collections::{HashMap, HashSet};
use std::io::BufReader;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::u8;
use tokio::sync::Notify;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::backend::common::{
    AppError, AppState, ConcurrencyLimiter, Token, create_response, into_host_path_safely,
    move_file_or_dir,
};
use crate::backend::constants::{
    DARKTABLE_PREVIEW_DEFAULT_SIZE, DARKTABLE_PREVIEW_SIZE_MAX, DARKTABLE_PREVIEW_SIZE_MIN,
    FACE_AVATAR_PAD, FACE_CROP_SIZE, FACE_THUMBNAIL_SIZE_MAX, FACE_THUMBNAIL_SIZE_MIN,
    PREVIEW_THUMBNAIL_SIZE, THUMBNAIL_SIZE, THUMBNAIL_SIZE_MAX, THUMBNAIL_SIZE_MIN,
};
use crate::backend::i18n::{RequestLocale, tr, trf};
use crate::backend::tasks::{
    Task, TaskBase, TaskStatus, format_subtask, task_file_name, wait_while_paused,
};

use super::decoder::{create_decoder, darktable_export, DecodeOptions, DecoderCreateOptions};
use super::helpers::{
    albums_root, ensure_exif_cached, is_user_visible_album_dir, read_album_meta,
    relative_display_path,
};
use super::types::*;

#[derive(Debug, Deserialize)]
pub struct ThumbnailQuery {
    size: Option<u32>,
    owner: Option<String>,
    /// Thumbnail generator: `lumeno` (default) or `darktable`.
    generator: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct FaceThumbnailQuery {
    /// Normalized face box `x,y,w,h` in `[0, 1]`.
    r#box: String,
    size: Option<u32>,
    owner: Option<String>,
}

/// How a thumbnail is produced. Encoded into the cache key so lumeno and
/// darktable outputs never collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThumbnailGenerator {
    /// App-side decode: raster via `image`, RAW via embedded JPEG (exiftool).
    #[default]
    Lumeno,
    /// `darktable-cli` demosaic / grade (RAW only; non-RAW falls back to lumeno).
    Darktable,
}

impl ThumbnailGenerator {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lumeno => "lumeno",
            Self::Darktable => "darktable",
        }
    }

    pub fn parse(value: Option<&str>) -> Option<Self> {
        match value.map(str::trim).filter(|s| !s.is_empty()) {
            None => Some(Self::Lumeno),
            Some(s) if s.eq_ignore_ascii_case("lumeno") => Some(Self::Lumeno),
            Some(s) if s.eq_ignore_ascii_case("darktable") => Some(Self::Darktable),
            Some(_) => None,
        }
    }
}

pub fn is_raw_path(path: &std::path::Path) -> bool {
    super::decoder::is_raw_path(path)
}

pub fn is_image_path(path: &std::path::Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "heic" | "heif"
            | "hif"
    ) || is_raw_path(path)
}

pub fn thumbnail_cache_key(
    username: &str,
    source: &std::path::Path,
    size: u32,
    metadata: &std::fs::Metadata,
    generator: ThumbnailGenerator,
) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hasher = Sha512::new();
    hasher.update(username.as_bytes());
    hasher.update(b"|");
    hasher.update(source.to_string_lossy().as_bytes());
    hasher.update(b"|");
    hasher.update(size.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(metadata.len().to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(modified.to_string().as_bytes());
    hasher.update(b"|v3|");
    hasher.update(generator.as_str().as_bytes());
    let digest = hasher.finalize();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Pre-generator cache key (`|v2`). Kept so existing thumbnails are reused after the v3 bump.
fn thumbnail_cache_key_v2(
    username: &str,
    source: &std::path::Path,
    size: u32,
    metadata: &std::fs::Metadata,
) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hasher = Sha512::new();
    hasher.update(username.as_bytes());
    hasher.update(b"|");
    hasher.update(source.to_string_lossy().as_bytes());
    hasher.update(b"|");
    hasher.update(size.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(metadata.len().to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(modified.to_string().as_bytes());
    hasher.update(b"|v2");
    let digest = hasher.finalize();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Resolve thumbnail cache path for the given generator. If a pre-v3 (`|v2`) file
/// exists and the new path does not, adopt it (rename/copy) so thumbs are not regenerated.
pub(crate) fn thumbnail_cache_file(
    data_path: &std::path::Path,
    username: &str,
    source: &std::path::Path,
    size: u32,
    metadata: &std::fs::Metadata,
    generator: ThumbnailGenerator,
) -> PathBuf {
    let dir = data_path.join("cache").join("thumbnails");
    let primary = dir.join(format!(
        "{}.webp",
        thumbnail_cache_key(username, source, size, metadata, generator)
    ));
    if primary.exists() {
        return primary;
    }

    let legacy = dir.join(format!(
        "{}.webp",
        thumbnail_cache_key_v2(username, source, size, metadata)
    ));
    if !legacy.exists() {
        return primary;
    }

    if let Some(parent) = primary.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::rename(&legacy, &primary) {
        Ok(()) => primary,
        Err(_) => {
            if std::fs::copy(&legacy, &primary).is_ok() {
                let _ = std::fs::remove_file(&legacy);
                primary
            } else {
                // Last resort: keep serving the legacy file without regenerating.
                legacy
            }
        }
    }
}

/// Cache key for EXIF metadata, keyed on file path, size, and mtime.
/// Separate from `thumbnail_cache_key` since EXIF has no user or size dimension.
pub(crate) fn exif_cache_key(source: &std::path::Path, metadata: &std::fs::Metadata) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hasher = Sha512::new();
    hasher.update(b"exif|");
    hasher.update(source.to_string_lossy().as_bytes());
    hasher.update(b"|");
    hasher.update(metadata.len().to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(modified.to_string().as_bytes());
    hasher.update(b"|v2");
    let digest = hasher.finalize();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

pub(crate) async fn ensure_thumbnail_cached(
    source: PathBuf,
    output: PathBuf,
    size: u32,
    generator: ThumbnailGenerator,
    thumbnail_limiter: &Arc<ConcurrencyLimiter>,
    darktable_limiter: &Arc<ConcurrencyLimiter>,
) -> Result<(), AppError> {
    if output.exists() {
        return Ok(());
    }

    let use_darktable = generator == ThumbnailGenerator::Darktable && is_raw_path(&source);
    let limiter = if use_darktable {
        darktable_limiter
    } else {
        thumbnail_limiter
    };
    let permit = limiter.acquire().await;
    if output.exists() {
        return Ok(());
    }

    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Unknown(format!("create thumbnail cache dir failed: {e}")))?;
    }

    let create_opts = if use_darktable {
        DecoderCreateOptions::darktable()
    } else {
        DecoderCreateOptions::auto()
    };
    let decode_opts = DecodeOptions::new().with_max_edge(size);
    let source_for_decode = source.clone();
    let output_blocking = output.clone();
    let result = async {
        let decoder = create_decoder(create_opts).map_err(|error| error.message)?;
        let image = decoder
            .decode_path(&source_for_decode, decode_opts)
            .await
            .map_err(|error| error.message)?;
        tokio::task::spawn_blocking(move || {
            image
                .save_with_format(&output_blocking, image::ImageFormat::WebP)
                .map_err(|e| format!("save webp thumbnail failed: {e}"))
        })
        .await
        .map_err(|e| format!("thumbnail save join failed: {e}"))?
    }
    .await;
    drop(permit);
    result.map_err(AppError::Unknown)
}

fn load_oriented_image_from_bytes(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let decoder = create_decoder(DecoderCreateOptions::auto()).map_err(|error| error.message)?;
    decoder
        .decode_bytes(bytes, DecodeOptions::new())
        .map_err(|error| error.message)
}

fn write_webp_thumbnail_from_bytes(
    bytes: &[u8],
    output: &std::path::Path,
    size: u32,
) -> Result<(), String> {
    let decoder = create_decoder(DecoderCreateOptions::auto()).map_err(|error| error.message)?;
    let img = decoder
        .decode_bytes(bytes, DecodeOptions::new().with_max_edge(size))
        .map_err(|error| error.message)?;
    img.save_with_format(output, image::ImageFormat::WebP)
        .map_err(|e| format!("save webp thumbnail failed: {e}"))?;
    Ok(())
}

fn parse_face_box_query(raw: &str, locale: &str) -> Result<[f32; 4], AppError> {
    let mut values = [0_f32; 4];
    for (index, part) in raw.split(',').take(4).enumerate() {
        values[index] = part.trim().parse::<f32>().map_err(|_| {
            AppError::BadRequest(tr(
                locale,
                "error.invalid_face_box",
            ))
        })?;
    }
    if raw.split(',').count() != 4 {
        return Err(AppError::BadRequest(tr(
            locale,
            "error.invalid_face_box",
        )));
    }
    let [x, y, w, h] = values;
    if w <= 0.0 || h <= 0.0 || x < 0.0 || y < 0.0 || x + w > 1.01 || y + h > 1.01 {
        return Err(AppError::BadRequest(tr(
            locale,
            "error.invalid_face_box",
        )));
    }
    Ok([
        x.clamp(0.0, 1.0),
        y.clamp(0.0, 1.0),
        w.clamp(0.0, 1.0),
        h.clamp(0.0, 1.0),
    ])
}

pub(crate) fn crop_face_avatar(image: &image::DynamicImage, box_xywh: [f32; 4]) -> Result<image::DynamicImage, String> {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err("empty image".to_string());
    }
    let [x, y, w, h] = box_xywh;
    let left = (x - w * FACE_AVATAR_PAD).clamp(0.0, 1.0);
    let top = (y - h * FACE_AVATAR_PAD).clamp(0.0, 1.0);
    let crop_w = (w * (1.0 + FACE_AVATAR_PAD * 2.0)).min(1.0 - left);
    let crop_h = (h * (1.0 + FACE_AVATAR_PAD * 2.0)).min(1.0 - top);
    if crop_w <= 0.01 || crop_h <= 0.01 {
        return Err("invalid face crop".to_string());
    }

    let left_px = (left * width as f32).floor() as u32;
    let top_px = (top * height as f32).floor() as u32;
    let crop_w_px = (crop_w * width as f32).ceil() as u32;
    let crop_h_px = (crop_h * height as f32).ceil() as u32;
    let crop_w_px = crop_w_px.min(width.saturating_sub(left_px)).max(1);
    let crop_h_px = crop_h_px.min(height.saturating_sub(top_px)).max(1);

    Ok(image.crop_imm(left_px, top_px, crop_w_px, crop_h_px))
}

fn face_thumbnail_cache_key(
    username: &str,
    source: &std::path::Path,
    size: u32,
    metadata: &std::fs::Metadata,
    box_xywh: [f32; 4],
) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hasher = Sha512::new();
    hasher.update(b"face|");
    hasher.update(username.as_bytes());
    hasher.update(b"|");
    hasher.update(source.to_string_lossy().as_bytes());
    hasher.update(b"|");
    hasher.update(size.to_string().as_bytes());
    hasher.update(b"|");
    for value in box_xywh {
        hasher.update(format!("{value:.4}").as_bytes());
        hasher.update(b",");
    }
    hasher.update(b"|");
    hasher.update(metadata.len().to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(modified.to_string().as_bytes());
    hasher.update(b"|v1");
    let digest = hasher.finalize();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

fn face_thumbnail_cache_file(
    data_path: &std::path::Path,
    username: &str,
    source: &std::path::Path,
    size: u32,
    metadata: &std::fs::Metadata,
    box_xywh: [f32; 4],
) -> PathBuf {
    data_path
        .join("cache")
        .join("face-thumbnails")
        .join(format!(
            "{}.webp",
            face_thumbnail_cache_key(username, source, size, metadata, box_xywh)
        ))
}

pub(crate) fn write_face_webp_thumbnail_from_image(
    image: &image::DynamicImage,
    output: &std::path::Path,
    size: u32,
    box_xywh: [f32; 4],
) -> Result<(), String> {
    let cropped = crop_face_avatar(image, box_xywh)?;
    let thumb = cropped.thumbnail(size, size);
    thumb
        .save_with_format(output, image::ImageFormat::WebP)
        .map_err(|e| format!("save face webp thumbnail failed: {e}"))?;
    Ok(())
}

fn is_readable_face_crop_sync(path: &FsPath) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() || meta.len() == 0 {
        return false;
    }
    std::fs::File::open(path).is_ok()
}

pub(crate) async fn is_readable_face_crop(path: &FsPath) -> bool {
    let Ok(meta) = tokio::fs::metadata(path).await else {
        return false;
    };
    if !meta.is_file() || meta.len() == 0 {
        return false;
    }
    tokio::fs::File::open(path).await.is_ok()
}

async fn clear_unusable_face_crop(path: &FsPath) {
    if tokio::fs::try_exists(path).await.unwrap_or(false)
        && !is_readable_face_crop(path).await
    {
        let _ = tokio::fs::remove_file(path).await;
    }
}

/// Write a face crop under AI cache from an already-decoded image (miss only).
pub(crate) fn write_cached_face_crop_from_image(
    image: &image::DynamicImage,
    crop_output: &FsPath,
    size: u32,
    box_xywh: [f32; 4],
) -> Result<(), String> {
    if is_readable_face_crop_sync(crop_output) {
        return Ok(());
    }
    if crop_output.exists() {
        let _ = std::fs::remove_file(crop_output);
    }
    if let Some(parent) = crop_output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create face-crops dir: {e}"))?;
    }
    write_face_webp_thumbnail_from_image(image, crop_output, size, box_xywh)
}

/// Ensure `face-crops/{face_id}.webp` exists and is readable.
///
/// Cache hits bypass the limiter. On miss (or unreadable entry): acquire
/// [`thumbnail_limiter`] (or [`darktable_limiter`] for RAW), decode, and write at `size`.
pub(crate) async fn ensure_cached_face_crop(
    album_dir: &FsPath,
    crop_output: &FsPath,
    source_relative: &str,
    box_xywh: [f32; 4],
    size: u32,
    thumbnail_limiter: &Arc<ConcurrencyLimiter>,
    darktable_limiter: &Arc<ConcurrencyLimiter>,
) -> Result<(), String> {
    if is_readable_face_crop(crop_output).await {
        return Ok(());
    }

    let source = album_dir.join(source_relative.replace('\\', "/").trim_start_matches('/'));
    if !source.is_file() {
        return Err(format!("source missing: {}", source.display()));
    }

    let limiter = if is_raw_path(&source) {
        darktable_limiter
    } else {
        thumbnail_limiter
    };
    let permit = limiter.acquire().await;
    if is_readable_face_crop(crop_output).await {
        return Ok(());
    }
    clear_unusable_face_crop(crop_output).await;
    if let Some(parent) = crop_output.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create face-crops dir: {e}"))?;
    }
    let source_for_decode = source.clone();
    let output_blocking = crop_output.to_path_buf();
    let result = async {
        let decoder = create_decoder(DecoderCreateOptions::auto()).map_err(|e| e.message)?;
        let image = decoder
            .decode_path(&source_for_decode, DecodeOptions::new())
            .await
            .map_err(|e| e.message)?;
        tokio::task::spawn_blocking(move || {
            write_face_webp_thumbnail_from_image(&image, &output_blocking, size, box_xywh)
        })
        .await
        .map_err(|e| format!("face crop join: {e}"))??;
        if !is_readable_face_crop_sync(&crop_output) {
            return Err(format!(
                "face crop not readable after write: {}",
                crop_output.display()
            ));
        }
        Ok(())
    }
    .await;
    drop(permit);
    result
}

fn write_face_webp_thumbnail_from_bytes(
    bytes: &[u8],
    output: &std::path::Path,
    size: u32,
    box_xywh: [f32; 4],
) -> Result<(), String> {
    let img = load_oriented_image_from_bytes(bytes)?;
    write_face_webp_thumbnail_from_image(&img, output, size, box_xywh)
}

async fn ensure_face_thumbnail_cached(
    source: PathBuf,
    output: PathBuf,
    size: u32,
    box_xywh: [f32; 4],
    thumbnail_limiter: &Arc<ConcurrencyLimiter>,
) -> Result<(), AppError> {
    if output.exists() {
        return Ok(());
    }

    let permit = thumbnail_limiter.acquire().await;
    if output.exists() {
        return Ok(());
    }

    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Unknown(format!("create face thumbnail cache dir failed: {e}")))?;
    }

    let source_for_decode = source.clone();
    let output_blocking = output.clone();
    let result = async {
        let decoder = create_decoder(DecoderCreateOptions::auto()).map_err(|error| error.message)?;
        let image = decoder
            .decode_path(&source_for_decode, DecodeOptions::new())
            .await
            .map_err(|error| error.message)?;
        tokio::task::spawn_blocking(move || {
            write_face_webp_thumbnail_from_image(&image, &output_blocking, size, box_xywh)
        })
        .await
        .map_err(|e| format!("face thumbnail save join failed: {e}"))?
    }
    .await;
    drop(permit);
    result.map_err(AppError::Unknown)
}

pub(crate) fn preview_thumbnail_cache_path(
    data_path: &std::path::Path,
    username: &str,
    source: &std::path::Path,
    metadata: &std::fs::Metadata,
) -> PathBuf {
    thumbnail_cache_file(
        data_path,
        username,
        source,
        PREVIEW_THUMBNAIL_SIZE,
        metadata,
        ThumbnailGenerator::Lumeno,
    )
}

pub(crate) async fn generate_thumbnail_batch(
    base: &TaskBase,
    cfg: &AppState,
    thumbnails: Vec<(PathBuf, PathBuf)>,
    concurrency: usize,
    cancel: &AtomicBool,
    paused: &AtomicBool,
) -> Vec<String> {
    use futures::StreamExt;

    futures::stream::iter(thumbnails)
        .map(|(source, output)| async move {
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return None;
                }
                if !paused.load(Ordering::Relaxed) {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            let done = base.processed_count.load(Ordering::Relaxed);
            let total = base.total_count.load(Ordering::Relaxed).max(1);
            base.set_cur_task(format_subtask(
                "thumbnail",
                &task_file_name(source.as_path()),
                done + 1,
                total,
            ))
            .await;
            let error = ensure_thumbnail_cached(
                source.clone(),
                output,
                THUMBNAIL_SIZE,
                ThumbnailGenerator::Lumeno,
                &cfg.thumbnail_limiter,
                &cfg.darktable_limiter,
            )
            .await
            .err()
            .map(|error| format!("{}: {error}", source.display()));
            base.add_processed_count(1);
            Some(error)
        })
        .buffer_unordered(concurrency.max(1))
        .filter_map(|item| async move { item })
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect()
}

pub(crate) struct BackgroundThumbnailTask {
    base: TaskBase,
    cfg: Arc<AppState>,
    cancel: AtomicBool,
    paused: AtomicBool,
}

impl BackgroundThumbnailTask {
    fn new(name: String, username: String, cfg: Arc<AppState>) -> Self {
        Self {
            base: TaskBase::new(name, username),
            cfg,
            cancel: AtomicBool::new(false),
            paused: AtomicBool::new(false),
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    async fn wait_if_paused(&self) -> bool {
        wait_while_paused(&self.cancel, &self.paused).await
    }
}

#[async_trait]
impl Task for BackgroundThumbnailTask {
    fn base(&self) -> &TaskBase {
        &self.base
    }

    fn deduplication_key(&self) -> Option<String> {
        Some(format!("background-thumbnails:{}", self.base.owner))
    }

    fn cancellable(&self) -> bool {
        true
    }

    fn pausable(&self) -> bool {
        true
    }

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Running).await;
        self.base.set_cur_task("Scanning albums".to_string()).await;

        let user_root = self.cfg.user_root_path(&self.base.owner);
        let albums_dir = albums_root(&user_root);
        // Background thumbs are album-only. File-browser trees outside `albums/`
        // stay on-demand via `/api/thumbnail`.
        let mut sources = Vec::new();
        if let Ok(mut dir) = tokio::fs::read_dir(&albums_dir).await {
            while let Ok(Some(entry)) = dir.next_entry().await {
                if self.is_cancelled() {
                    break;
                }
                let Ok(meta) = entry.metadata().await else {
                    continue;
                };
                if !meta.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if !is_user_visible_album_dir(&name) {
                    continue;
                }
                collect_album_images(entry.path().as_path(), &mut sources).await;
            }
        }
        if self.is_cancelled() {
            self.base.set_cur_task("Cancelled".to_string()).await;
            self.base.set_status(TaskStatus::Cancelled).await;
            return;
        }

        let manifest_path = self
            .cfg
            .data_path
            .join("cache")
            .join("thumbnail-manifests")
            .join(format!("{}.json", self.base.owner));
        let old_manifest = tokio::fs::read_to_string(&manifest_path)
            .await
            .ok()
            .and_then(|raw| serde_json::from_str::<HashMap<String, String>>(&raw).ok())
            .unwrap_or_default();
        let mut current_manifest = HashMap::new();
        let mut thumbnails = Vec::new();
        for source in sources {
            if self.is_cancelled() {
                self.base.set_cur_task("Cancelled".to_string()).await;
                self.base.set_status(TaskStatus::Cancelled).await;
                return;
            }
            let Ok(metadata) = tokio::fs::metadata(&source).await else {
                continue;
            };
            // Defence: never manage thumbs for paths outside this user's albums/.
            if !source.starts_with(&albums_dir) {
                continue;
            }
            let cache_key = thumbnail_cache_key(
                &self.base.owner,
                &source,
                THUMBNAIL_SIZE,
                &metadata,
                ThumbnailGenerator::Lumeno,
            );
            // Adopt pre-v3 cache files so background warm does not regenerate everything.
            let output = thumbnail_cache_file(
                self.cfg.data_path.as_path(),
                &self.base.owner,
                &source,
                THUMBNAIL_SIZE,
                &metadata,
                ThumbnailGenerator::Lumeno,
            );
            current_manifest.insert(cache_key, source.to_string_lossy().into_owned());
            thumbnails.push((source, output));
        }
        let stale_keys = old_manifest
            .keys()
            .filter(|key| !current_manifest.contains_key(*key))
            .cloned()
            .collect::<Vec<_>>();
        self.base
            .set_total_count(thumbnails.len() + stale_keys.len());

        if self.wait_if_paused().await {
            self.base.set_cur_task("Cancelled".to_string()).await;
            self.base.set_status(TaskStatus::Cancelled).await;
            return;
        }

        // Default background thumbs always use lumeno (embedded JPEG for RAW).
        let mut errors = generate_thumbnail_batch(
            &self.base,
            &self.cfg,
            thumbnails,
            self.cfg.thumbnail_limiter.limit(),
            &self.cancel,
            &self.paused,
        )
        .await;

        if self.is_cancelled() {
            if let Some(parent) = manifest_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            if let Ok(data) = serde_json::to_vec_pretty(&current_manifest) {
                let _ = tokio::fs::write(&manifest_path, data).await;
            }
            self.base.set_cur_task("Cancelled".to_string()).await;
            self.base.set_status(TaskStatus::Cancelled).await;
            return;
        }

        for cache_key in stale_keys {
            if self.wait_if_paused().await || self.is_cancelled() {
                self.base.set_cur_task("Cancelled".to_string()).await;
                self.base.set_status(TaskStatus::Cancelled).await;
                return;
            }
            self.base
                .set_cur_task(format!("Removing {cache_key}.webp"))
                .await;
            let cache_path = self
                .cfg
                .data_path
                .join("cache")
                .join("thumbnails")
                .join(format!("{cache_key}.webp"));
            if let Err(error) = tokio::fs::remove_file(&cache_path).await
                && error.kind() != std::io::ErrorKind::NotFound
            {
                errors.push(format!("{}: {error}", cache_path.display()));
            }
            self.base.add_processed_count(1);
        }

        if let Some(parent) = manifest_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        if let Ok(data) = serde_json::to_vec_pretty(&current_manifest)
            && let Err(error) = tokio::fs::write(&manifest_path, data).await
        {
            errors.push(format!("{}: {error}", manifest_path.display()));
        }

        if self.is_cancelled() {
            self.base.set_cur_task("Cancelled".to_string()).await;
            self.base.set_status(TaskStatus::Cancelled).await;
        } else if errors.is_empty() {
            self.base.set_cur_task("Finished".to_string()).await;
            self.base.set_status(TaskStatus::Finished).await;
        } else {
            self.base
                .set_error_msg(format!("{} thumbnail(s) failed", errors.len()))
                .await;
            self.base.set_status(TaskStatus::Failed).await;
            for error in errors {
                eprintln!("[thumbnail-prewarm] {error}");
            }
        }
    }

    async fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.paused.store(false, Ordering::Relaxed);
        self.base.set_cur_task("Cancelled".to_string()).await;
        self.base.set_status(TaskStatus::Cancelled).await;
    }

    async fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
        self.base.set_status(TaskStatus::Paused).await;
        self.base.set_cur_task("Paused".to_string()).await;
    }

    async fn resume(&self, _extra_info: Value) {
        self.paused.store(false, Ordering::Relaxed);
        self.base.set_status(TaskStatus::Running).await;
        self.base.set_cur_task("Resuming".to_string()).await;
    }
}

pub async fn queue_background_thumbnail_tasks(cfg: &Arc<AppState>) {
    for username in cfg.user_manager.list_usernames().await {
        let locale = cfg.user_manager.get_locale(&username).await;
        let task = Arc::new(BackgroundThumbnailTask::new(
            tr(&locale, "task.background_thumbnails"),
            username,
            cfg.clone(),
        ));
        cfg.task_mgr.add_task_if_absent(task).await;
    }
}

pub fn spawn_thumbnail_prewarmer(cfg: Arc<AppState>) {
    let wake = Arc::new(Notify::new());
    let wake_on_setting_change = wake.clone();
    cfg.settings_manager.register_change_handler(move |change| {
        if change
            .changed_keys
            .iter()
            .any(|key| key == "thumbnail_generation_mode" || key == "thumbnail_scan_interval")
            && crate::backend::settings::SettingsManager::background_thumbnail_generation_from(
                &change.settings,
            )
        {
            wake_on_setting_change.notify_one();
        }
    });

    tokio::spawn(async move {
        loop {
            if cfg.settings_manager.background_thumbnail_generation().await {
                queue_background_thumbnail_tasks(&cfg).await;
            }
            let interval = cfg.settings_manager.thumbnail_scan_interval().await;
            tokio::select! {
                _ = wake.notified() => {}
                _ = tokio::time::sleep(interval) => {}
            }
        }
    });
}

/// Resolve the effective file-system root for a photo that may be owned by
/// another user (shared album). Verifies read access when `owner` ≠ `username`.
pub(crate) async fn resolve_photo_owner_root(
    cfg: &AppState,
    username: &str,
    user_root: PathBuf,
    owner: Option<&str>,
    path: &str,
    locale: &str,
) -> Result<PathBuf, AppError> {
    match owner {
        Some(o) if o != username => {
            crate::backend::album::api::verify_shared_photo_access(cfg, username, o, path, locale).await?;
            Ok(cfg.user_root_path(o))
        }
        _ => Ok(user_root),
    }
}

#[named]
pub async fn api_thumbnail(
    locale: RequestLocale,
    token: Token,
    Path(path): Path<String>,
    query: Query<ThumbnailQuery>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Response<Body>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let effective_root = resolve_photo_owner_root(
        &cfg,
        &username,
        root_path,
        query.owner.as_deref(),
        &path,
        locale.as_str(),
    )
    .await?;

    let source = into_host_path_safely(effective_root.as_path(), &path)?;
    if !source.exists() || !source.is_file() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", path.clone())],
        )));
    }
    if !is_image_path(source.as_path()) {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.not_previewable_image",
        )));
    }

    // size=0 means full resolution (same semantics as darktable_preview).
    // Default matches THUMBNAIL_SIZE / album grid prewarm.
    let size = match query.size {
        Some(0) => 0,
        Some(s) => s.clamp(THUMBNAIL_SIZE_MIN, THUMBNAIL_SIZE_MAX),
        None => THUMBNAIL_SIZE,
    };
    let generator = ThumbnailGenerator::parse(query.generator.as_deref()).ok_or_else(|| {
        AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_thumbnail_generator",
        ))
    })?;
    let metadata = tokio::fs::metadata(&source)
        .await
        .map_err(|_| AppError::NotFound(source.display().to_string()))?;
    let cache_owner = query.owner.as_deref().unwrap_or(&username);
    let cache_path = thumbnail_cache_file(
        cfg.data_path.as_path(),
        cache_owner,
        source.as_path(),
        size,
        &metadata,
        generator,
    );

    // Cache hits bypass the limiter inside ensure_thumbnail_cached().
    ensure_thumbnail_cached(
        source,
        cache_path.clone(),
        size,
        generator,
        &cfg.thumbnail_limiter,
        &cfg.darktable_limiter,
    )
    .await?;

    let file = tokio::fs::File::open(&cache_path)
        .await
        .map_err(|e| AppError::Unknown(format!("open thumbnail failed: {e}")))?;
    let file_size = file
        .metadata()
        .await
        .map_err(|e| AppError::Unknown(format!("stat thumbnail failed: {e}")))?
        .len();

    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("image/webp"));
    headers.insert(
        "Cache-Control",
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    headers.insert(
        "Content-Length",
        HeaderValue::from_str(file_size.to_string().as_str())
            .map_err(|e| AppError::Unknown(format!("invalid content length header: {e}")))?,
    );

    let mut ret = Response::builder()
        .status(StatusCode::OK)
        .body(Body::from_stream(ReaderStream::new(file)))
        .map_err(|e| AppError::Unknown(format!("build thumbnail response failed: {e}")))?;
    *(ret.headers_mut()) = headers;
    Ok(ret)
}

#[named]
pub async fn api_face_thumbnail(
    locale: RequestLocale,
    token: Token,
    Path(path): Path<String>,
    query: Query<FaceThumbnailQuery>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Response<Body>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let effective_root = resolve_photo_owner_root(
        &cfg,
        &username,
        root_path,
        query.owner.as_deref(),
        &path,
        locale.as_str(),
    )
    .await?;

    let source = into_host_path_safely(effective_root.as_path(), &path)?;
    if !source.exists() || !source.is_file() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", path.clone())],
        )));
    }
    if !is_image_path(source.as_path()) {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.not_previewable_image",
        )));
    }

    let box_xywh = parse_face_box_query(query.r#box.as_str(), locale.as_str())?;
    let size = query
        .size
        .unwrap_or(FACE_CROP_SIZE)
        .clamp(FACE_THUMBNAIL_SIZE_MIN, FACE_THUMBNAIL_SIZE_MAX);
    let metadata = tokio::fs::metadata(&source)
        .await
        .map_err(|_| AppError::NotFound(source.display().to_string()))?;
    let cache_owner = query.owner.as_deref().unwrap_or(&username);
    let cache_path = face_thumbnail_cache_file(
        cfg.data_path.as_path(),
        cache_owner,
        source.as_path(),
        size,
        &metadata,
        box_xywh,
    );

    ensure_face_thumbnail_cached(
        source,
        cache_path.clone(),
        size,
        box_xywh,
        &cfg.thumbnail_limiter,
    )
    .await?;

    let file = tokio::fs::File::open(&cache_path)
        .await
        .map_err(|e| AppError::Unknown(format!("open face thumbnail failed: {e}")))?;
    let file_size = file
        .metadata()
        .await
        .map_err(|e| AppError::Unknown(format!("stat face thumbnail failed: {e}")))?
        .len();

    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("image/webp"));
    headers.insert(
        "Cache-Control",
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    headers.insert(
        "Content-Length",
        HeaderValue::from_str(file_size.to_string().as_str())
            .map_err(|e| AppError::Unknown(format!("invalid content length header: {e}")))?,
    );

    let mut ret = Response::builder()
        .status(StatusCode::OK)
        .body(Body::from_stream(ReaderStream::new(file)))
        .map_err(|e| AppError::Unknown(format!("build face thumbnail response failed: {e}")))?;
    *(ret.headers_mut()) = headers;
    Ok(ret)
}

// ── Darktable preview / export ───────────────────────────────────────────────

pub(crate) use super::decoder::find_xmp_sidecar;

#[derive(Debug, Deserialize)]
pub struct DarktablePreviewQuery {
    size: Option<u32>,
    owner: Option<String>,
}

/// GET /api/albums/darktable_preview/{path} — Generate a preview processed through darktable XMP sidecar.
#[named]
pub async fn api_darktable_preview(
    locale: RequestLocale,
    token: Token,
    Path(path): Path<String>,
    query: Query<DarktablePreviewQuery>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Response<Body>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let effective_root = resolve_photo_owner_root(
        &cfg,
        &username,
        root_path,
        query.owner.as_deref(),
        &path,
        locale.as_str(),
    )
    .await?;

    let source = into_host_path_safely(effective_root.as_path(), &path)?;
    if !source.exists() || !source.is_file() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", path.clone())],
        )));
    }
    if !is_image_path(source.as_path()) {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.not_previewable_image",
        )));
    }

    // XMP is optional: use it if found, basic RAW development if not
    let xmp_path = find_xmp_sidecar(&source);

    // size=0 means full resolution (no --width/--height)
    let size = query.size.unwrap_or(DARKTABLE_PREVIEW_DEFAULT_SIZE);
    let size = if size == 0 {
        0
    } else {
        size.clamp(DARKTABLE_PREVIEW_SIZE_MIN, DARKTABLE_PREVIEW_SIZE_MAX)
    };

    // Build a cache key that includes the XMP modification time (if any)
    let src_meta = tokio::fs::metadata(&source)
        .await
        .map_err(|_| AppError::NotFound(source.display().to_string()))?;
    let xmp_modified = if let Some(ref xmp) = xmp_path {
        tokio::fs::metadata(xmp)
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0)
    } else {
        0
    };

    let src_modified = src_meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut hasher = Sha512::new();
    hasher.update(username.as_bytes());
    hasher.update(b"|dt|");
    hasher.update(source.to_string_lossy().as_bytes());
    hasher.update(b"|");
    hasher.update(size.to_string().as_bytes());

    hasher.update(b"|");
    hasher.update(src_meta.len().to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(src_modified.to_string().as_bytes());
    if xmp_path.is_some() {
        hasher.update(b"|xmp|");
        hasher.update(xmp_modified.to_string().as_bytes());
    } else {
        hasher.update(b"|noxmp|");
    }
    let digest = hasher.finalize();
    let cache_key = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);

    let cache_path = cfg
        .data_path
        .join("cache")
        .join("darktable")
        .join(format!("{cache_key}.jpg"));

    if !cache_path.exists() {
        let permit = cfg.darktable_limiter.acquire().await;
        if cache_path.exists() {
            drop(permit);
        } else {
            if let Some(parent) = cache_path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    AppError::Unknown(format!("create darktable cache dir failed: {e}"))
                })?;
            }
            let source_for_decode = source.clone();
            let cache_blocking = cache_path.clone();
            let decode_opts = DecodeOptions::with_xmp().with_max_edge(size);
            let result = async {
                let decoder = create_decoder(DecoderCreateOptions::darktable())
                    .map_err(|error| error.message)?;
                let image = decoder
                    .decode_path(&source_for_decode, decode_opts)
                    .await
                    .map_err(|error| error.message)?;
                tokio::task::spawn_blocking(move || {
                    image
                        .save_with_format(&cache_blocking, image::ImageFormat::Jpeg)
                        .map_err(|e| format!("save darktable preview failed: {e}"))
                })
                .await
                .map_err(|e| format!("darktable preview join failed: {e}"))?
            }
            .await;
            drop(permit);
            result.map_err(AppError::Unknown)?;
        }
    }

    let file = tokio::fs::File::open(&cache_path)
        .await
        .map_err(|e| AppError::Unknown(format!("open darktable preview failed: {e}")))?;
    let file_size = file
        .metadata()
        .await
        .map_err(|e| AppError::Unknown(format!("stat darktable preview failed: {e}")))?
        .len();

    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("image/jpeg"));
    headers.insert(
        "Cache-Control",
        HeaderValue::from_static("private, no-cache"),
    );
    headers.insert(
        "Content-Length",
        HeaderValue::from_str(file_size.to_string().as_str())
            .map_err(|e| AppError::Unknown(format!("invalid content length header: {e}")))?,
    );
    let mut ret = Response::builder()
        .status(StatusCode::OK)
        .body(Body::from_stream(ReaderStream::new(file)))
        .map_err(|e| AppError::Unknown(format!("build darktable preview response failed: {e}")))?;
    *(ret.headers_mut()) = headers;
    Ok(ret)
}

#[derive(Debug, Deserialize)]
pub struct ExportPayload {
    path: String,
    format: String,
    quality: Option<u32>,
    max_width: Option<u32>,
    #[serde(default)]
    owner: Option<String>,
}

/// POST /api/albums/export — Export a photo through darktable-cli with XMP, format & quality options.
/// Returns the exported file as a download.
#[named]
pub async fn api_albums_export(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<ExportPayload>,
) -> Result<Response<Body>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let effective_root = resolve_photo_owner_root(
        &cfg,
        &username,
        root_path,
        payload.owner.as_deref(),
        &payload.path,
        locale.as_str(),
    )
    .await?;

    let source = into_host_path_safely(effective_root.as_path(), &payload.path)?;
    if !source.exists() || !source.is_file() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", payload.path.clone())],
        )));
    }
    if !is_image_path(source.as_path()) {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.not_previewable_image",
        )));
    }

    let xmp_path = find_xmp_sidecar(&source)
        .ok_or_else(|| AppError::BadRequest(tr(locale.as_str(), "error.no_xmp_sidecar")))?;

    // Validate and determine output extension
    let (out_ext, content_type) = match payload.format.to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => ("jpg", "image/jpeg"),
        "png" => ("png", "image/png"),
        "tiff" | "tif" => ("tiff", "image/tiff"),
        "webp" => ("webp", "image/webp"),
        _ => {
            return Err(AppError::BadRequest(tr(
                locale.as_str(),
                "error.unsupported_export_format",
            )));
        }
    };

    let quality = payload.quality.unwrap_or(95).clamp(1, 100);
    let max_width = payload.max_width.unwrap_or(0); // 0 = full size

    // Unique temp output path
    let export_id = uuid::Uuid::new_v4().to_string();
    let export_dir = cfg.data_path.join("cache").join("exports");
    tokio::fs::create_dir_all(&export_dir)
        .await
        .map_err(|e| AppError::Unknown(format!("create export dir failed: {e}")))?;
    let export_path = export_dir.join(format!("{export_id}.{out_ext}"));

    {
        let permit = cfg.darktable_limiter.acquire().await;
        darktable_export(
            &source,
            &xmp_path,
            &export_path,
            out_ext,
            quality,
            max_width,
        )
        .await
        .map_err(|error| AppError::Unknown(error.to_string()))?;
        drop(permit);
    }

    let bytes = tokio::fs::read(&export_path)
        .await
        .map_err(|e| AppError::Unknown(format!("read export file failed: {e}")))?;
    let _ = tokio::fs::remove_file(&export_path).await;

    // Build download filename from original name
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    let download_name = format!("{stem}_edited.{out_ext}");

    let mut headers = HeaderMap::new();
    headers.insert(
        "Content-Type",
        HeaderValue::from_str(content_type)
            .map_err(|e| AppError::Unknown(format!("invalid content type: {e}")))?,
    );
    headers.insert(
        "Content-Disposition",
        HeaderValue::from_str(&format!("attachment; filename=\"{download_name}\""))
            .map_err(|e| AppError::Unknown(format!("invalid disposition header: {e}")))?,
    );
    headers.insert(
        "Content-Length",
        HeaderValue::from_str(bytes.len().to_string().as_str())
            .map_err(|e| AppError::Unknown(format!("invalid content length: {e}")))?,
    );

    let mut ret = Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(bytes))
        .map_err(|e| AppError::Unknown(format!("build export response failed: {e}")))?;
    *(ret.headers_mut()) = headers;
    Ok(ret)
}

#[derive(Debug, Deserialize)]
pub struct HasXmpQuery {
    owner: Option<String>,
}

/// GET /api/albums/has_xmp/{path} — Check if an XMP sidecar exists for the given image.
#[named]
pub async fn api_has_xmp(
    locale: RequestLocale,
    token: Token,
    Path(path): Path<String>,
    query: Query<HasXmpQuery>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let effective_root = resolve_photo_owner_root(
        &cfg,
        &username,
        root_path,
        query.owner.as_deref(),
        &path,
        locale.as_str(),
    )
    .await?;

    let source = into_host_path_safely(effective_root.as_path(), &path)?;
    let has_xmp = source.exists() && find_xmp_sidecar(&source).is_some();
    Ok(Json(json!({ "has_xmp": has_xmp })))
}


#[async_recursion]
pub(crate) async fn collect_album_images(path: &FsPath, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        if is_image_path(path) {
            out.push(path.to_path_buf());
        }
        return;
    }
    if !path.is_dir() {
        return;
    }

    let Ok(mut dir) = tokio::fs::read_dir(path).await else {
        return;
    };
    while let Ok(Some(entry)) = dir.next_entry().await {
        collect_album_images(entry.path().as_path(), out).await;
    }
}

pub(crate) async fn collect_album_image_paths(path: &FsPath) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_album_images(path, &mut out).await;
    out
}
