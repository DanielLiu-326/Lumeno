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
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};
use std::collections::{HashMap, HashSet};
use std::io::BufReader;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::u8;
use tokio::sync::Notify;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::backend::common::{
    AppError, AppState, ConcurrencyLimiter, ConcurrencyPermit, Token, create_response,
    into_host_path_safely, move_file_or_dir,
};
use crate::backend::i18n::{RequestLocale, tr, trf};
use crate::backend::tasks::{Task, TaskBase, TaskStatus};

use super::helpers::{album_index_path, albums_root, ensure_exif_cached, read_album_meta, relative_display_path};
use super::types::*;

#[derive(Debug, Deserialize)]
pub struct ThumbnailQuery {
    size: Option<u32>,
    owner: Option<String>,
}

pub fn is_raw_path(path: &std::path::Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        // Canon
        "cr2" | "cr3" | "crw" |
        // Nikon
        "nef" | "nrw" |
        // Sony
        "arw" | "srf" | "sr2" |
        // Fujifilm
        "raf" |
        // Olympus / OM System
        "orf" |
        // Panasonic / Leica
        "rw2" | "raw" | "rwl" |
        // Pentax / Ricoh
        "pef" |
        // Samsung
        "srw" |
        // Sigma Foveon
        "x3f" |
        // Minolta
        "mrw" |
        // Kodak
        "dcr" | "kdc" |
        // Phase One
        "iiq" |
        // Hasselblad
        "3fr" | "fff" |
        // Mamiya
        "mef" |
        // Leaf / Mamiya
        "mos" |
        // Epson
        "erf" |
        // Generic DNG (most modern cameras)
        "dng"
    )
}

pub fn is_image_path(path: &std::path::Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tif" | "tiff"
    ) || is_raw_path(path)
}

pub fn thumbnail_cache_key(
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
    hasher.update(b"|v2"); // bump to invalidate thumbnails without EXIF orientation
    let digest = hasher.finalize();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest)
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
    thumbnail_limiter: &Arc<ConcurrencyLimiter>,
    darktable_limiter: &Arc<ConcurrencyLimiter>,
) -> Result<(), AppError> {
    if output.exists() {
        return Ok(());
    }

    let limiter = if is_raw_path(&source) {
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

    if is_raw_path(&source) {
        // Use darktable-cli to generate thumbnail for RAW files
        let result = DarktableClient::generate_thumbnail(&source, &output, size, permit).await;
        result
    } else {
        let source_blocking = source.clone();
        let output_blocking = output.clone();
        let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
            let bytes =
                std::fs::read(&source_blocking).map_err(|e| format!("read image failed: {e}"))?;
            let img =
                image::load_from_memory(&bytes).map_err(|e| format!("open image failed: {e}"))?;
            // Apply EXIF orientation so portrait shots display correctly
            let img = {
                let orientation = (|| -> Option<u16> {
                    let mut cursor = std::io::Cursor::new(&bytes);
                    let exif = exif::Reader::new().read_from_container(&mut cursor).ok()?;
                    let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
                    if let exif::Value::Short(ref v) = field.value {
                        v.first().copied()
                    } else {
                        None
                    }
                })()
                .unwrap_or(1);
                match orientation {
                    2 => img.fliph(),
                    3 => img.rotate180(),
                    4 => img.flipv(),
                    5 => img.rotate90().fliph(),
                    6 => img.rotate90(),
                    7 => img.rotate270().fliph(),
                    8 => img.rotate270(),
                    _ => img,
                }
            };
            let thumb = img.thumbnail(size, size);
            thumb
                .save_with_format(&output_blocking, image::ImageFormat::WebP)
                .map_err(|e| format!("save webp thumbnail failed: {e}"))?;
            Ok(())
        })
        .await
        .map_err(|e| AppError::Unknown(format!("thumbnail task join failed: {e}")))?
        .map_err(AppError::Unknown);
        drop(permit);
        result
    }
}

const BACKGROUND_THUMBNAIL_SIZE: u32 = 360;

pub(crate) async fn generate_thumbnail_batch(
    base: &TaskBase,
    cfg: &AppState,
    thumbnails: Vec<(PathBuf, PathBuf)>,
    concurrency: usize,
) -> Vec<String> {
    use futures::StreamExt;

    futures::stream::iter(thumbnails)
        .map(|(source, output)| async move {
            base.set_cur_task(
                source
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_string(),
            )
            .await;
            let error = ensure_thumbnail_cached(
                source.clone(),
                output,
                BACKGROUND_THUMBNAIL_SIZE,
                &cfg.thumbnail_limiter,
                &cfg.darktable_limiter,
            )
            .await
            .err()
            .map(|error| format!("{}: {error}", source.display()));
            base.add_processed_count(1);
            error
        })
        .buffer_unordered(concurrency.max(1))
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect()
}

pub(crate) struct BackgroundThumbnailTask {
    base: TaskBase,
    cfg: Arc<AppState>,
}

