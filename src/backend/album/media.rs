use async_recursion::async_recursion;
use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{Path, Query};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::{Extension, Json};
use base64::Engine;
use function_name::named;
use image::GenericImageView;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha512};
use std::collections::HashMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::u8;
use tokio::sync::Notify;
use tokio_util::io::ReaderStream;

use crate::backend::common::{
    AppError, AppState, ConcurrencyLimiter, Token, into_host_path_safely,
};
use crate::backend::constants::{
    DARKTABLE_PREVIEW_DEFAULT_SIZE, DARKTABLE_PREVIEW_SIZE_MAX, DARKTABLE_PREVIEW_SIZE_MIN,
    FACE_AVATAR_PAD, PREVIEW_THUMBNAIL_SIZE, THUMBNAIL_SIZE, THUMBNAIL_SIZE_MAX,
    THUMBNAIL_SIZE_MIN,
};
use crate::backend::files::send_file;
use crate::backend::i18n::{RequestLocale, tr, trf};
use crate::backend::tasks::{
    Task, TaskBase, TaskStatus, format_subtask, task_file_name, wait_while_paused,
};

use super::decoder::{
    DecodeOptions, DecoderCreateOptions, apply_max_edge, create_decoder, darktable_export,
};
use super::helpers::{albums_root, is_user_visible_album_dir};

#[derive(Debug, Deserialize)]
pub struct ThumbnailQuery {
    size: Option<u32>,
    owner: Option<String>,
    /// Thumbnail generator: `lumeno` (default) or `darktable`.
    generator: Option<String>,
}

/// How a thumbnail is produced. Encoded into the cache key so lumeno and
/// darktable outputs never collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThumbnailGenerator {
    /// App-side decode: raster via `image`, RAW via LibRaw; Nikon HE/HE* uses
    /// the full-resolution JPEG embedded by the camera because TicoRAW is proprietary.
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
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "heic" | "heif" | "hif"
    ) || is_raw_path(path)
}

pub fn is_video_path(path: &std::path::Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "mp4" | "m4v" | "mov" | "webm"
    )
}

pub fn is_album_media_path(path: &std::path::Path) -> bool {
    is_image_path(path) || is_video_path(path)
}

/// Path component of cache keys: relative to the app data root so bind-mount
/// prefixes (`/root/lumeno` vs `/app`) do not invalidate thumbnails / EXIF.
pub(crate) fn cache_relative_path(data_path: &std::path::Path, source: &std::path::Path) -> String {
    fn to_rel(data: &std::path::Path, source: &std::path::Path) -> Option<String> {
        source
            .strip_prefix(data)
            .ok()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .filter(|rel| !rel.is_empty())
    }
    if let Some(rel) = to_rel(data_path, source) {
        return rel;
    }
    if let (Ok(data), Ok(src)) = (data_path.canonicalize(), source.canonicalize())
        && let Some(rel) = to_rel(&data, &src)
    {
        return rel;
    }
    source.to_string_lossy().replace('\\', "/")
}

pub fn thumbnail_cache_key(
    username: &str,
    data_path: &std::path::Path,
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
    hasher.update(cache_relative_path(data_path, source).as_bytes());
    hasher.update(b"|");
    hasher.update(size.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(metadata.len().to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(modified.to_string().as_bytes());
    hasher.update(b"|v4|");
    hasher.update(generator.as_str().as_bytes());
    let digest = hasher.finalize();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
}

/// Resolve the current thumbnail cache path for the selected generator.
/// Files live under `cache/thumbnails/{owner}/` and cannot leave that directory.
pub(crate) fn thumbnail_cache_file(
    data_path: &std::path::Path,
    username: &str,
    source: &std::path::Path,
    size: u32,
    metadata: &std::fs::Metadata,
    generator: ThumbnailGenerator,
) -> Result<PathBuf, AppError> {
    let dir = owner_cache_dir(data_path, "thumbnails", username)?;
    Ok(dir.join(format!(
        "{}.webp",
        thumbnail_cache_key(username, data_path, source, size, metadata, generator)
    )))
}

fn save_decoded_outputs(
    image: image::DynamicImage,
    request_path: PathBuf,
    request_size: u32,
    request_format: image::ImageFormat,
) -> Result<(), String> {
    let requested = apply_max_edge(image, request_size);
    if let Some(parent) = request_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create output dir failed: {e}"))?;
    }
    requested
        .save_with_format(&request_path, request_format)
        .map_err(|e| format!("save decoded output failed: {e}"))?;
    Ok(())
}

async fn xmp_modified_secs(xmp_path: Option<&std::path::Path>) -> Option<u64> {
    let path = xmp_path?;
    let secs = tokio::fs::metadata(path)
        .await
        .ok()
        .and_then(|meta| meta.modified().ok())
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    Some(secs)
}

fn source_modified_secs(metadata: &std::fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// `size == 0` is the full-resolution darktable render. Other sizes are JPEG
/// derivatives of that file and share this key, including the XMP mtime.
fn darktable_preview_cache_path(
    data_path: &std::path::Path,
    username: &str,
    source: &std::path::Path,
    size: u32,
    src_len: u64,
    src_modified: u64,
    xmp_modified: Option<u64>,
) -> PathBuf {
    let mut hasher = Sha512::new();
    hasher.update(username.as_bytes());
    hasher.update(b"|dt|");
    hasher.update(cache_relative_path(data_path, source).as_bytes());
    hasher.update(b"|v2|");
    hasher.update(size.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(src_len.to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(src_modified.to_string().as_bytes());
    if let Some(xmp_modified) = xmp_modified {
        hasher.update(b"|xmp|");
        hasher.update(xmp_modified.to_string().as_bytes());
    } else {
        hasher.update(b"|noxmp|");
    }
    let digest = hasher.finalize();
    let cache_key = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    data_path
        .join("cache")
        .join("darktable")
        .join(format!("{cache_key}.jpg"))
}

fn scale_darktable_preview(master: &std::path::Path, dest: &std::path::Path, size: u32) -> Result<(), String> {
    let image = image::open(master).map_err(|error| error.to_string())?;
    let scaled = apply_max_edge(image, size);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("create darktable cache dir failed: {error}"))?;
    }
    let tmp = dest.with_extension("part");
    scaled
        .save_with_format(&tmp, image::ImageFormat::Jpeg)
        .map_err(|error| format!("save darktable preview failed: {error}"))?;
    std::fs::rename(&tmp, dest).map_err(|error| format!("rename darktable preview failed: {error}"))?;
    Ok(())
}

/// Return the requested darktable JPEG when it already exists, or scale it from
/// the full-resolution render. Does not run darktable-cli.
async fn existing_or_scaled_darktable_preview(
    master: &std::path::Path,
    sized: &std::path::Path,
    size: u32,
) -> Result<Option<PathBuf>, String> {
    if sized.exists() {
        return Ok(Some(sized.to_path_buf()));
    }
    if size == 0 || !master.exists() {
        return Ok(None);
    }
    let master = master.to_path_buf();
    let sized_buf = sized.to_path_buf();
    tokio::task::spawn_blocking(move || scale_darktable_preview(&master, &sized_buf, size))
        .await
        .map_err(|error| format!("scale darktable preview failed: {error}"))??;
    Ok(Some(sized.to_path_buf()))
}

/// Cache key for EXIF metadata, keyed on file path, size, and mtime.
/// Separate from `thumbnail_cache_key` since EXIF has no user or size dimension.
pub(crate) fn exif_cache_key(
    data_path: &std::path::Path,
    source: &std::path::Path,
    metadata: &std::fs::Metadata,
) -> String {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut hasher = Sha512::new();
    hasher.update(b"exif|");
    hasher.update(cache_relative_path(data_path, source).as_bytes());
    hasher.update(b"|");
    hasher.update(metadata.len().to_string().as_bytes());
    hasher.update(b"|");
    hasher.update(modified.to_string().as_bytes());
    hasher.update(b"|v3");
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

    if is_video_path(&source) {
        return ensure_video_poster_cached(source, output, size, thumbnail_limiter).await;
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
    let mut decode_opts = DecodeOptions::for_path(&source)
        .map_err(|error| AppError::Unknown(error.message))?
        .with_max_edge(size);
    if use_darktable {
        decode_opts = decode_opts.with_xmp();
    }
    let source_for_decode = source.clone();
    let output_blocking = output.clone();
    let result = async {
        let decoder = create_decoder(create_opts).map_err(|error| error.message)?;
        let image = decoder
            .decode_path(&source_for_decode, decode_opts)
            .await
            .map_err(|error| error.message)?;
        tokio::task::spawn_blocking(move || {
            save_decoded_outputs(image, output_blocking, size, image::ImageFormat::WebP)
        })
        .await
        .map_err(|e| format!("thumbnail save join failed: {e}"))?
    }
    .await;
    drop(permit);
    result.map_err(AppError::Unknown)
}

async fn ffmpeg_frame_jpeg(source: &FsPath, seek: &str) -> Result<Vec<u8>, String> {
    let output = tokio::process::Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-ss")
        .arg(seek)
        .arg("-i")
        .arg(source)
        .arg("-frames:v")
        .arg("1")
        .arg("-f")
        .arg("image2pipe")
        .arg("-vcodec")
        .arg("mjpeg")
        .arg("-")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await
        .map_err(|e| format!("ffmpeg failed to start: {e}"))?;
    if !output.status.success() || output.stdout.is_empty() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if err.is_empty() {
            return Err("ffmpeg frame extract failed".to_string());
        }
        return Err(format!("ffmpeg frame extract failed: {err}"));
    }
    Ok(output.stdout)
}

async fn extract_video_poster_jpeg(source: &FsPath) -> Result<Vec<u8>, String> {
    match ffmpeg_frame_jpeg(source, "1").await {
        Ok(bytes) => Ok(bytes),
        Err(_) => ffmpeg_frame_jpeg(source, "0").await,
    }
}

async fn ensure_video_poster_cached(
    source: PathBuf,
    output: PathBuf,
    size: u32,
    thumbnail_limiter: &Arc<ConcurrencyLimiter>,
) -> Result<(), AppError> {
    let permit = thumbnail_limiter.acquire().await;
    if output.exists() {
        return Ok(());
    }
    if let Some(parent) = output.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Unknown(format!("create thumbnail cache dir failed: {e}")))?;
    }
    let jpeg = extract_video_poster_jpeg(&source)
        .await
        .map_err(AppError::Unknown)?;
    let output_blocking = output.clone();
    let result = tokio::task::spawn_blocking(move || {
        let image = image::load_from_memory(&jpeg).map_err(|e| e.to_string())?;
        save_decoded_outputs(image, output_blocking, size, image::ImageFormat::WebP)
    })
    .await
    .map_err(|e| AppError::Unknown(format!("thumbnail save join failed: {e}")))?;
    drop(permit);
    result.map_err(AppError::Unknown)
}