impl BackgroundThumbnailTask {
    fn new(name: String, username: String, cfg: Arc<AppState>) -> Self {
        Self {
            base: TaskBase::new(name, username),
            cfg,
        }
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

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Running).await;
        self.base.set_cur_task("Scanning albums".to_string()).await;

        let user_root = self.cfg.user_root_path(&self.base.owner);
        let mut sources = Vec::new();
        collect_album_images(&albums_root(&user_root), &mut sources).await;
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
            let Ok(metadata) = tokio::fs::metadata(&source).await else {
                continue;
            };
            let cache_key = thumbnail_cache_key(
                &self.base.owner,
                &source,
                BACKGROUND_THUMBNAIL_SIZE,
                &metadata,
            );
            let output = self
                .cfg
                .data_path
                .join("cache")
                .join("thumbnails")
                .join(format!("{cache_key}.webp"));
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

        let (raw_thumbnails, image_thumbnails): (Vec<_>, Vec<_>) = thumbnails
            .into_iter()
            .partition(|(source, _)| is_raw_path(source));
        let (mut image_errors, raw_errors) = tokio::join!(
            generate_thumbnail_batch(
                &self.base,
                &self.cfg,
                image_thumbnails,
                self.cfg.thumbnail_limiter.limit(),
            ),
            generate_thumbnail_batch(
                &self.base,
                &self.cfg,
                raw_thumbnails,
                self.cfg.darktable_limiter.limit(),
            ),
        );
        image_errors.extend(raw_errors);
        let mut errors = image_errors;