pub(crate) fn crop_face_avatar(
    image: &image::DynamicImage,
    box_xywh: [f32; 4],
) -> Result<image::DynamicImage, String> {
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

fn owner_cache_dir(
    data_path: &std::path::Path,
    kind: &str,
    owner: &str,
) -> Result<PathBuf, AppError> {
    if !crate::backend::album::helpers::is_safe_cache_owner(owner) {
        return Err(AppError::NotFound(owner.to_string()));
    }
    let root = data_path.join("cache").join(kind);
    let dir = into_host_path_safely(root.as_path(), owner)?;
    if !dir.starts_with(&root) {
        return Err(AppError::NotFound(owner.to_string()));
    }
    Ok(dir)
}

pub(crate) fn purge_owner_thumbnail_cache(data_path: &std::path::Path, owner: &str) {
    if !crate::backend::album::helpers::is_safe_cache_owner(owner) {
        return;
    }
    if let Ok(dir) = owner_cache_dir(data_path, "thumbnails", owner) {
        let _ = std::fs::remove_dir_all(dir);
    }
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
    if tokio::fs::try_exists(path).await.unwrap_or(false) && !is_readable_face_crop(path).await {
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
        std::fs::create_dir_all(parent).map_err(|e| format!("create face-crops dir: {e}"))?;
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
            .decode_path(
                &source_for_decode,
                DecodeOptions::for_path(&source_for_decode).map_err(|e| e.message)?,
            )
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

/// 360px lumeno preview used by AI scan / pHash.
///
/// Returns the cache path for a valid owner even if the file does not exist
/// yet. `None` only if `username` is not a safe cache-owner segment.
pub(crate) fn preview_thumbnail_cache_path(
    data_path: &std::path::Path,
    username: &str,
    source: &std::path::Path,
    metadata: &std::fs::Metadata,
) -> Option<PathBuf> {
    thumbnail_cache_file(
        data_path,
        username,
        source,
        PREVIEW_THUMBNAIL_SIZE,
        metadata,
        ThumbnailGenerator::Lumeno,
    )
    .ok()
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
                collect_album_media(entry.path().as_path(), &mut sources).await;
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
                self.cfg.data_path.as_path(),
                &source,
                THUMBNAIL_SIZE,
                &metadata,
                ThumbnailGenerator::Lumeno,
            );
            let Ok(output) = thumbnail_cache_file(
                self.cfg.data_path.as_path(),
                &self.base.owner,
                &source,
                THUMBNAIL_SIZE,
                &metadata,
                ThumbnailGenerator::Lumeno,
            ) else {
                continue;
            };
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
            let cache_path = match owner_cache_dir(
                self.cfg.data_path.as_path(),
                "thumbnails",
                self.base.owner.as_str(),
            ) {
                Ok(dir) => dir.join(format!("{cache_key}.webp")),
                Err(_) => continue,
            };
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
                .set_cur_task(format!("Finished ({} thumbnail(s) failed)", errors.len()))
                .await;
            self.base.set_status(TaskStatus::Finished).await;
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
            crate::backend::album::api::verify_shared_photo_access(cfg, username, o, path, locale)
                .await?;
            Ok(cfg.user_root_path(o))
        }
        _ => Ok(user_root),
    }
}

async fn serve_image_file(
    path: &std::path::Path,
    content_type: &'static str,
    cache_control: &'static str,
) -> Result<Response<Body>, AppError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| AppError::Unknown(format!("open image failed: {e}")))?;
    let file_size = file
        .metadata()
        .await
        .map_err(|e| AppError::Unknown(format!("stat image failed: {e}")))?
        .len();
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static(content_type));
    headers.insert("Cache-Control", HeaderValue::from_static(cache_control));
    headers.insert(
        "Content-Length",
        HeaderValue::from_str(file_size.to_string().as_str())
            .map_err(|e| AppError::Unknown(format!("invalid content length header: {e}")))?,
    );
    let mut ret = Response::builder()
        .status(StatusCode::OK)
        .body(Body::from_stream(ReaderStream::new(file)))
        .map_err(|e| AppError::Unknown(format!("build image response failed: {e}")))?;
    *(ret.headers_mut()) = headers;
    Ok(ret)
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
    let video = is_video_path(source.as_path());
    if !video && !is_image_path(source.as_path()) {
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
    // Video posters always use the still-image cache slot. darktable does not apply.
    if !video {
        ThumbnailGenerator::parse(query.generator.as_deref()).ok_or_else(|| {
            AppError::BadRequest(tr(locale.as_str(), "error.invalid_thumbnail_generator"))
        })?;
    }
    let metadata = tokio::fs::metadata(&source)
        .await
        .map_err(|_| AppError::NotFound(source.display().to_string()))?;
    let cache_owner = query.owner.as_deref().unwrap_or(&username);
    if !video {
        let xmp_path = find_xmp_sidecar(&source);
        let xmp_modified = xmp_modified_secs(xmp_path.as_deref()).await;
        let src_modified = source_modified_secs(&metadata);
        let sized = darktable_preview_cache_path(
            cfg.data_path.as_path(),
            &username,
            source.as_path(),
            size,
            metadata.len(),
            src_modified,
            xmp_modified,
        );
        let master = darktable_preview_cache_path(
            cfg.data_path.as_path(),
            &username,
            source.as_path(),
            0,
            metadata.len(),
            src_modified,
            xmp_modified,
        );
        match existing_or_scaled_darktable_preview(&master, &sized, size).await {
            Ok(Some(preview)) => {
                return serve_image_file(
                    &preview,
                    "image/jpeg",
                    "private, max-age=31536000, immutable",
                )
                .await;
            }
            Ok(None) => {}
            Err(error) => eprintln!("[thumbnail] darktable preview scale failed: {error}"),
        }
    }
    let cache_path = thumbnail_cache_file(
        cfg.data_path.as_path(),
        cache_owner,
        source.as_path(),
        size,
        &metadata,
        ThumbnailGenerator::Lumeno,
    )?;
    let owner_dir = owner_cache_dir(cfg.data_path.as_path(), "thumbnails", cache_owner)?;
    if !cache_path.starts_with(&owner_dir) {
        return Err(AppError::NotFound(path.clone()));
    }

    // Cache hits bypass the limiter inside ensure_thumbnail_cached().
    // A missing darktable preview stays on LibRaw; this request does not start darktable.
    ensure_thumbnail_cached(
        source,
        cache_path.clone(),
        size,
        ThumbnailGenerator::Lumeno,
        &cfg.thumbnail_limiter,
        &cfg.darktable_limiter,
    )
    .await?;

    serve_image_file(&cache_path, "image/webp", "private, max-age=31536000, immutable").await
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

    // XMP is optional: use it if found, basic RAW development if not.
    // darktable-cli always writes the original pixel count. Smaller sizes are
    // scaled from that file and stored beside it.
    let xmp_path = find_xmp_sidecar(&source);
    let size = query.size.unwrap_or(DARKTABLE_PREVIEW_DEFAULT_SIZE);
    let size = if size == 0 {
        0
    } else {
        size.clamp(DARKTABLE_PREVIEW_SIZE_MIN, DARKTABLE_PREVIEW_SIZE_MAX)
    };

    let src_meta = tokio::fs::metadata(&source)
        .await
        .map_err(|_| AppError::NotFound(source.display().to_string()))?;
    let xmp_modified = xmp_modified_secs(xmp_path.as_deref()).await;
    let src_modified = source_modified_secs(&src_meta);
    let sized = darktable_preview_cache_path(
        cfg.data_path.as_path(),
        &username,
        source.as_path(),
        size,
        src_meta.len(),
        src_modified,
        xmp_modified,
    );
    let master = darktable_preview_cache_path(
        cfg.data_path.as_path(),
        &username,
        source.as_path(),
        0,
        src_meta.len(),
        src_modified,
        xmp_modified,
    );

    if let Some(preview) = existing_or_scaled_darktable_preview(&master, &sized, size)
        .await
        .map_err(AppError::Unknown)?
    {
        return serve_image_file(&preview, "image/jpeg", "private, no-cache").await;
    }

    if !master.exists() {
        let permit = cfg.darktable_limiter.acquire().await;
        if !master.exists() {
            if let Some(parent) = master.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|e| {
                    AppError::Unknown(format!("create darktable cache dir failed: {e}"))
                })?;
            }
            let source_for_decode = source.clone();
            let master_blocking = master.clone();
            let decode_opts = DecodeOptions::for_path(&source_for_decode)
                .map_err(|error| AppError::Unknown(error.message))?
                .with_xmp()
                .with_max_edge(0);
            let result = async {
                let decoder = create_decoder(DecoderCreateOptions::darktable())
                    .map_err(|error| error.message)?;
                let image = decoder
                    .decode_path(&source_for_decode, decode_opts)
                    .await
                    .map_err(|error| error.message)?;
                tokio::task::spawn_blocking(move || {
                    save_decoded_outputs(
                        image,
                        master_blocking,
                        0,
                        image::ImageFormat::Jpeg,
                    )
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

    let preview = existing_or_scaled_darktable_preview(&master, &sized, size)
        .await
        .map_err(AppError::Unknown)?
        .unwrap_or(master);
    serve_image_file(&preview, "image/jpeg", "private, no-cache").await
}

#[derive(Debug, Deserialize)]
pub struct ExportPayload {
    path: String,
    format: String,
    quality: Option<u32>,
    max_width: Option<u32>,
    /// Apply XMP grading (darktable edit) when exporting. Defaults to true;
    /// when true a sidecar must exist.
    #[serde(default)]
    apply_xmp: Option<bool>,
    #[serde(default)]
    owner: Option<String>,
}

/// POST /api/albums/export — Export a photo as a download.
///
/// With `apply_xmp` the image is graded through darktable-cli with its XMP sidecar.
/// Without it, darktable is only used on demand: an unchanged format at full size
/// streams the original file untouched, otherwise darktable-cli re-encodes
/// (format / max width / quality) without the sidecar.
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

    let xmp_path = if payload.apply_xmp.unwrap_or(true) {
        Some(
            find_xmp_sidecar(&source)
                .ok_or_else(|| AppError::BadRequest(tr(locale.as_str(), "error.no_xmp_sidecar")))?,
        )
    } else {
        None
    };

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

    // Download name from the original file
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();

    // When grading is off and nothing needs converting (same format, full size),
    // stream the original file untouched — no darktable pass at all.
    if xmp_path.is_none() {
        let src_ext = source
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        let same_format = match (src_ext.as_str(), out_ext) {
            ("jpg" | "jpeg", "jpg")
            | ("png", "png")
            | ("tif" | "tiff", "tiff")
            | ("webp", "webp") => true,
            _ => false,
        };
        if same_format && max_width == 0 {
            return send_file(source, true).await;
        }
    }

    let download_name = if xmp_path.is_some() {
        format!("{stem}_edited.{out_ext}")
    } else {
        format!("{stem}.{out_ext}")
    };

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
            xmp_path.as_deref(),
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
    // ASCII fallback plus filename* so non-ASCII (e.g. Chinese) names survive
    // instead of failing HeaderValue::from_str.
    let fallback_name: String = download_name
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_control() && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let fallback_name = if fallback_name.trim_matches('_').is_empty() {
        "export"
    } else {
        fallback_name.as_str()
    };
    let encoded_name = utf8_percent_encode(&download_name, NON_ALPHANUMERIC).to_string();

    let mut headers = HeaderMap::new();
    headers.insert(
        "Content-Type",
        HeaderValue::from_str(content_type)
            .map_err(|e| AppError::Unknown(format!("invalid content type: {e}")))?,
    );
    headers.insert(
        "Content-Disposition",
        HeaderValue::from_str(&format!(
            "attachment; filename=\"{fallback_name}\"; filename*=UTF-8''{encoded_name}"
        ))
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
async fn collect_album_paths(path: &FsPath, out: &mut Vec<PathBuf>, videos: bool) {
    if path.is_file() {
        let include = if videos {
            is_album_media_path(path)
        } else {
            is_image_path(path)
        };
        if include {
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
        collect_album_paths(entry.path().as_path(), out, videos).await;
    }
}

pub(crate) async fn collect_album_images(path: &FsPath, out: &mut Vec<PathBuf>) {
    collect_album_paths(path, out, false).await;
}

pub(crate) async fn collect_album_media(path: &FsPath, out: &mut Vec<PathBuf>) {
    collect_album_paths(path, out, true).await;
}

pub(crate) async fn collect_album_image_paths(path: &FsPath) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_album_images(path, &mut out).await;
    out
}

#[cfg(test)]
mod tests {
    use super::{cache_relative_path, is_album_media_path, is_image_path, is_video_path};
    use std::path::PathBuf;

    #[test]
    fn video_extensions_are_album_media_but_not_images() {
        let mp4 = PathBuf::from("clip.MP4");
        let mov = PathBuf::from("clip.mov");
        let webm = PathBuf::from("clip.webm");
        let jpg = PathBuf::from("still.jpg");
        let txt = PathBuf::from("notes.txt");
        assert!(is_video_path(&mp4));
        assert!(is_video_path(&mov));
        assert!(is_video_path(&webm));
        assert!(is_album_media_path(&mp4));
        assert!(!is_image_path(&mp4));
        assert!(is_image_path(&jpg));
        assert!(is_album_media_path(&jpg));
        assert!(!is_video_path(&jpg));
        assert!(!is_album_media_path(&txt));
    }

    #[test]
    fn owner_cache_dir_rejects_path_escape() {
        let data = PathBuf::from("/app/dist/data");
        assert!(super::owner_cache_dir(&data, "thumbnails", "daniel").is_ok());
        assert!(super::owner_cache_dir(&data, "thumbnails", "../exif").is_err());
        assert!(super::owner_cache_dir(&data, "thumbnails", "a/b").is_err());
        assert!(super::owner_cache_dir(&data, "thumbnails", "").is_err());
        assert!(super::owner_cache_dir(&data, "thumbnails", ".hidden").is_err());
    }

    #[test]
    fn cache_key_uses_path_relative_to_data_root() {
        let data = PathBuf::from("/app/dist/data");
        let under_root = PathBuf::from("/root/lumeno/dist/data");
        let photo_app = data.join("daniel/albums/raw/a.jpg");
        let photo_root = under_root.join("daniel/albums/raw/a.jpg");
        assert_eq!(
            cache_relative_path(&data, &photo_app),
            "daniel/albums/raw/a.jpg"
        );
        assert_eq!(
            cache_relative_path(&under_root, &photo_root),
            "daniel/albums/raw/a.jpg"
        );
        assert_ne!(
            photo_app.to_string_lossy().as_ref(),
            photo_root.to_string_lossy().as_ref()
        );
    }

    #[test]
    fn scaled_darktable_preview_uses_the_full_render() {
        let dir = std::env::temp_dir().join(format!("lumeno-dt-scale-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let master = dir.join("master.jpg");
        let sized = dir.join("sized.jpg");
        let image = image::DynamicImage::new_rgb8(800, 600);
        image
            .save_with_format(&master, image::ImageFormat::Jpeg)
            .unwrap();
        super::scale_darktable_preview(&master, &sized, 100).unwrap();
        let grid = image::open(&sized).unwrap();
        assert!(grid.width().max(grid.height()) <= 100);
        assert!(sized.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