        for cache_key in stale_keys {
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

        if errors.is_empty() {
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

    async fn cancel(&self) {}
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

/// Encapsulates all `darktable-cli` invocations. Each method creates an
/// isolated config directory per call so concurrent requests don't share a
/// darktable database and cause lock conflicts.
pub(crate) struct DarktableClient;

impl DarktableClient {
    async fn run(
        mut command: tokio::process::Command,
        operation: &'static str,
        source: PathBuf,
        config_dir: PathBuf,
        permit: ConcurrencyPermit,
        thumbnail_output: Option<(PathBuf, PathBuf)>,
    ) -> Result<std::process::ExitStatus, AppError> {
        let task = tokio::spawn(async move {
            let started = std::time::Instant::now();
            let active = permit.active();
            let limit = permit.limit();
            let limiter_id = permit.limiter_id();
            let mut child = command.spawn().map_err(|e| {
                AppError::Unknown(format!("failed to run darktable-cli for {operation}: {e}"))
            })?;
            let pid = child.id().unwrap_or(0);
            eprintln!(
                "[darktable] start service_pid={} limiter={limiter_id:x} operation={operation} pid={pid} active={active}/{limit} source={}",
                std::process::id(),
                source.display()
            );
            let status = child
                .wait()
                .await
                .map_err(|e| AppError::Unknown(format!("failed to wait for darktable-cli: {e}")))?;
            eprintln!(
                "[darktable] finish service_pid={} limiter={limiter_id:x} operation={operation} pid={pid} active={active}/{limit} status={status} elapsed_ms={} source={}",
                std::process::id(),
                started.elapsed().as_millis(),
                source.display()
            );
            let _ = tokio::fs::remove_dir_all(config_dir).await;
            if status.success() {
                if let Some((temp, output)) = thumbnail_output {
                    tokio::task::spawn_blocking(move || -> Result<(), AppError> {
                        let image = image::open(&temp).map_err(|e| {
                            AppError::Unknown(format!("open darktable output: {e}"))
                        })?;
                        image
                            .save_with_format(&output, image::ImageFormat::WebP)
                            .map_err(|e| AppError::Unknown(format!("save webp thumbnail: {e}")))?;
                        let _ = std::fs::remove_file(temp);
                        Ok(())
                    })
                    .await
                    .map_err(|e| {
                        AppError::Unknown(format!("thumbnail convert join failed: {e}"))
                    })??;
                }
            }
            drop(permit);
            Ok(status)
        });

        task.await
            .map_err(|e| AppError::Unknown(format!("darktable task join failed: {e}")))?
    }

    /// Generate a WebP thumbnail for a RAW file via `darktable-cli`.
    async fn generate_thumbnail(
        source: &std::path::Path,
        output: &std::path::Path,
        size: u32,
        permit: ConcurrencyPermit,
    ) -> Result<(), AppError> {
        // darktable-cli outputs to a temp jpg then we convert to webp
        let temp_jpg = output.with_extension("tmp.jpg");
        let _ = tokio::fs::remove_file(&temp_jpg).await;
        let config_dir =
            std::env::temp_dir().join(format!("darktable-config-{}", uuid::Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&config_dir);
        let mut command = tokio::process::Command::new("darktable-cli");
        command
            .arg(source)
            .arg(&temp_jpg)
            .arg("--width")
            .arg(size.to_string())
            .arg("--height")
            .arg(size.to_string())
            .arg("--core")
            .arg("--configdir")
            .arg(&config_dir)
            .arg("--library")
            .arg(":memory:")
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        let status = Self::run(
            command,
            "thumbnail",
            source.to_path_buf(),
            config_dir,
            permit,
            Some((temp_jpg.clone(), output.to_path_buf())),
        )
        .await?;
        if !status.success() {
            let _ = tokio::fs::remove_file(&temp_jpg).await;
            return Err(AppError::Unknown(
                "darktable-cli exited with non-zero status".into(),
            ));
        }
        Ok(())
    }

    /// Generate a JPEG preview via `darktable-cli`, optionally applying an XMP sidecar.
    async fn generate_preview(
        source: &std::path::Path,
        xmp: Option<&std::path::Path>,
        output: &std::path::Path,
        size: u32,
        permit: ConcurrencyPermit,
    ) -> Result<(), AppError> {
        let config_dir =
            std::env::temp_dir().join(format!("darktable-config-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&config_dir).await.ok();
        let mut cmd = tokio::process::Command::new("darktable-cli");
        cmd.arg(source);
        if let Some(xmp_path) = xmp {
            cmd.arg(xmp_path);
        }
        cmd.arg(output);
        if size > 0 {
            cmd.arg("--width")
                .arg(size.to_string())
                .arg("--height")
                .arg(size.to_string());
        }
        cmd.arg("--core")
            .arg("--configdir")
            .arg(&config_dir)
            .arg("--library")
            .arg(":memory:")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let status = Self::run(
            cmd,
            "preview",
            source.to_path_buf(),
            config_dir,
            permit,
            None,
        )
        .await?;
        if !status.success() {
            let _ = tokio::fs::remove_file(output).await;
            return Err(AppError::Unknown(
                "darktable-cli exited with non-zero status".into(),
            ));
        }
        Ok(())
    }

    /// Export a processed image via `darktable-cli` with an XMP sidecar.
    async fn export(
        source: &std::path::Path,
        xmp: &std::path::Path,
        output: &std::path::Path,
        out_ext: &str,
        quality: u32,
        max_width: u32,
        permit: ConcurrencyPermit,
    ) -> Result<(), AppError> {
        let config_dir =
            std::env::temp_dir().join(format!("darktable-config-{}", uuid::Uuid::new_v4()));
        tokio::fs::create_dir_all(&config_dir).await.ok();
        let mut cmd = tokio::process::Command::new("darktable-cli");
        cmd.arg(source).arg(xmp).arg(output);
        if max_width > 0 {
            cmd.arg("--width").arg(max_width.to_string());
        }
        cmd.arg("--core").arg("--configdir").arg(&config_dir);
        if out_ext == "jpg" {
            cmd.arg("--conf")
                .arg(format!("plugins/imageio/format/jpeg/quality={quality}"));
        }
        cmd.arg("--library")
            .arg(":memory:")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let status = Self::run(
            cmd,
            "export",
            source.to_path_buf(),
            config_dir,
            permit,
            None,
        )
        .await?;
        if !status.success() {
            let _ = tokio::fs::remove_file(output).await;
            return Err(AppError::Unknown("darktable-cli export failed".into()));
        }
        Ok(())
    }
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

    let size = query.size.unwrap_or(256).clamp(64, 1024);
    let metadata = tokio::fs::metadata(&source)
        .await
        .map_err(|_| AppError::NotFound(source.display().to_string()))?;
    let cache_owner = query.owner.as_deref().unwrap_or(&username);
    let cache_key = thumbnail_cache_key(cache_owner, source.as_path(), size, &metadata);
    let cache_path = cfg
        .data_path
        .join("cache")
        .join("thumbnails")
        .join(format!("{cache_key}.webp"));

    // Cache hits bypass the limiter inside ensure_thumbnail_cached().
    ensure_thumbnail_cached(
        source,
        cache_path.clone(),
        size,
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

// ── Darktable preview / export ───────────────────────────────────────────────

/// Find the XMP sidecar file for a given source image.
/// Checks: source.xmp, then source_without_ext.xmp
pub(crate) fn find_xmp_sidecar(source: &std::path::Path) -> Option<PathBuf> {
    // e.g. IMG_1234.ARW.xmp
    let xmp1 = source.with_extension(format!(
        "{}.xmp",
        source.extension().unwrap_or_default().to_string_lossy()
    ));
    if xmp1.exists() {
        return Some(xmp1);
    }
    // e.g. IMG_1234.xmp
    let xmp2 = source.with_extension("xmp");
    if xmp2.exists() {
        return Some(xmp2);
    }
    None
}

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
    let size = query.size.unwrap_or(1600);
    let size = if size == 0 { 0 } else { size.clamp(256, 4096) };

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
            let result = DarktableClient::generate_preview(
                &source,
                xmp_path.as_deref(),
                &cache_path,
                size,
                permit,
            )
            .await;
            result?;
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
        let result = DarktableClient::export(
            &source,
            &xmp_path,
            &export_path,
            out_ext,
            quality,
            max_width,
            permit,
        )
        .await;
        result?;
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
