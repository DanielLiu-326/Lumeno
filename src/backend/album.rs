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

use super::common::{
    AppError, AppState, ConcurrencyLimiter, ConcurrencyPermit, Token, create_response,
    into_host_path_safely, move_file_or_dir,
};
use super::i18n::{RequestLocale, tr, trf};
use super::tasks::{Task, TaskBase, TaskStatus};

// ── Thumbnail ────────────────────────────────────────────────────────────────

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
fn exif_cache_key(source: &std::path::Path, metadata: &std::fs::Metadata) -> String {
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

async fn ensure_thumbnail_cached(
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

async fn generate_thumbnail_batch(
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

struct BackgroundThumbnailTask {
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
            && super::settings::SettingsManager::background_thumbnail_generation_from(
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
struct DarktableClient;

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
async fn resolve_photo_owner_root(
    cfg: &AppState,
    username: &str,
    user_root: PathBuf,
    owner: Option<&str>,
    path: &str,
    locale: &str,
) -> Result<PathBuf, AppError> {
    match owner {
        Some(o) if o != username => {
            verify_shared_photo_access(cfg, username, o, path, locale).await?;
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
fn find_xmp_sidecar(source: &std::path::Path) -> Option<PathBuf> {
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

// ── Album types ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateAlbumPayload {
    name: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAlbumPayload {
    dir_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cover: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteAlbumPayload {
    dir_name: String,
}

#[derive(Debug, Deserialize)]
pub struct ImportAlbumPayload {
    album: String,
    sources: Vec<String>,
    move_files: Option<bool>,
    #[serde(default)]
    conflict: super::files::ConflictAction,
}

#[derive(Debug, Deserialize)]
pub struct AlbumPhotosQuery {
    album: String,
    from: Option<String>,
    to: Option<String>,
    tags: Option<String>,
    ratings: Option<String>,
    color_label: Option<String>,
    /// Legacy single-value camera filter (exact match).
    camera: Option<String>,
    /// Multi-value camera filter: comma-separated list, photo must match any one.
    cameras: Option<String>,
    /// Legacy single-value lens filter (exact match).
    lens: Option<String>,
    /// Multi-value lens filter: comma-separated list, photo must match any one.
    lenses: Option<String>,
    /// Legacy single-value format filter (exact extension match).
    format: Option<String>,
    /// Multi-value format filter: comma-separated list of extensions, photo must match any one.
    formats: Option<String>,
    sort: Option<String>,
    offset: Option<u32>,
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StringListInput {
    List(Vec<String>),
    CommaSeparated(String),
}

fn deserialize_optional_string_list<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<StringListInput>::deserialize(deserializer).map(|value| {
        value.map(|input| match input {
            StringListInput::List(values) => values,
            StringListInput::CommaSeparated(value) => value
                .split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect(),
        })
    })
}

#[derive(Debug, Deserialize)]
pub struct UpdatePhotoMetaPayload {
    path: String,
    #[serde(default)]
    album: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    rating: Option<u8>,
    #[serde(default, deserialize_with = "deserialize_optional_string_list")]
    color_label: Option<Vec<String>>,
}

// ── Color Label ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum ColorLabel {
    #[default]
    None = 255,
    Red = 0,
    Yellow = 1,
    Green = 2,
    Blue = 3,
    Purple = 4,
}

impl ColorLabel {
    pub fn from_darktable(n: u8) -> Self {
        match n {
            0 => Self::Red,
            1 => Self::Yellow,
            2 => Self::Green,
            3 => Self::Blue,
            4 => Self::Purple,
            _ => Self::None,
        }
    }

    pub fn to_darktable(self) -> Option<u8> {
        match self {
            Self::None => None,
            Self::Red => Some(0),
            Self::Yellow => Some(1),
            Self::Green => Some(2),
            Self::Blue => Some(3),
            Self::Purple => Some(4),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Purple => "purple",
        }
    }

    pub fn from_str_label(s: &str) -> Self {
        // Handle legacy comma-separated format by taking the first element
        let first = s.split(',').next().unwrap_or("").trim();
        match first {
            "red" => Self::Red,
            "yellow" => Self::Yellow,
            "green" => Self::Green,
            "blue" => Self::Blue,
            "purple" => Self::Purple,
            _ => Self::None,
        }
    }
}

impl Serialize for ColorLabel {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ColorLabel {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::from_str_label(&s))
    }
}

/// Deserialize `Vec<ColorLabel>` accepting either a JSON array or a legacy
/// comma-separated string (for backward compatibility with old index files).
mod de_color_labels {
    use super::ColorLabel;
    use serde::Deserializer;
    use serde::de::{self, SeqAccess, Visitor};
    use std::fmt;

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ColorLabel>, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Vec<ColorLabel>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("array of color label strings or legacy comma-separated string")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Vec<ColorLabel>, E> {
                Ok(s.split(',')
                    .map(|x| ColorLabel::from_str_label(x.trim()))
                    .filter(|c| *c != ColorLabel::None)
                    .collect())
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<ColorLabel>, A::Error> {
                let mut labels = Vec::new();
                while let Some(s) = seq.next_element::<String>()? {
                    let cl = ColorLabel::from_str_label(&s);
                    if cl != ColorLabel::None {
                        labels.push(cl);
                    }
                }
                Ok(labels)
            }
        }
        d.deserialize_any(V)
    }
}

// ── ImageSoftwareMeta ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct ImageSoftwareMeta {
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    rating: u8,
    #[serde(default, deserialize_with = "de_color_labels::deserialize")]
    color_label: Vec<ColorLabel>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ExifCacheEntry {
    #[serde(default)]
    date: String,
    #[serde(default)]
    datetime: String,
    #[serde(default)]
    camera_make: String,
    #[serde(default)]
    camera_model: String,
    #[serde(default)]
    lens: String,
    #[serde(default)]
    focal_length: String,
    #[serde(default)]
    aperture: String,
    #[serde(default)]
    shutter_speed: String,
    #[serde(default)]
    iso: Option<u32>,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AlbumPhotoItem {
    path: String,
    name: String,
    date: String,
    datetime: String,
    size: u64,
    modified: Option<u64>,
    camera_make: String,
    camera_model: String,
    lens: String,
    focal_length: String,
    aperture: String,
    shutter_speed: String,
    iso: Option<u32>,
    #[serde(default)]
    width: u32,
    #[serde(default)]
    height: u32,
    rating: u8,
    #[serde(default, deserialize_with = "de_color_labels::deserialize")]
    color_label: Vec<ColorLabel>,
    tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AlbumIndex {
    version: u32,
    entries: Vec<AlbumPhotoItem>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AlbumMeta {
    #[serde(default)]
    name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    cover: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    shares: Vec<AlbumShare>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AlbumShare {
    pub username: String,
    #[serde(default)]
    pub can_write: bool,
    #[serde(default)]
    pub can_delete: bool,
}

// ── Album helpers ────────────────────────────────────────────────────────────

fn album_meta_path(album_dir: &FsPath) -> PathBuf {
    album_dir.join("album_meta.json")
}

async fn read_album_meta(album_dir: &FsPath) -> AlbumMeta {
    let path = album_meta_path(album_dir);
    if let Ok(raw) = tokio::fs::read_to_string(&path).await {
        if let Ok(meta) = serde_json::from_str::<AlbumMeta>(&raw) {
            return meta;
        }
    }
    AlbumMeta {
        name: album_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
        ..Default::default()
    }
}

async fn write_album_meta(album_dir: &FsPath, meta: &AlbumMeta) -> Result<(), AppError> {
    let path = album_meta_path(album_dir);
    let raw = serde_json::to_string_pretty(meta)
        .map_err(|e| AppError::Unknown(format!("serialize album meta failed: {e}")))?;
    tokio::fs::write(&path, raw)
        .await
        .map_err(|e| AppError::Unknown(format!("write album meta failed: {e}")))?;
    Ok(())
}

fn albums_root(root_path: &FsPath) -> PathBuf {
    root_path.join("albums")
}

fn cache_exif_root(data_path: &FsPath) -> PathBuf {
    data_path.join("cache").join("exif")
}

fn album_index_path(data_path: &FsPath, username: &str, album_name: &str) -> PathBuf {
    data_path
        .join("cache")
        .join("index")
        .join(username)
        .join(format!("{album_name}.json"))
}

fn xmp_sidecar_path(photo: &FsPath) -> PathBuf {
    PathBuf::from(format!("{}.xmp", photo.to_string_lossy()))
}

/// Parse `xmp:Rating`, `dc:subject` tags, and `darktable:colorlabels` from XMP content.
fn parse_xmp_meta(content: &str) -> ImageSoftwareMeta {
    let mut meta = ImageSoftwareMeta::default();
    let mut reader = quick_xml::Reader::from_str(content);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut depth_dc_subject: u32 = 0;
    let mut in_dc_subject_bag = false;
    let mut depth_colorlabels: u32 = 0;
    let mut in_colorlabels_seq = false;
    let mut collect_tag = false;
    let mut collect_colorlabel = false;

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => match e.name().as_ref() {
                b"rdf:Description" => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"xmp:Rating" {
                            if let Ok(v) = attr.unescape_value() {
                                meta.rating = v.trim().parse::<u8>().unwrap_or(0).min(5);
                            }
                        }
                    }
                }
                b"dc:subject" => {
                    depth_dc_subject += 1;
                }
                b"rdf:Bag" => {
                    if depth_dc_subject > 0 {
                        in_dc_subject_bag = true;
                    }
                }
                b"darktable:colorlabels" => {
                    depth_colorlabels += 1;
                }
                b"rdf:Seq" => {
                    if depth_colorlabels > 0 {
                        in_colorlabels_seq = true;
                    }
                }
                b"rdf:li" => {
                    if in_dc_subject_bag {
                        collect_tag = true;
                    } else if in_colorlabels_seq {
                        collect_colorlabel = true;
                    }
                }
                _ => {}
            },
            Ok(Event::Text(ref e)) => {
                if collect_tag || collect_colorlabel {
                    if let Ok(text) = e.unescape() {
                        let text = text.trim();
                        if collect_tag && !text.is_empty() {
                            meta.tags.push(text.to_string());
                        } else if collect_colorlabel {
                            if let Ok(n) = text.parse::<u8>() {
                                let cl = ColorLabel::from_darktable(n);
                                if cl != ColorLabel::None {
                                    meta.color_label.push(cl);
                                }
                            }
                        }
                    }
                    collect_tag = false;
                    collect_colorlabel = false;
                }
            }
            Ok(Event::End(ref e)) => match e.name().as_ref() {
                b"dc:subject" => {
                    depth_dc_subject = depth_dc_subject.saturating_sub(1);
                    if depth_dc_subject == 0 {
                        in_dc_subject_bag = false;
                    }
                }
                b"rdf:Bag" => {
                    if depth_dc_subject == 0 {
                        in_dc_subject_bag = false;
                    }
                }
                b"darktable:colorlabels" => {
                    depth_colorlabels = depth_colorlabels.saturating_sub(1);
                    if depth_colorlabels == 0 {
                        in_colorlabels_seq = false;
                    }
                }
                b"rdf:Seq" => {
                    if depth_colorlabels == 0 {
                        in_colorlabels_seq = false;
                    }
                }
                b"rdf:li" => {
                    collect_tag = false;
                    collect_colorlabel = false;
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    meta
}

/// Write a `<darktable:colorlabels>` block using a quick-xml Writer.
fn write_colorlabels_block<W: std::io::Write>(writer: &mut Writer<W>, labels: &[ColorLabel]) {
    if labels.is_empty() {
        return;
    }
    writer
        .write_event(Event::Start(BytesStart::new("darktable:colorlabels")))
        .unwrap();
    writer
        .write_event(Event::Start(BytesStart::new("rdf:Seq")))
        .unwrap();
    for cl in labels {
        if let Some(n) = cl.to_darktable() {
            writer
                .write_event(Event::Start(BytesStart::new("rdf:li")))
                .unwrap();
            writer
                .write_event(Event::Text(BytesText::new(&n.to_string())))
                .unwrap();
            writer
                .write_event(Event::End(BytesEnd::new("rdf:li")))
                .unwrap();
        }
    }
    writer
        .write_event(Event::End(BytesEnd::new("rdf:Seq")))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new("darktable:colorlabels")))
        .unwrap();
}

/// Write a `<dc:subject>` block using a quick-xml Writer.
fn write_dc_subject_block<W: std::io::Write>(writer: &mut Writer<W>, tags: &[String]) {
    if tags.is_empty() {
        return;
    }
    writer
        .write_event(Event::Start(BytesStart::new("dc:subject")))
        .unwrap();
    writer
        .write_event(Event::Start(BytesStart::new("rdf:Bag")))
        .unwrap();
    for tag in tags {
        writer
            .write_event(Event::Start(BytesStart::new("rdf:li")))
            .unwrap();
        writer
            .write_event(Event::Text(BytesText::new(tag)))
            .unwrap();
        writer
            .write_event(Event::End(BytesEnd::new("rdf:li")))
            .unwrap();
    }
    writer
        .write_event(Event::End(BytesEnd::new("rdf:Bag")))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new("dc:subject")))
        .unwrap();
}

/// Generate a minimal valid XMP sidecar containing only lumeno-managed fields.
fn generate_minimal_xmp(meta: &ImageSoftwareMeta) -> String {
    let mut out = Vec::new();
    let mut writer = Writer::new_with_indent(&mut out, b' ', 1);

    writer
        .write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .unwrap();

    let mut xmpmeta = BytesStart::new("x:xmpmeta");
    xmpmeta.push_attribute(("xmlns:x", "adobe:ns:meta/"));
    xmpmeta.push_attribute(("x:xmptk", "XMP Core 4.4.0-Exiv2"));
    writer.write_event(Event::Start(xmpmeta)).unwrap();

    let mut rdf = BytesStart::new("rdf:RDF");
    rdf.push_attribute(("xmlns:rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"));
    writer.write_event(Event::Start(rdf)).unwrap();

    let mut desc = BytesStart::new("rdf:Description");
    desc.push_attribute(("rdf:about", ""));
    desc.push_attribute(("xmlns:xmp", "http://ns.adobe.com/xap/1.0/"));
    desc.push_attribute(("xmlns:dc", "http://purl.org/dc/elements/1.1/"));
    if !meta.color_label.is_empty() {
        desc.push_attribute(("xmlns:darktable", "http://darktable.sf.net/"));
    }
    let rating_str = meta.rating.to_string();
    desc.push_attribute(("xmp:Rating", rating_str.as_str()));
    writer.write_event(Event::Start(desc)).unwrap();

    write_colorlabels_block(&mut writer, &meta.color_label);
    write_dc_subject_block(&mut writer, &meta.tags);

    writer
        .write_event(Event::End(BytesEnd::new("rdf:Description")))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new("rdf:RDF")))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new("x:xmpmeta")))
        .unwrap();

    String::from_utf8(out).expect("XMP output is valid UTF-8")
}

/// Merge lumeno metadata (rating, tags, color labels) into existing XMP by streaming
/// with quick-xml Reader/Writer. Only our managed fields are replaced; all other
/// content (darktable history, dc:creator, etc.) is passed through unchanged.
pub fn merge_xmp_meta(existing: Option<&str>, meta: &ImageSoftwareMeta) -> String {
    let source = match existing {
        Some(s) if !s.trim().is_empty() => s,
        _ => return generate_minimal_xmp(meta),
    };

    let mut out: Vec<u8> = Vec::with_capacity(source.len() + 256);
    let mut writer = Writer::new(&mut out);
    let mut reader = quick_xml::Reader::from_str(source);
    // Keep existing whitespace so pass-through content is preserved verbatim.
    reader.config_mut().trim_text(false);

    let mut buf = Vec::new();
    // When > 0 we are inside a block being replaced; track nesting depth.
    let mut skip_depth: u32 = 0;
    let mut skip_is_dc_subject = false;
    let mut found_dc_subject = false;
    let mut found_colorlabels = false;
    let rating_str = meta.rating.to_string();

    loop {
        buf.clear();
        let event = match reader.read_event_into(&mut buf) {
            Ok(e) => e,
            Err(_) => break,
        };

        // ── Inside a skipped block: track nesting, write replacement on close ──
        if skip_depth > 0 {
            match &event {
                Event::Start(_) => skip_depth += 1,
                Event::End(_) => {
                    skip_depth -= 1;
                    if skip_depth == 0 {
                        if skip_is_dc_subject {
                            write_dc_subject_block(&mut writer, &meta.tags);
                        } else {
                            write_colorlabels_block(&mut writer, &meta.color_label);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }

        // ── Normal pass-through with targeted modifications ──
        match event {
            // Rebuild rdf:Description start tag with updated xmp:Rating (and
            // xmlns:darktable if now needed).
            Event::Start(ref e) if e.name().as_ref() == b"rdf:Description" => {
                let mut new_elem = BytesStart::new("rdf:Description");
                let mut found_rating = false;
                let mut found_darktable_ns = false;
                for attr in e.attributes().flatten() {
                    let key = attr.key.as_ref();
                    if key == b"xmp:Rating" {
                        new_elem.push_attribute(("xmp:Rating", rating_str.as_str()));
                        found_rating = true;
                    } else {
                        if key == b"xmlns:darktable" {
                            found_darktable_ns = true;
                        }
                        let k = std::str::from_utf8(attr.key.as_ref()).unwrap_or_default();
                        let v = attr.unescape_value().unwrap_or_default();
                        new_elem.push_attribute((k, v.as_ref()));
                    }
                }
                if !found_rating {
                    new_elem.push_attribute(("xmp:Rating", rating_str.as_str()));
                }
                if !found_darktable_ns && !meta.color_label.is_empty() {
                    new_elem.push_attribute(("xmlns:darktable", "http://darktable.sf.net/"));
                }
                writer.write_event(Event::Start(new_elem)).unwrap();
            }

            // Skip existing dc:subject; write our replacement when block closes.
            Event::Start(ref e) if e.name().as_ref() == b"dc:subject" => {
                skip_depth = 1;
                skip_is_dc_subject = true;
                found_dc_subject = true;
            }

            // Skip existing darktable:colorlabels; write our replacement when block closes.
            Event::Start(ref e) if e.name().as_ref() == b"darktable:colorlabels" => {
                skip_depth = 1;
                skip_is_dc_subject = false;
                found_colorlabels = true;
            }

            // Before closing rdf:Description, insert any blocks that were absent.
            Event::End(ref e) if e.name().as_ref() == b"rdf:Description" => {
                if !found_dc_subject {
                    write_dc_subject_block(&mut writer, &meta.tags);
                }
                if !found_colorlabels {
                    write_colorlabels_block(&mut writer, &meta.color_label);
                }
                writer
                    .write_event(Event::End(BytesEnd::new("rdf:Description")))
                    .unwrap();
            }

            Event::Eof => break,

            // Everything else (darktable:history, dc:creator, whitespace, etc.).
            other => {
                writer.write_event(other).unwrap();
            }
        }
    }

    String::from_utf8(out).unwrap_or_else(|_| generate_minimal_xmp(meta))
}

fn sanitize_album_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('\0')
        || trimmed.starts_with('.')
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut set = HashSet::new();
    let mut out = Vec::new();
    for tag in tags {
        let t = tag.trim().to_lowercase();
        if t.is_empty() {
            continue;
        }
        if set.insert(t.clone()) {
            out.push(t);
        }
    }
    out
}

fn relative_display_path(root_path: &FsPath, full_path: &FsPath) -> String {
    full_path
        .strip_prefix(root_path)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| full_path.to_string_lossy().replace('\\', "/"))
}

fn parse_exif_datetime_to_date(input: &str) -> Option<String> {
    let trimmed = input.trim_matches('"').trim();
    let base = trimmed.get(0..19).unwrap_or(trimmed);
    // EXIF raw uses colons "2024:01:15 10:30:00", but display_value() may
    // format with dashes "2024-01-15 10:30:00". Try both.
    NaiveDateTime::parse_from_str(base, "%Y:%m:%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(base, "%Y-%m-%d %H:%M:%S"))
        .ok()
        .map(|d| d.date().format("%Y-%m-%d").to_string())
}

fn parse_date_from_parent_dirs(path: &FsPath) -> Option<String> {
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy();
        if NaiveDate::parse_from_str(&name, "%Y-%m-%d").is_ok() {
            return Some(name.to_string());
        }
    }
    None
}

fn fallback_date_from_metadata(metadata: &std::fs::Metadata) -> String {
    metadata
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "1970-01-01".to_string())
}

/// Extract EXIF from RAW files via exiftool CLI (the `exif` crate doesn't support most RAW formats).
async fn ensure_exif_via_exiftool(source: &FsPath) -> ExifCacheEntry {
    let output = tokio::process::Command::new("exiftool")
        .arg("-json")
        .arg("-DateTimeOriginal")
        .arg("-Make")
        .arg("-Model")
        .arg("-LensModel")
        .arg("-FocalLength")
        .arg("-FNumber")
        .arg("-ExposureTime")
        .arg("-ISO")
        .arg("-ImageWidth")
        .arg("-ImageHeight")
        .arg(source)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .await;
    let Ok(output) = output else {
        return ExifCacheEntry::default();
    };
    if !output.status.success() {
        return ExifCacheEntry::default();
    }
    let Ok(text) = String::from_utf8(output.stdout) else {
        return ExifCacheEntry::default();
    };
    let Ok(parsed) = serde_json::from_str::<Vec<Value>>(&text) else {
        return ExifCacheEntry::default();
    };
    let Some(obj) = parsed.first() else {
        return ExifCacheEntry::default();
    };

    let get = |key: &str| -> String {
        obj.get(key)
            .and_then(|v| match v {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .unwrap_or_default()
    };

    let datetime_raw = get("DateTimeOriginal");
    let date = parse_exif_datetime_to_date(&datetime_raw).unwrap_or_default();
    // Normalize "2026:04:04 17:27:57" to "2026-04-04 17:27:57" for display consistency
    let datetime_display = if datetime_raw.len() >= 10 && &datetime_raw[4..5] == ":" {
        format!(
            "{}{}",
            datetime_raw[..10].replace(':', "-"),
            &datetime_raw[10..]
        )
    } else {
        datetime_raw
    };

    let aperture_raw = get("FNumber");
    let aperture = if aperture_raw.is_empty() {
        String::new()
    } else {
        format!("f/{aperture_raw}")
    };

    let shutter = get("ExposureTime");
    let shutter_speed = if shutter.is_empty() {
        String::new()
    } else if shutter.contains('/') {
        format!("{shutter} s")
    } else if let Ok(val) = shutter.parse::<f64>() {
        if val < 1.0 && val > 0.0 {
            format!("1/{} s", (1.0 / val).round() as u64)
        } else {
            format!("{shutter} s")
        }
    } else {
        format!("{shutter} s")
    };

    let focal = get("FocalLength");
    let focal_length = if focal.is_empty() {
        String::new()
    } else if focal.contains("mm") {
        focal
    } else {
        format!("{focal} mm")
    };

    let iso = get("ISO").parse::<u32>().ok();

    ExifCacheEntry {
        date,
        datetime: datetime_display,
        camera_make: get("Make"),
        camera_model: get("Model"),
        lens: get("LensModel").trim().to_string(),
        focal_length,
        aperture,
        shutter_speed,
        iso,
        width: get("ImageWidth").parse().unwrap_or(0),
        height: get("ImageHeight").parse().unwrap_or(0),
    }
}

async fn ensure_exif_cached(
    data_path: &FsPath,
    source: &FsPath,
) -> Result<ExifCacheEntry, AppError> {
    let metadata = tokio::fs::metadata(source)
        .await
        .map_err(|_| AppError::NotFound(source.display().to_string()))?;

    let cache_key = exif_cache_key(source, &metadata);
    let cache_path = cache_exif_root(data_path).join(format!("{cache_key}.json"));

    if cache_path.exists() {
        let cached_raw = tokio::fs::read_to_string(&cache_path)
            .await
            .map_err(|e| AppError::Unknown(format!("read exif cache failed: {e}")))?;
        if let Ok(cached) = serde_json::from_str::<ExifCacheEntry>(&cached_raw)
            && !cached.date.is_empty()
        {
            return Ok(cached);
        }
    }

    let source_for_block = source.to_path_buf();
    let is_raw = is_raw_path(source);
    let exif_info = if is_raw {
        // Use exiftool CLI for RAW files (the exif crate doesn't support most RAW formats)
        ensure_exif_via_exiftool(source).await
    } else {
        tokio::task::spawn_blocking(move || -> ExifCacheEntry {
            let (width, height) = image::image_dimensions(&source_for_block).unwrap_or((0, 0));
            let file = match std::fs::File::open(&source_for_block) {
                Ok(f) => f,
                Err(_) => {
                    return ExifCacheEntry {
                        width,
                        height,
                        ..Default::default()
                    };
                }
            };
            let mut reader = BufReader::new(file);
            let exif = match exif::Reader::new().read_from_container(&mut reader) {
                Ok(e) => e,
                Err(_) => {
                    return ExifCacheEntry {
                        width,
                        height,
                        ..Default::default()
                    };
                }
            };

            let get_str = |tag: exif::Tag| -> String {
                exif.get_field(tag, exif::In::PRIMARY)
                    .map(|f| f.display_value().with_unit(&exif).to_string())
                    .unwrap_or_default()
            };

            let datetime_raw = get_str(exif::Tag::DateTimeOriginal);
            let datetime_raw = if datetime_raw.is_empty() {
                get_str(exif::Tag::DateTime)
            } else {
                datetime_raw
            };
            let datetime_raw = datetime_raw.trim_matches('"').trim().to_string();
            let date = parse_exif_datetime_to_date(&datetime_raw).unwrap_or_default();

            let iso = exif
                .get_field(exif::Tag::PhotographicSensitivity, exif::In::PRIMARY)
                .and_then(|f| match f.value {
                    exif::Value::Short(ref v) => v.first().map(|&x| x as u32),
                    exif::Value::Long(ref v) => v.first().copied(),
                    _ => f.display_value().to_string().parse::<u32>().ok(),
                });

            ExifCacheEntry {
                date,
                datetime: datetime_raw.clone(),
                camera_make: get_str(exif::Tag::Make)
                    .trim_matches('"')
                    .trim()
                    .to_string(),
                camera_model: get_str(exif::Tag::Model)
                    .trim_matches('"')
                    .trim()
                    .to_string(),
                lens: get_str(exif::Tag::LensModel)
                    .trim_matches('"')
                    .trim()
                    .to_string(),
                focal_length: get_str(exif::Tag::FocalLength),
                aperture: get_str(exif::Tag::FNumber),
                shutter_speed: get_str(exif::Tag::ExposureTime),
                iso,
                width,
                height,
            }
        })
        .await
        .map_err(|e| AppError::Unknown(format!("read exif join failed: {e}")))?
    };

    let mut entry = exif_info;
    if entry.date.is_empty() {
        entry.date = fallback_date_from_metadata(&metadata);
    }

    if let Some(parent) = cache_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Unknown(format!("create exif cache dir failed: {e}")))?;
    }
    let cache_data = serde_json::to_string(&entry)
        .map_err(|e| AppError::Unknown(format!("serialize exif cache failed: {e}")))?;
    tokio::fs::write(&cache_path, cache_data)
        .await
        .map_err(|e| AppError::Unknown(format!("write exif cache failed: {e}")))?;

    Ok(entry)
}

async fn read_image_software_meta(image_path: &FsPath) -> ImageSoftwareMeta {
    let xmp_path = find_xmp_sidecar(image_path);
    match xmp_path {
        Some(ref p) => match tokio::fs::read_to_string(p).await {
            Ok(raw) => parse_xmp_meta(&raw),
            Err(_) => ImageSoftwareMeta::default(),
        },
        None => ImageSoftwareMeta::default(),
    }
}

async fn write_image_software_meta_if_needed(
    image_path: &FsPath,
    meta: &ImageSoftwareMeta,
) -> Result<(), AppError> {
    let xmp_path = xmp_sidecar_path(image_path);
    if meta.tags.is_empty() && meta.rating == 0 && meta.color_label.is_empty() {
        if xmp_path.exists() {
            let _ = tokio::fs::remove_file(&xmp_path).await;
        }
        return Ok(());
    }

    let existing = tokio::fs::read_to_string(&xmp_path).await.ok();
    let new_xmp = merge_xmp_meta(existing.as_deref(), meta);
    tokio::fs::write(&xmp_path, new_xmp)
        .await
        .map_err(|e| AppError::Unknown(format!("write XMP sidecar failed: {e}")))?;
    Ok(())
}

#[async_recursion]
async fn collect_album_images(path: &FsPath, out: &mut Vec<PathBuf>) {
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

// ── Migration: JSON sidecars → XMP ───────────────────────────────────────────

#[async_recursion]
async fn scan_json_sidecars_recursive(
    dir: &FsPath,
    excluded_dir: &FsPath,
    out: &mut Vec<PathBuf>,
) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path == excluded_dir {
            continue;
        }
        if path.is_dir() {
            scan_json_sidecars_recursive(&path, excluded_dir, out).await;
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                let image_path = path.parent().unwrap_or_else(|| FsPath::new("")).join(stem);
                if image_path.exists() && is_image_path(&image_path) {
                    out.push(path);
                }
            }
        }
    }
}

/// Scan `data_path` recursively for legacy `.json` image sidecars.
pub async fn scan_json_sidecars(data_path: &FsPath) -> Vec<PathBuf> {
    let mut result = Vec::new();
    scan_json_sidecars_recursive(data_path, &data_path.join("cache"), &mut result).await;
    result
}

/// Migrate a single `.json` sidecar to XMP, then delete the JSON file.
pub async fn migrate_json_sidecar(json_path: &FsPath) {
    let Ok(raw) = tokio::fs::read_to_string(json_path).await else {
        return;
    };
    let Ok(meta) = serde_json::from_str::<ImageSoftwareMeta>(&raw) else {
        return;
    };
    let path_str = json_path.to_string_lossy();
    let image_str = path_str.strip_suffix(".json").unwrap_or(&path_str);
    let xmp_path = xmp_sidecar_path(FsPath::new(image_str));
    let existing_xmp = tokio::fs::read_to_string(&xmp_path).await.ok();
    let new_xmp = merge_xmp_meta(existing_xmp.as_deref(), &meta);
    if tokio::fs::write(&xmp_path, new_xmp).await.is_ok() {
        let _ = tokio::fs::remove_file(json_path).await;
    }
}

async fn unique_destination_path(dir: &FsPath, file_name: &str) -> PathBuf {
    let mut candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let stem = FsPath::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    let ext = FsPath::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let mut idx = 1usize;
    loop {
        let name = if ext.is_empty() {
            format!("{stem}_{idx}")
        } else {
            format!("{stem}_{idx}.{ext}")
        };
        candidate = dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
        idx += 1;
    }
}

async fn rebuild_album_index(
    data_path: &FsPath,
    root_path: &FsPath,
    username: &str,
    album_name: &str,
    task_base: Option<&TaskBase>,
) -> Result<AlbumIndex, AppError> {
    use futures::StreamExt;

    let album_dir = albums_root(root_path).join(album_name);
    if !album_dir.exists() {
        return Ok(AlbumIndex {
            version: 2,
            entries: Vec::new(),
        });
    }

    let mut photos_paths = Vec::new();
    collect_album_images(album_dir.as_path(), &mut photos_paths).await;
    let total = photos_paths.len();

    if let Some(base) = task_base {
        base.set_total_count(total);
        base.set_status(TaskStatus::Running).await;
    }

    let data_path_buf = data_path.to_path_buf();
    let root_path_buf = root_path.to_path_buf();

    // Process EXIF/metadata concurrently — up to 8 photos at a time.
    let mut entries: Vec<AlbumPhotoItem> = futures::stream::iter(photos_paths)
        .map(|photo| {
            let data_path = data_path_buf.clone();
            let root_path = root_path_buf.clone();
            let task_base = task_base;
            async move {
                if let Some(base) = task_base {
                    base.set_cur_task(
                        photo
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or_default()
                            .to_string(),
                    )
                    .await;
                }
                let Ok(meta) = tokio::fs::metadata(&photo).await else {
                    if let Some(base) = task_base {
                        base.add_processed_count(1);
                    }
                    return None;
                };
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());

                let mut exif_info = ensure_exif_cached(&data_path, photo.as_path())
                    .await
                    .unwrap_or_default();
                if exif_info.date.is_empty() {
                    exif_info.date = parse_date_from_parent_dirs(photo.as_path())
                        .unwrap_or_else(|| fallback_date_from_metadata(&meta));
                }

                let software_meta = read_image_software_meta(photo.as_path()).await;
                let tags = normalize_tags(&software_meta.tags);

                let item = AlbumPhotoItem {
                    path: relative_display_path(&root_path, photo.as_path()),
                    name: photo
                        .file_name()
                        .and_then(|f| f.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    date: exif_info.date,
                    datetime: exif_info.datetime,
                    size: meta.len(),
                    modified,
                    camera_make: exif_info.camera_make,
                    camera_model: exif_info.camera_model,
                    lens: exif_info.lens,
                    focal_length: exif_info.focal_length,
                    aperture: exif_info.aperture,
                    shutter_speed: exif_info.shutter_speed,
                    iso: exif_info.iso,
                    width: exif_info.width,
                    height: exif_info.height,
                    rating: software_meta.rating,
                    color_label: software_meta.color_label,
                    tags,
                };
                if let Some(base) = task_base {
                    base.add_processed_count(1);
                }
                Some(item)
            }
        })
        .buffer_unordered(8)
        .filter_map(|x| async move { x })
        .collect()
        .await;

    entries.sort_by(|a, b| b.date.cmp(&a.date).then(a.name.cmp(&b.name)));

    let index = AlbumIndex {
        version: 2,
        entries,
    };

    let index_path = album_index_path(data_path, username, album_name);
    if let Some(parent) = index_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Unknown(format!("create index dir failed: {e}")))?;
    }
    let index_data = serde_json::to_string(&index)
        .map_err(|e| AppError::Unknown(format!("serialize index failed: {e}")))?;
    tokio::fs::write(&index_path, index_data)
        .await
        .map_err(|e| AppError::Unknown(format!("write index failed: {e}")))?;

    Ok(index)
}

pub(crate) async fn import_album_files(
    cfg: &Arc<AppState>,
    root_path: &FsPath,
    username: &str,
    album_name: &str,
    sources: &[String],
    move_files: bool,
    conflict_action: super::files::ConflictAction,
    task_base: Option<&TaskBase>,
) -> Result<usize, AppError> {
    let album_base = albums_root(root_path).join(album_name);
    tokio::fs::create_dir_all(&album_base)
        .await
        .map_err(|e| AppError::Unknown(format!("create album dir failed: {e}")))?;

    let mut all_images = Vec::new();
    for source in sources {
        let full = into_host_path_safely(root_path, source)?;
        collect_album_images(full.as_path(), &mut all_images).await;
    }

    if let Some(base) = task_base {
        base.set_total_count(all_images.len());
        base.set_status(TaskStatus::Running).await;
    }

    if all_images.is_empty() {
        return Ok(0);
    }

    let mut imported = 0usize;
    for image in all_images {
        if let Some(base) = task_base {
            base.set_cur_task(
                image
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            )
            .await;
        }

        let exif = ensure_exif_cached(cfg.data_path.as_path(), image.as_path()).await?;
        let date = if exif.date.is_empty() {
            "unknown".to_string()
        } else {
            exif.date
        };
        let year = date.get(0..4).unwrap_or("unknown").to_string();
        let target_dir = album_base.join(&year).join(&date);
        tokio::fs::create_dir_all(&target_dir)
            .await
            .map_err(|e| AppError::Unknown(format!("create target dir failed: {e}")))?;

        let file_name = image
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("image");
        // Apply conflict action
        let dst_candidate = target_dir.join(file_name);
        let dst = if dst_candidate.exists() {
            match conflict_action {
                super::files::ConflictAction::Rename => {
                    unique_destination_path(target_dir.as_path(), file_name).await
                }
                super::files::ConflictAction::Overwrite => dst_candidate,
                super::files::ConflictAction::Skip => {
                    imported += 1;
                    if let Some(base) = task_base {
                        base.add_processed_count(1);
                    }
                    continue;
                }
            }
        } else {
            dst_candidate
        };

        if move_files {
            if move_file_or_dir(image.as_path(), dst.as_path())
                .await
                .is_err()
            {
                tokio::fs::copy(image.as_path(), dst.as_path())
                    .await
                    .map_err(|e| AppError::Unknown(format!("import copy fallback failed: {e}")))?;
                let _ = tokio::fs::remove_file(image.as_path()).await;
            }
        } else {
            tokio::fs::copy(image.as_path(), dst.as_path())
                .await
                .map_err(|e| AppError::Unknown(format!("import copy failed: {e}")))?;
        }

        // Also move/copy any XMP sidecar alongside the image.
        {
            let image_dir = image.parent().unwrap_or_else(|| FsPath::new(""));
            let target_dir = dst.parent().unwrap_or_else(|| FsPath::new(""));
            let stem = image.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let img_name_str = image.file_name().and_then(|s| s.to_str()).unwrap_or("");
            for xmp_filename in [format!("{stem}.xmp"), format!("{img_name_str}.xmp")] {
                let xmp_src = image_dir.join(&xmp_filename);
                if xmp_src.exists() {
                    let xmp_dst = target_dir.join(&xmp_filename);
                    if move_files {
                        let _ = move_file_or_dir(&xmp_src, &xmp_dst).await;
                    } else {
                        let _ = tokio::fs::copy(&xmp_src, &xmp_dst).await;
                    }
                    break;
                }
            }
        }

        imported += 1;

        if let Some(base) = task_base {
            base.add_processed_count(1);
        }
    }

    let _ = rebuild_album_index(
        cfg.data_path.as_path(),
        root_path,
        username,
        album_name,
        None,
    )
    .await;

    Ok(imported)
}

// ── Rebuild index task ───────────────────────────────────────────────────────

struct RebuildIndexTask {
    base: TaskBase,
    data_path: PathBuf,
    root_path: PathBuf,
    owner: String,
    album_name: String,
}

impl RebuildIndexTask {
    fn new(
        task_name: String,
        username: String,
        data_path: PathBuf,
        root_path: PathBuf,
        owner: String,
        album_name: String,
    ) -> Self {
        Self {
            base: TaskBase::new(task_name, username),
            data_path,
            root_path,
            owner,
            album_name,
        }
    }
}

#[async_trait]
impl Task for RebuildIndexTask {
    fn base(&self) -> &TaskBase {
        &self.base
    }

    fn deduplication_key(&self) -> Option<String> {
        Some(format!("rebuild-index:{}:{}", self.owner, self.album_name))
    }

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Pending).await;
        match rebuild_album_index(
            &self.data_path,
            &self.root_path,
            &self.owner,
            &self.album_name,
            Some(&self.base),
        )
        .await
        {
            Ok(index) => {
                self.base
                    .set_cur_task(format!("Finished, {} photos indexed", index.entries.len()))
                    .await;
                self.base.set_status(TaskStatus::Finished).await;
            }
            Err(e) => {
                self.base.set_error_msg(format!("{e}")).await;
                self.base.set_status(TaskStatus::Failed).await;
            }
        }
    }

    async fn cancel(&self) {}
}

struct ImportAlbumTask {
    base: TaskBase,
    cfg: Arc<AppState>,
    root_path: PathBuf,
    username: String,
    album_name: String,
    sources: Vec<String>,
    move_files: bool,
    conflict_action: super::files::ConflictAction,
}

impl ImportAlbumTask {
    fn new(
        task_name: String,
        username: String,
        cfg: Arc<AppState>,
        root_path: PathBuf,
        album_name: String,
        sources: Vec<String>,
        move_files: bool,
        conflict_action: super::files::ConflictAction,
    ) -> Self {
        Self {
            base: TaskBase::new(task_name, username.clone()),
            cfg,
            root_path,
            username,
            album_name,
            sources,
            move_files,
            conflict_action,
        }
    }
}

#[async_trait]
impl Task for ImportAlbumTask {
    fn base(&self) -> &TaskBase {
        &self.base
    }

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Pending).await;
        match import_album_files(
            &self.cfg,
            &self.root_path,
            &self.username,
            &self.album_name,
            &self.sources,
            self.move_files,
            self.conflict_action,
            Some(&self.base),
        )
        .await
        {
            Ok(imported) => {
                self.base
                    .set_cur_task(format!("Finished, {imported} photos imported"))
                    .await;
                self.base.set_status(TaskStatus::Finished).await;
            }
            Err(e) => {
                self.base.set_error_msg(format!("{e}")).await;
                self.base.set_status(TaskStatus::Failed).await;
            }
        }
    }

    async fn cancel(&self) {}
}

async fn load_existing_album_index(
    data_path: &FsPath,
    username: &str,
    album_name: &str,
) -> Result<Option<AlbumIndex>, AppError> {
    let index_path = album_index_path(data_path, username, album_name);
    if !index_path.exists() {
        return Ok(None);
    }
    let raw = tokio::fs::read_to_string(&index_path)
        .await
        .map_err(|e| AppError::Unknown(format!("read index failed: {e}")))?;
    if let Ok(index) = serde_json::from_str::<AlbumIndex>(&raw)
        && index.version >= 2
    {
        return Ok(Some(index));
    }
    Ok(None)
}

async fn load_or_queue_album_index(
    cfg: &AppState,
    task_owner: &str,
    root_path: &FsPath,
    index_owner: &str,
    album_name: &str,
    locale: &str,
) -> Result<Option<AlbumIndex>, AppError> {
    if let Some(index) =
        load_existing_album_index(cfg.data_path.as_path(), index_owner, album_name).await?
    {
        return Ok(Some(index));
    }

    let task = Arc::new(RebuildIndexTask::new(
        trf(
            locale,
            "task.rebuild_index",
            &[("album", album_name.to_string())],
        ),
        task_owner.to_string(),
        cfg.data_path.clone(),
        root_path.to_path_buf(),
        index_owner.to_string(),
        album_name.to_string(),
    ));
    cfg.task_mgr.add_task_if_absent(task).await;
    Ok(None)
}

// ── Album API handlers ───────────────────────────────────────────────────────

#[named]
pub async fn api_albums_list(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let albums_dir = albums_root(root_path.as_path());
    tokio::fs::create_dir_all(&albums_dir)
        .await
        .map_err(|e| AppError::Unknown(format!("create albums dir failed: {e}")))?;

    let mut album_names = Vec::new();
    let mut entries = tokio::fs::read_dir(&albums_dir)
        .await
        .map_err(|e| AppError::Unknown(format!("read albums dir failed: {e}")))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| AppError::Unknown(format!("read album entry failed: {e}")))?
    {
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if meta.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.starts_with("__") {
                album_names.push(name);
            }
        }
    }
    album_names.sort();

    let mut albums = Vec::new();
    for dir_name in &album_names {
        let album_dir = albums_dir.join(dir_name);
        let meta = read_album_meta(album_dir.as_path()).await;
        let display_name = if meta.name.is_empty() {
            dir_name.clone()
        } else {
            meta.name.clone()
        };
        let index = load_or_queue_album_index(
            cfg.as_ref(),
            &username,
            root_path.as_path(),
            &username,
            dir_name,
            locale.as_str(),
        )
        .await
        .ok()
        .flatten();
        let photo_count = index.map(|i| i.entries.len()).unwrap_or(0);
        albums.push(json!({
            "dir_name": dir_name,
            "name": display_name,
            "cover": meta.cover,
            "description": meta.description,
            "photo_count": photo_count,
            "shares": meta.shares,
        }));
    }

    let shared_albums = collect_shared_albums(&cfg, &username, locale.as_str()).await;

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "albums": albums,
            "shared_albums": shared_albums,
        })),
    ))
}

#[named]
pub async fn api_albums_create(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<CreateAlbumPayload>,
) -> Result<Json<Value>, AppError> {
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(name) = sanitize_album_name(&payload.name) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_album_name",
        )));
    };

    let album_dir = albums_root(root_path.as_path()).join(&name);
    tokio::fs::create_dir_all(&album_dir)
        .await
        .map_err(|e| AppError::Unknown(format!("create album dir failed: {e}")))?;
    let meta = AlbumMeta {
        name: name.clone(),
        cover: String::new(),
        description: payload.description.clone().unwrap_or_default(),
        ..Default::default()
    };
    write_album_meta(album_dir.as_path(), &meta).await?;
    let message = tr(locale.as_str(), "response.album_created");
    Ok(create_response(true, &message, 0, None))
}

#[named]
pub async fn api_albums_update(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<UpdateAlbumPayload>,
) -> Result<Json<Value>, AppError> {
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(dir_name) = sanitize_album_name(&payload.dir_name) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_album_identifier",
        )));
    };
    let album_dir = albums_root(root_path.as_path()).join(&dir_name);
    if !album_dir.exists() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", dir_name.clone())],
        )));
    }
    let mut meta = read_album_meta(album_dir.as_path()).await;
    if let Some(name) = &payload.name {
        meta.name = name.trim().to_string();
    }
    if let Some(cover) = &payload.cover {
        meta.cover = cover.trim().to_string();
    }
    if let Some(desc) = &payload.description {
        meta.description = desc.clone();
    }
    write_album_meta(album_dir.as_path(), &meta).await?;
    let message = tr(locale.as_str(), "response.album_updated");
    Ok(create_response(true, &message, 0, None))
}

#[named]
pub async fn api_albums_delete(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<DeleteAlbumPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(dir_name) = sanitize_album_name(&payload.dir_name) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_album_identifier",
        )));
    };
    let album_dir = albums_root(root_path.as_path()).join(&dir_name);
    if !album_dir.exists() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", dir_name.clone())],
        )));
    }
    tokio::fs::remove_dir_all(&album_dir)
        .await
        .map_err(|e| AppError::Unknown(format!("delete album dir failed: {e}")))?;
    let index_path = album_index_path(cfg.data_path.as_path(), &username, &dir_name);
    let _ = tokio::fs::remove_file(&index_path).await;
    let message = tr(locale.as_str(), "response.album_deleted");
    Ok(create_response(true, &message, 0, None))
}

#[named]
pub async fn api_albums_import(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<ImportAlbumPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let ImportAlbumPayload {
        album,
        sources,
        move_files,
        conflict,
    } = payload.0;

    let Some(album_name) = sanitize_album_name(&album) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_album_name",
        )));
    };
    if sources.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.import_sources_empty",
        )));
    }

    for source in &sources {
        let _ = into_host_path_safely(root_path.as_path(), source)?;
    }

    // All sources must be from the same directory
    {
        let first_parent = std::path::Path::new(sources.first().unwrap())
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .to_string();
        for source in &sources[1..] {
            let parent = std::path::Path::new(source)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_string();
            if parent != first_parent {
                return Err(AppError::BadRequest(tr(
                    locale.as_str(),
                    "error.import_sources_different_dirs",
                )));
            }
        }
    }

    let task = Arc::new(ImportAlbumTask::new(
        trf(
            locale.as_str(),
            "task.import_album",
            &[("album", album_name.clone())],
        ),
        username,
        cfg.clone(),
        root_path,
        album_name,
        sources,
        move_files.unwrap_or(false),
        conflict,
    ));
    let task_id = task.uuid().to_string();
    cfg.task_mgr.add_task(task).await;

    let message = tr(locale.as_str(), "response.album_import_task_created");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({ "task_id": task_id })),
    ))
}

// ── Date histogram ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DateHistogramQuery {
    album: String,
}

#[named]
pub async fn api_albums_date_histogram(
    locale: RequestLocale,
    token: Token,
    query: Query<DateHistogramQuery>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let resolved =
        resolve_album_access(&cfg, &username, &root_path, &query.album, locale.as_str()).await?;
    let album_dir = albums_root(resolved.root_path.as_path()).join(&resolved.album_name);

    if !album_dir.exists() {
        let message = tr(locale.as_str(), "response.success");
        return Ok(create_response(
            true,
            &message,
            0,
            Some(json!({ "buckets": [] })),
        ));
    }

    let Some(index) = load_or_queue_album_index(
        cfg.as_ref(),
        &username,
        resolved.root_path.as_path(),
        &resolved.owner,
        &resolved.album_name,
        locale.as_str(),
    )
    .await?
    else {
        let message = tr(locale.as_str(), "response.success");
        return Ok(create_response(
            true,
            &message,
            0,
            Some(json!({ "buckets": [] })),
        ));
    };

    // Aggregate counts by YYYY-MM-DD
    let mut day_counts: Vec<(String, usize)> = Vec::new();
    for entry in &index.entries {
        let date = if entry.date.len() >= 10 {
            &entry.date[..10]
        } else {
            &entry.date
        };
        if let Some(last) = day_counts.last_mut() {
            if last.0 == date {
                last.1 += 1;
                continue;
            }
        }
        day_counts.push((date.to_string(), 1));
    }

    // Index entries are sorted by date desc, so re-sort day_counts ascending
    day_counts.sort_by(|a, b| a.0.cmp(&b.0));

    let buckets: Vec<Value> = day_counts
        .iter()
        .map(|(d, c)| json!({ "date": d, "count": c }))
        .collect();

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({ "buckets": buckets })),
    ))
}

// ── Collections ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Default)]
struct CollectionFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    camera: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lens: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ratings: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
}

#[derive(Debug, Serialize)]
struct CollectionNode {
    id: String,
    label: String,
    count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<CollectionFilter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    children: Vec<CollectionNode>,
}

/// GET /api/albums/collections?album=<name> — Build a Darktable-style collection tree
/// from the album index. Each node carries a `filter` object that the frontend applies
/// directly to the photos query, so the backend needs no separate logic.
#[named]
pub async fn api_albums_collections(
    locale: RequestLocale,
    token: Token,
    query: Query<DateHistogramQuery>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let resolved =
        resolve_album_access(&cfg, &username, &root_path, &query.album, locale.as_str()).await?;
    let album_dir = albums_root(resolved.root_path.as_path()).join(&resolved.album_name);

    if !album_dir.exists() {
        let message = tr(locale.as_str(), "response.success");
        return Ok(create_response(
            true,
            &message,
            0,
            Some(json!({ "collections": [] })),
        ));
    }

    let Some(index) = load_or_queue_album_index(
        cfg.as_ref(),
        &username,
        resolved.root_path.as_path(),
        &resolved.owner,
        &resolved.album_name,
        locale.as_str(),
    )
    .await?
    else {
        let message = tr(locale.as_str(), "response.success");
        return Ok(create_response(
            true,
            &message,
            0,
            Some(json!({ "collections": [] })),
        ));
    };

    let total = index.entries.len();

    // ── All Photos ──
    let all_node = CollectionNode {
        id: "all".to_string(),
        label: "all".to_string(),
        count: total,
        filter: Some(CollectionFilter::default()),
        children: vec![],
    };

    // ── By Date: year → month ──
    let mut date_tree: std::collections::BTreeMap<i32, std::collections::BTreeMap<u32, usize>> =
        std::collections::BTreeMap::new();
    for entry in &index.entries {
        if entry.date.len() >= 7 {
            if let (Ok(year), Ok(month)) = (
                entry.date[..4].parse::<i32>(),
                entry.date[5..7].parse::<u32>(),
            ) {
                if (1..=12).contains(&month) {
                    *date_tree.entry(year).or_default().entry(month).or_default() += 1;
                }
            }
        }
    }
    let date_children: Vec<CollectionNode> = date_tree
        .iter()
        .rev()
        .map(|(&year, months)| {
            let year_count: usize = months.values().sum();
            let month_children: Vec<CollectionNode> = months
                .iter()
                .rev()
                .filter_map(|(&month, &count)| {
                    let first_day = format!("{:04}-{:02}-01", year, month);
                    let last_day = NaiveDate::from_ymd_opt(
                        if month == 12 { year + 1 } else { year },
                        if month == 12 { 1 } else { month + 1 },
                        1,
                    )?
                    .pred_opt()?
                    .format("%Y-%m-%d")
                    .to_string();
                    Some(CollectionNode {
                        id: format!("date:{:04}-{:02}", year, month),
                        label: format!("{:04}-{:02}", year, month),
                        count,
                        filter: Some(CollectionFilter {
                            from: Some(first_day),
                            to: Some(last_day),
                            ..Default::default()
                        }),
                        children: vec![],
                    })
                })
                .collect();
            CollectionNode {
                id: format!("date:{}", year),
                label: year.to_string(),
                count: year_count,
                filter: Some(CollectionFilter {
                    from: Some(format!("{:04}-01-01", year)),
                    to: Some(format!("{:04}-12-31", year)),
                    ..Default::default()
                }),
                children: month_children,
            }
        })
        .collect();
    let date_group = CollectionNode {
        id: "date".to_string(),
        label: "date".to_string(),
        count: total,
        filter: None,
        children: date_children,
    };

    // ── By Camera ──
    let mut camera_map: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for entry in &index.entries {
        let cam = format!("{} {}", entry.camera_make, entry.camera_model)
            .trim()
            .to_string();
        if !cam.is_empty() {
            *camera_map.entry(cam).or_default() += 1;
        }
    }
    let camera_children: Vec<CollectionNode> = camera_map
        .iter()
        .map(|(cam, &count)| CollectionNode {
            id: format!("camera:{}", cam),
            label: cam.clone(),
            count,
            filter: Some(CollectionFilter {
                camera: Some(cam.clone()),
                ..Default::default()
            }),
            children: vec![],
        })
        .collect();
    let camera_group = CollectionNode {
        id: "camera".to_string(),
        label: "camera".to_string(),
        count: camera_children.iter().map(|c| c.count).sum(),
        filter: None,
        children: camera_children,
    };

    // ── By Lens ──
    let mut lens_map: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for entry in &index.entries {
        if !entry.lens.is_empty() {
            *lens_map.entry(entry.lens.clone()).or_default() += 1;
        }
    }
    let lens_children: Vec<CollectionNode> = lens_map
        .iter()
        .map(|(lens, &count)| CollectionNode {
            id: format!("lens:{}", lens),
            label: lens.clone(),
            count,
            filter: Some(CollectionFilter {
                lens: Some(lens.clone()),
                ..Default::default()
            }),
            children: vec![],
        })
        .collect();
    let lens_group = CollectionNode {
        id: "lens".to_string(),
        label: "lens".to_string(),
        count: lens_children.iter().map(|c| c.count).sum(),
        filter: None,
        children: lens_children,
    };

    // ── By Tag ──
    let mut tag_map: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for entry in &index.entries {
        for tag in &entry.tags {
            *tag_map.entry(tag.clone()).or_default() += 1;
        }
    }
    let tag_children: Vec<CollectionNode> = tag_map
        .iter()
        .map(|(tag, &count)| CollectionNode {
            id: format!("tag:{}", tag),
            label: tag.clone(),
            count,
            filter: Some(CollectionFilter {
                tags: Some(tag.clone()),
                ..Default::default()
            }),
            children: vec![],
        })
        .collect();
    let tag_group = CollectionNode {
        id: "tag".to_string(),
        label: "tag".to_string(),
        count: tag_children.iter().map(|c| c.count).sum(),
        filter: None,
        children: tag_children,
    };

    // ── By Rating ──
    let mut rating_map: std::collections::BTreeMap<u8, usize> = std::collections::BTreeMap::new();
    for entry in &index.entries {
        *rating_map.entry(entry.rating).or_default() += 1;
    }
    let rating_children: Vec<CollectionNode> = (1u8..=5)
        .rev()
        .filter_map(|star| {
            rating_map.get(&star).map(|&count| CollectionNode {
                id: format!("rating:{}", star),
                label: "★".repeat(star as usize),
                count,
                filter: Some(CollectionFilter {
                    ratings: Some(star.to_string()),
                    ..Default::default()
                }),
                children: vec![],
            })
        })
        .collect();
    let rating_group = CollectionNode {
        id: "rating".to_string(),
        label: "rating".to_string(),
        count: rating_children.iter().map(|c| c.count).sum(),
        filter: None,
        children: rating_children,
    };

    // ── By Color Label ──
    let mut color_map: HashMap<String, usize> = HashMap::new();
    for entry in &index.entries {
        for cl in &entry.color_label {
            let s = cl.as_str();
            if !s.is_empty() {
                *color_map.entry(s.to_string()).or_default() += 1;
            }
        }
    }
    let color_order = ["red", "yellow", "green", "blue", "purple"];
    let color_children: Vec<CollectionNode> = color_order
        .iter()
        .filter_map(|&key| {
            color_map.get(key).map(|&count| {
                let mut label = key.to_string();
                label.get_mut(0..1).map(|s| s.make_ascii_uppercase());
                CollectionNode {
                    id: format!("color:{}", key),
                    label,
                    count,
                    filter: Some(CollectionFilter {
                        color_label: Some(key.to_string()),
                        ..Default::default()
                    }),
                    children: vec![],
                }
            })
        })
        .collect();
    let color_group = CollectionNode {
        id: "color".to_string(),
        label: "color".to_string(),
        count: color_children.iter().map(|c| c.count).sum(),
        filter: None,
        children: color_children,
    };

    // ── By Format ──
    let mut format_map: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for entry in &index.entries {
        if let Some(ext) = std::path::Path::new(&entry.name)
            .extension()
            .and_then(|e| e.to_str())
        {
            *format_map.entry(ext.to_ascii_uppercase()).or_default() += 1;
        }
    }
    let format_children: Vec<CollectionNode> = format_map
        .iter()
        .map(|(fmt, &count)| CollectionNode {
            id: format!("format:{}", fmt.to_ascii_lowercase()),
            label: fmt.clone(),
            count,
            filter: Some(CollectionFilter {
                format: Some(fmt.to_ascii_lowercase()),
                ..Default::default()
            }),
            children: vec![],
        })
        .collect();
    let format_group = CollectionNode {
        id: "format".to_string(),
        label: "format".to_string(),
        count: format_children.iter().map(|c| c.count).sum(),
        filter: None,
        children: format_children,
    };

    let collections = vec![
        all_node,
        date_group,
        camera_group,
        lens_group,
        tag_group,
        rating_group,
        color_group,
        format_group,
    ];

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({ "collections": collections })),
    ))
}

#[named]
pub async fn api_albums_photos(
    locale: RequestLocale,
    token: Token,
    query: Query<AlbumPhotosQuery>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let resolved =
        resolve_album_access(&cfg, &username, &root_path, &query.album, locale.as_str()).await?;
    let album_name = resolved.album_name;
    let root_path = resolved.root_path;
    let owner = resolved.owner;

    let album_dir = albums_root(root_path.as_path()).join(&album_name);
    let empty_response = || {
        let message = tr(locale.as_str(), "response.success");
        create_response(
            true,
            &message,
            0,
            Some(json!({
                "photos": [],
                "available_tags": [],
                "available_cameras": [],
                "available_lenses": [],
                "total_count": 0,
                "offset": 0,
                "limit": 0,
            })),
        )
    };
    if !album_dir.exists() {
        return Ok(empty_response());
    }

    let Some(index) = load_or_queue_album_index(
        cfg.as_ref(),
        &username,
        root_path.as_path(),
        &owner,
        &album_name,
        locale.as_str(),
    )
    .await?
    else {
        return Ok(empty_response());
    };

    let requested_tags = query
        .tags
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();

    let from = query.from.as_deref().unwrap_or("").to_string();
    let to = query.to.as_deref().unwrap_or("").to_string();

    // Parse ratings filter: comma-separated list of allowed rating values (e.g. "0,3,5")
    let ratings_filter: Vec<u8> = query
        .ratings
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter_map(|s| s.trim().parse::<u8>().ok())
        .collect();

    let color_filter_set: Vec<String> = query
        .color_label
        .as_deref()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    // Camera: prefer multi-value list; fall back to legacy single-value.
    let cameras_filter: Vec<String> = {
        let multi = query.cameras.as_deref().unwrap_or("");
        if !multi.is_empty() {
            multi
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            let single = query.camera.as_deref().unwrap_or("").to_lowercase();
            if single.is_empty() {
                vec![]
            } else {
                vec![single]
            }
        }
    };

    // Lens: prefer multi-value list; fall back to legacy single-value.
    let lenses_filter: Vec<String> = {
        let multi = query.lenses.as_deref().unwrap_or("");
        if !multi.is_empty() {
            multi
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            let single = query.lens.as_deref().unwrap_or("").to_lowercase();
            if single.is_empty() {
                vec![]
            } else {
                vec![single]
            }
        }
    };

    // Format: prefer multi-value list; fall back to legacy single-value.
    let formats_filter: Vec<String> = {
        let multi = query.formats.as_deref().unwrap_or("");
        if !multi.is_empty() {
            multi
                .split(',')
                .map(|s| s.trim().to_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        } else {
            let single = query.format.as_deref().unwrap_or("").to_lowercase();
            if single.is_empty() {
                vec![]
            } else {
                vec![single]
            }
        }
    };
    let offset = query.offset.unwrap_or(0) as usize;
    let requested_limit = query.limit.unwrap_or(100);
    let limit = if requested_limit == 0 {
        usize::MAX
    } else {
        requested_limit as usize
    };
    let include_facets = offset == 0;

    // Parse sort criteria: comma-separated "field:dir" pairs (e.g. "date:desc,name:asc")
    let sort_criteria: Vec<(&str, bool)> = query
        .sort
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter_map(|s| {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            let mut parts = s.splitn(2, ':');
            let field = parts.next().unwrap_or("").trim();
            let dir = parts.next().unwrap_or("asc").trim();
            if field.is_empty() {
                return None;
            }
            Some((field, dir == "desc"))
        })
        .collect();

    let matches_filters = |p: &AlbumPhotoItem| {
        if !from.is_empty() && p.date < from {
            return false;
        }
        if !to.is_empty() && p.date > to {
            return false;
        }
        if !ratings_filter.is_empty() && !ratings_filter.contains(&p.rating) {
            return false;
        }
        if !color_filter_set.is_empty() {
            if !p
                .color_label
                .iter()
                .any(|cl| color_filter_set.iter().any(|f| f == cl.as_str()))
            {
                return false;
            }
        }
        if !cameras_filter.is_empty() {
            let cam = format!("{} {}", p.camera_make, p.camera_model)
                .trim()
                .to_lowercase();
            if !cameras_filter.contains(&cam) {
                return false;
            }
        }
        if !lenses_filter.is_empty() && !lenses_filter.contains(&p.lens.to_lowercase()) {
            return false;
        }
        if !formats_filter.is_empty() {
            let ext = std::path::Path::new(&p.name)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();
            if !formats_filter.contains(&ext) {
                return false;
            }
        }
        if !requested_tags.is_empty() && !requested_tags.iter().all(|t| p.tags.contains(t)) {
            return false;
        }
        true
    };

    let (available_tags, available_cameras, available_lenses, available_formats) = if include_facets
    {
        let mut all_tags = HashSet::new();
        let mut all_cameras = HashSet::new();
        let mut all_lenses = HashSet::new();
        let mut all_formats = HashSet::new();

        for entry in &index.entries {
            for t in &entry.tags {
                all_tags.insert(t.clone());
            }
            let cam = format!("{} {}", entry.camera_make, entry.camera_model)
                .trim()
                .to_string();
            if !cam.is_empty() {
                all_cameras.insert(cam);
            }
            if !entry.lens.is_empty() {
                all_lenses.insert(entry.lens.clone());
            }
            if let Some(ext) = std::path::Path::new(&entry.name)
                .extension()
                .and_then(|e| e.to_str())
            {
                all_formats.insert(ext.to_uppercase());
            }
        }

        let mut available_tags = all_tags.into_iter().collect::<Vec<_>>();
        available_tags.sort();
        let mut available_cameras = all_cameras.into_iter().collect::<Vec<_>>();
        available_cameras.sort();
        let mut available_lenses = all_lenses.into_iter().collect::<Vec<_>>();
        available_lenses.sort();
        let mut available_formats = all_formats.into_iter().collect::<Vec<_>>();
        available_formats.sort();

        (
            available_tags,
            available_cameras,
            available_lenses,
            available_formats,
        )
    } else {
        (Vec::new(), Vec::new(), Vec::new(), Vec::new())
    };

    // Apply sorting
    let (paginated_photos, total_count): (Vec<&AlbumPhotoItem>, usize) = if sort_criteria.is_empty()
    {
        let mut total_count = 0usize;
        let mut paginated_photos = Vec::with_capacity(limit.min(64));

        for photo in index.entries.iter() {
            if !matches_filters(photo) {
                continue;
            }

            if total_count >= offset && paginated_photos.len() < limit {
                paginated_photos.push(photo);
            }
            total_count += 1;
        }

        (paginated_photos, total_count)
    } else {
        let mut photos: Vec<&AlbumPhotoItem> = index
            .entries
            .iter()
            .filter(|photo| matches_filters(photo))
            .collect();

        photos.sort_by(|a, b| {
            for &(field, desc) in &sort_criteria {
                let ord = match field {
                    "name" => a.name.cmp(&b.name),
                    "date" => a.date.cmp(&b.date),
                    "size" => a.size.cmp(&b.size),
                    "rating" => a.rating.cmp(&b.rating),
                    "camera" => {
                        let ca = format!("{} {}", a.camera_make, a.camera_model);
                        let cb = format!("{} {}", b.camera_make, b.camera_model);
                        ca.cmp(&cb)
                    }
                    "lens" => a.lens.cmp(&b.lens),
                    _ => std::cmp::Ordering::Equal,
                };
                let ord = if desc { ord.reverse() } else { ord };
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            std::cmp::Ordering::Equal
        });

        let total_count = photos.len();
        let paginated_photos = photos.into_iter().skip(offset).take(limit).collect();
        (paginated_photos, total_count)
    };

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "photos": paginated_photos,
            "available_tags": available_tags,
            "available_cameras": available_cameras,
            "available_lenses": available_lenses,
            "available_formats": available_formats,
            "total_count": total_count,
            "offset": offset,
            "limit": requested_limit,
        })),
    ))
}

#[named]
pub async fn api_albums_photo_meta(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<UpdatePhotoMetaPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, user_root)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    // Determine the effective root path and owner for the photo
    let (root_path, index_owner) = if let Some(album_ref) = &payload.album {
        let resolved =
            resolve_album_access(&cfg, &username, &user_root, album_ref, locale.as_str()).await?;
        if !resolved.can_write {
            return Err(AppError::Forbidden(tr(
                locale.as_str(),
                "error.no_write_permission_album",
            )));
        }
        (resolved.root_path, resolved.owner)
    } else {
        (user_root, username.clone())
    };

    let photo = into_host_path_safely(root_path.as_path(), &payload.path)?;
    if !photo.exists() || !photo.is_file() || !is_image_path(photo.as_path()) {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_image_path",
        )));
    }

    let mut current = read_image_software_meta(photo.as_path()).await;
    if let Some(tags) = &payload.tags {
        current.tags = normalize_tags(tags);
    }
    if let Some(rating) = payload.rating {
        current.rating = rating.min(5);
    }
    if let Some(colors) = &payload.color_label {
        current.color_label = colors
            .iter()
            .map(|s| ColorLabel::from_str_label(s))
            .filter(|c| *c != ColorLabel::None)
            .collect();
    }
    write_image_software_meta_if_needed(photo.as_path(), &current).await?;

    let photo_rel = relative_display_path(root_path.as_path(), photo.as_path());
    if let Some(album_name) = photo_rel
        .strip_prefix("albums/")
        .and_then(|rest| rest.split('/').next())
    {
        let index_path = album_index_path(cfg.data_path.as_path(), &index_owner, album_name);
        if index_path.exists() {
            if let Ok(raw) = tokio::fs::read_to_string(&index_path).await {
                if let Ok(mut index) = serde_json::from_str::<AlbumIndex>(&raw) {
                    if let Some(entry) = index.entries.iter_mut().find(|e| e.path == photo_rel) {
                        entry.tags = current.tags.clone();
                        entry.rating = current.rating;
                        entry.color_label = current.color_label.clone();
                    }
                    if let Ok(data) = serde_json::to_string(&index) {
                        let _ = tokio::fs::write(&index_path, data).await;
                    }
                }
            }
        }
    }

    let message = tr(locale.as_str(), "response.metadata_updated");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "tags": current.tags,
            "rating": current.rating,
            "color_label": current.color_label,
        })),
    ))
}

#[derive(Debug, Deserialize)]
pub struct DeletePhotosPayload {
    album: String,
    paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct RebuildIndexPayload {
    album: String,
}

#[named]
pub async fn api_albums_rebuild_index(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<RebuildIndexPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, user_root)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let resolved =
        resolve_album_access(&cfg, &username, &user_root, &payload.album, locale.as_str()).await?;

    let album_label = resolved.album_name.clone();
    let task = Arc::new(RebuildIndexTask::new(
        trf(
            locale.as_str(),
            "task.rebuild_index",
            &[("album", album_label.clone())],
        ),
        username,
        cfg.data_path.clone(),
        resolved.root_path,
        resolved.owner,
        resolved.album_name,
    ));
    let (task_id, _) = cfg.task_mgr.add_task_if_absent(task).await;

    let message = tr(locale.as_str(), "response.rebuild_index_task_created");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({ "task_id": task_id.to_string() })),
    ))
}

// ── Delete photos ────────────────────────────────────────────────────────────

#[named]
pub async fn api_albums_delete_photos(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<DeletePhotosPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, user_root)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let resolved =
        resolve_album_access(&cfg, &username, &user_root, &payload.album, locale.as_str()).await?;
    if !resolved.can_delete {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.no_delete_permission_album",
        )));
    }

    if payload.paths.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.no_photos_selected",
        )));
    }

    let album_dir = albums_root(&resolved.root_path).join(&resolved.album_name);
    let mut deleted = 0usize;
    for rel_path in &payload.paths {
        let full = into_host_path_safely(resolved.root_path.as_path(), rel_path)?;
        // Ensure the file is actually inside this album dir
        if !full.starts_with(&album_dir) {
            continue;
        }
        if full.is_file() {
            let _ = tokio::fs::remove_file(&full).await;
            // Also remove XMP sidecar metadata
            let sidecar = xmp_sidecar_path(&full);
            if sidecar.exists() {
                let _ = tokio::fs::remove_file(&sidecar).await;
            }
            deleted += 1;
        }
    }

    // Clean up empty directories inside the album
    cleanup_empty_dirs(&album_dir).await;

    // Rebuild the index
    let index = rebuild_album_index(
        cfg.data_path.as_path(),
        resolved.root_path.as_path(),
        &resolved.owner,
        &resolved.album_name,
        None,
    )
    .await?;

    let message = trf(
        locale.as_str(),
        "response.photos_deleted",
        &[("count", deleted.to_string())],
    );
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "deleted": deleted,
            "remaining": index.entries.len(),
        })),
    ))
}

// ── Copy photo to another album ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CopyPhotoPayload {
    photo_path: String,
    source_album: String,
    target_album: String,
}

pub async fn api_albums_copy_photo(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<CopyPhotoPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, user_root)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    // Resolve source album (read access required)
    let source = resolve_album_access(
        &cfg,
        &username,
        &user_root,
        &payload.source_album,
        locale.as_str(),
    )
    .await?;

    // Resolve target album (write access required)
    let target = resolve_album_access(
        &cfg,
        &username,
        &user_root,
        &payload.target_album,
        locale.as_str(),
    )
    .await?;
    if !target.can_write {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.no_write_permission_album",
        )));
    }

    // Validate that photo path is inside the source album directory
    let source_file = into_host_path_safely(source.root_path.as_path(), &payload.photo_path)?;
    let source_album_dir = albums_root(&source.root_path).join(&source.album_name);
    if !source_file.starts_with(&source_album_dir) {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.invalid_shared_path",
        )));
    }
    if !source_file.is_file() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", payload.photo_path.clone())],
        )));
    }

    // Destination: target album dir + same filename (handle conflicts)
    let target_album_dir = albums_root(&target.root_path).join(&target.album_name);
    tokio::fs::create_dir_all(&target_album_dir)
        .await
        .map_err(|e| AppError::Unknown(format!("create target album dir failed: {e}")))?;

    let file_name = source_file
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| AppError::BadRequest("invalid file name".to_string()))?;
    let stem = std::path::Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    let ext = std::path::Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let dest = {
        let candidate = target_album_dir.join(file_name);
        if !candidate.exists() {
            candidate
        } else {
            let mut i = 1u32;
            loop {
                let name = if ext.is_empty() {
                    format!("{}_{}", stem, i)
                } else {
                    format!("{}_{}.{}", stem, i, ext)
                };
                let candidate = target_album_dir.join(&name);
                if !candidate.exists() {
                    break candidate;
                }
                i += 1;
            }
        }
    };

    tokio::fs::copy(&source_file, &dest)
        .await
        .map_err(|e| AppError::Unknown(format!("copy photo failed: {e}")))?;

    // Rebuild target album index
    rebuild_album_index(
        cfg.data_path.as_path(),
        target.root_path.as_path(),
        &target.owner,
        &target.album_name,
        None,
    )
    .await?;

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(true, &message, 0, None))
}

/// Remove empty directories recursively inside `dir`.
#[async_recursion]
async fn cleanup_empty_dirs(dir: &FsPath) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.metadata().await.map(|m| m.is_dir()).unwrap_or(false) {
            cleanup_empty_dirs(&entry.path()).await;
        }
    }
    // After cleaning children, remove this dir if it's now empty
    // (but only sub-dirs, not the album root — we check by re-reading)
    if let Ok(mut re) = tokio::fs::read_dir(dir).await {
        if re.next_entry().await.ok().flatten().is_none() {
            let _ = tokio::fs::remove_dir(dir).await;
        }
    }
}

// ── Sharing ──────────────────────────────────────────────────────────────────

/// Verify that `username` has read access to a photo path owned by `owner`.
/// The path should be of form "albums/{album_name}/..." relative to owner's root.
pub async fn verify_shared_photo_access(
    cfg: &AppState,
    username: &str,
    owner: &str,
    path: &str,
    locale: &str,
) -> Result<(), AppError> {
    let album_name = path
        .strip_prefix("albums/")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("");
    if album_name.is_empty() {
        return Err(AppError::Forbidden(tr(locale, "error.invalid_shared_path")));
    }
    let owner_root = cfg.user_root_path(owner);
    let album_dir = albums_root(&owner_root).join(album_name);
    if !album_dir.exists() {
        return Err(AppError::NotFound(trf(
            locale,
            "error.resource_not_found",
            &[("value", path.to_string())],
        )));
    }
    let meta = read_album_meta(&album_dir).await;
    if !meta.shares.iter().any(|s| s.username == username) {
        return Err(AppError::Forbidden(tr(locale, "error.no_album_access")));
    }
    Ok(())
}

/// Resolved album access info for sharing-aware endpoints.
struct ResolvedAlbum {
    owner: String,
    root_path: PathBuf,
    album_name: String,
    can_write: bool,
    can_delete: bool,
    is_owner: bool,
}

async fn resolve_album_access(
    cfg: &AppState,
    username: &str,
    user_root: &FsPath,
    album_input: &str,
    locale: &str,
) -> Result<ResolvedAlbum, AppError> {
    // Try "owner/album_name" format for shared albums
    if let Some((owner, album_name)) = album_input.split_once('/') {
        if owner != username {
            let Some(album_name) = sanitize_album_name(album_name) else {
                return Err(AppError::BadRequest(tr(locale, "error.invalid_album_name")));
            };
            let owner_root = cfg.user_root_path(owner);
            let album_dir = albums_root(&owner_root).join(&album_name);
            if !album_dir.exists() {
                return Err(AppError::NotFound(trf(
                    locale,
                    "error.resource_not_found",
                    &[("value", album_input.to_string())],
                )));
            }
            let meta = read_album_meta(&album_dir).await;
            let share = meta.shares.iter().find(|s| s.username == username);
            let Some(share) = share else {
                return Err(AppError::Forbidden(tr(locale, "error.album_access_denied")));
            };
            return Ok(ResolvedAlbum {
                owner: owner.to_string(),
                root_path: owner_root,
                album_name,
                can_write: share.can_write,
                can_delete: share.can_delete,
                is_owner: false,
            });
        }
    }
    // Own album
    let Some(album_name) = sanitize_album_name(album_input) else {
        return Err(AppError::BadRequest(tr(locale, "error.invalid_album_name")));
    };
    Ok(ResolvedAlbum {
        owner: username.to_string(),
        root_path: user_root.to_path_buf(),
        album_name,
        can_write: true,
        can_delete: true,
        is_owner: true,
    })
}

// ── Dashboard helpers ────────────────────────────────────────────────────────

struct AlbumSource {
    dir_name: String,
    owner: String,
    owner_root: PathBuf,
    display_name: String,
    cover: String,
    description: String,
}

/// Gather every album accessible by `username`: own albums + albums shared to them.
async fn collect_accessible_album_sources(cfg: &AppState, username: &str) -> Vec<AlbumSource> {
    let mut sources = Vec::new();
    let user_root = cfg.user_root_path(username);

    // Own albums
    let albums_dir = albums_root(&user_root);
    if let Ok(mut entries) = tokio::fs::read_dir(&albums_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.metadata().await.map(|m| m.is_dir()).unwrap_or(false) {
                let dir_name = entry.file_name().to_string_lossy().to_string();
                let meta = read_album_meta(&entry.path()).await;
                let display_name = if meta.name.is_empty() {
                    dir_name.clone()
                } else {
                    meta.name
                };
                sources.push(AlbumSource {
                    dir_name,
                    owner: username.to_string(),
                    owner_root: user_root.clone(),
                    display_name,
                    cover: meta.cover,
                    description: meta.description,
                });
            }
        }
    }

    // Shared albums (from other users)
    let all_users = cfg.user_manager.list_usernames().await;
    for owner in &all_users {
        if owner == username {
            continue;
        }
        let owner_root = cfg.user_root_path(owner);
        let albums_dir = albums_root(&owner_root);
        if let Ok(mut entries) = tokio::fs::read_dir(&albums_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.metadata().await.map(|m| m.is_dir()).unwrap_or(false) {
                    let dir_name = entry.file_name().to_string_lossy().to_string();
                    let meta = read_album_meta(&entry.path()).await;
                    if !meta.shares.iter().any(|s| s.username == username) {
                        continue;
                    }
                    let display_name = if meta.name.is_empty() {
                        dir_name.clone()
                    } else {
                        meta.name
                    };
                    sources.push(AlbumSource {
                        dir_name,
                        owner: owner.clone(),
                        owner_root: owner_root.clone(),
                        display_name,
                        cover: meta.cover,
                        description: meta.description,
                    });
                }
            }
        }
    }

    sources
}

pub async fn api_dashboard(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let username = session.username;
    let sources = collect_accessible_album_sources(&cfg, &username).await;
    let own_album_count = sources.iter().filter(|source| source.owner == username).count();
    let shared_album_count = sources.len().saturating_sub(own_album_count);

    let now = chrono::Local::now();
    let today = now.format("%Y-%m-%d").to_string();
    let today_md = now.format("%m-%d").to_string();
    let mut accessible_photo_count = 0usize;
    let mut total_bytes = 0u64;
    let mut dated_photo_count = 0usize;
    let mut photos = Vec::<Value>::new();
    let mut albums = Vec::<(String, Value)>::new();

    for source in sources {
        let album_key = if source.owner == username {
            source.dir_name.clone()
        } else {
            format!("{}/{}", source.owner, source.dir_name)
        };
        let index = load_or_queue_album_index(
            cfg.as_ref(),
            &username,
            &source.owner_root,
            &source.owner,
            &source.dir_name,
            locale.as_str(),
        )
        .await?
        .unwrap_or(AlbumIndex {
            version: 2,
            entries: Vec::new(),
        });

        accessible_photo_count += index.entries.len();
        total_bytes += index.entries.iter().map(|entry| entry.size).sum::<u64>();
        dated_photo_count += index
            .entries
            .iter()
            .filter(|entry| !entry.date.is_empty() && entry.date != "1970-01-01")
            .count();

        let latest_date = index
            .entries
            .iter()
            .map(|entry| entry.date.as_str())
            .max()
            .unwrap_or("")
            .to_string();
        albums.push((
            latest_date.clone(),
            json!({
                "album": album_key,
                "dir_name": source.dir_name,
                "name": source.display_name,
                "owner": source.owner,
                "cover": source.cover,
                "description": source.description,
                "photo_count": index.entries.len(),
                "latest_date": latest_date,
            }),
        ));

        for entry in index.entries {
            let mut photo = serde_json::to_value(entry).unwrap_or_default();
            if let Some(object) = photo.as_object_mut() {
                object.insert("album".to_string(), json!(&album_key));
                object.insert("album_name".to_string(), json!(&source.display_name));
                object.insert("owner".to_string(), json!(&source.owner));
            }
            photos.push(photo);
        }
    }

    photos.sort_by(|left, right| {
        let left_date = left
            .get("datetime")
            .and_then(Value::as_str)
            .filter(|date| !date.is_empty())
            .or_else(|| left.get("date").and_then(Value::as_str))
            .unwrap_or("");
        let right_date = right
            .get("datetime")
            .and_then(Value::as_str)
            .filter(|date| !date.is_empty())
            .or_else(|| right.get("date").and_then(Value::as_str))
            .unwrap_or("");
        right_date.cmp(left_date).then_with(|| {
            left.get("path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .cmp(right.get("path").and_then(Value::as_str).unwrap_or(""))
        })
    });
    let recent_photos = photos.iter().take(12).cloned().collect::<Vec<_>>();

    albums.sort_by(|left, right| right.0.cmp(&left.0));
    let recent_albums = albums
        .into_iter()
        .take(6)
        .map(|(_, album)| album)
        .collect::<Vec<_>>();

    let on_this_day_photos = photos
        .iter()
        .filter(|photo| {
            photo
                .get("date")
                .and_then(Value::as_str)
                .and_then(|date| date.get(5..))
                == Some(today_md.as_str())
        })
        .take(8)
        .cloned()
        .collect::<Vec<_>>();
    let on_this_day_count = on_this_day_photos.len();

    let seed = format!("{username}|{today}");
    let mut daily_picks = photos.clone();
    daily_picks.sort_by_cached_key(|photo| {
        let path = photo.get("path").and_then(Value::as_str).unwrap_or("");
        let album = photo.get("album").and_then(Value::as_str).unwrap_or("");
        let mut hasher = Sha256::new();
        hasher.update(seed.as_bytes());
        hasher.update(b"|");
        hasher.update(album.as_bytes());
        hasher.update(b"|");
        hasher.update(path.as_bytes());
        let result = hasher.finalize();
        u64::from_le_bytes(result[..8].try_into().unwrap_or([0u8; 8]))
    });
    daily_picks.truncate(8);

    let task_map = cfg.task_mgr.get_task_status_map_for_user(&username).await;
    let mut active_tasks = task_map
        .into_iter()
        .filter_map(|(id, mut task)| {
            let status = task.get("status").and_then(Value::as_u64)?;
            if status > TaskStatus::Paused as u64 {
                return None;
            }
            task.as_object_mut()?.insert("id".to_string(), json!(id));
            Some(task)
        })
        .collect::<Vec<_>>();
    let active_task_count = active_tasks.len();
    active_tasks.sort_by(|left, right| {
        left.get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(right.get("name").and_then(Value::as_str).unwrap_or(""))
    });
    active_tasks.truncate(6);

    let admin = if session.role == super::auth::Role::Admin {
        let settings = cfg.settings_manager.get_all().await;
        Some(json!({
            "user_count": cfg.user_manager.list_usernames().await.len(),
            "webdav_share_count": super::settings::webdav_share_count(&cfg.data_path).await,
            "settings": {
                "allow_registration": settings.get("allow_registration").cloned().unwrap_or(json!(true)),
                "thumbnail_generation_mode": settings.get("thumbnail_generation_mode").cloned().unwrap_or(json!("on_demand")),
                "thumbnail_concurrency": settings.get("thumbnail_concurrency").cloned().unwrap_or(json!(3)),
                "darktable_concurrency": settings.get("darktable_concurrency").cloned().unwrap_or(json!(1)),
            }
        }))
    } else {
        None
    };

    let data = json!({
        "summary": {
            "own_album_count": own_album_count,
            "shared_album_count": shared_album_count,
            "accessible_photo_count": accessible_photo_count,
            "total_bytes": total_bytes,
            "dated_photo_count": dated_photo_count,
            "active_task_count": active_task_count,
            "uptime_seconds": cfg.started_at.elapsed().as_secs(),
        },
        "recent_photos": recent_photos,
        "recent_albums": recent_albums,
        "on_this_day": {
            "date": today_md,
            "count": on_this_day_count,
            "photos": on_this_day_photos,
        },
        "daily_picks": {
            "date": today,
            "photos": daily_picks,
        },
        "active_tasks": active_tasks,
        "admin": admin,
    });
    Ok(create_response(
        true,
        &tr(locale.as_str(), "response.success"),
        0,
        Some(data),
    ))
}

// ── On This Day ──────────────────────────────────────────────────────────────

pub async fn api_on_this_day(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some((username, _)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    use chrono::Local;
    use std::collections::BTreeMap;

    let now = Local::now();
    let today_md = format!("{:02}-{:02}", now.month(), now.day());

    let sources = collect_accessible_album_sources(&cfg, &username).await;
    let mut by_year: BTreeMap<String, Vec<Value>> = BTreeMap::new();

    for src in sources {
        let Ok(Some(index)) = load_or_queue_album_index(
            cfg.as_ref(),
            &username,
            &src.owner_root,
            &src.owner,
            &src.dir_name,
            locale.as_str(),
        )
        .await
        else {
            continue;
        };

        // album key: "dir_name" for own, "owner/dir_name" for shared
        let album_key = if src.owner == username {
            src.dir_name.clone()
        } else {
            format!("{}/{}", src.owner, src.dir_name)
        };

        for entry in &index.entries {
            let mut entry_val = serde_json::to_value(entry).unwrap_or_default();
            let date_str = entry_val
                .get("date")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string();
            if date_str.len() != 10 {
                continue;
            }
            let entry_md = &date_str[5..]; // "MM-DD"
            if entry_md != today_md {
                continue;
            }
            let year = date_str[..4].to_string();
            if let Some(obj) = entry_val.as_object_mut() {
                obj.insert("album".to_string(), json!(&album_key));
                obj.insert("album_name".to_string(), json!(&src.display_name));
                obj.insert("owner".to_string(), json!(&src.owner));
            }
            by_year.entry(year).or_default().push(entry_val);
        }
    }

    let groups: Vec<Value> = by_year
        .into_iter()
        .rev()
        .map(|(year, photos)| json!({ "year": year, "photos": photos }))
        .collect();

    Ok(Json(json!({
        "ok": true,
        "date": format!("{:02}-{:02}", now.month(), now.day()),
        "groups": groups,
    })))
}

// ── Daily Picks ──────────────────────────────────────────────────────────────

pub async fn api_daily_picks(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some((username, _)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    use chrono::Local;

    let today = Local::now().format("%Y-%m-%d").to_string();
    let sources = collect_accessible_album_sources(&cfg, &username).await;

    let mut all_photos: Vec<Value> = Vec::new();
    for src in sources {
        let Ok(Some(index)) = load_or_queue_album_index(
            cfg.as_ref(),
            &username,
            &src.owner_root,
            &src.owner,
            &src.dir_name,
            locale.as_str(),
        )
        .await
        else {
            continue;
        };
        let album_key = if src.owner == username {
            src.dir_name.clone()
        } else {
            format!("{}/{}", src.owner, src.dir_name)
        };
        for entry in &index.entries {
            let mut item = serde_json::to_value(entry).unwrap_or_default();
            if let Some(obj) = item.as_object_mut() {
                obj.insert("album".to_string(), json!(&album_key));
                obj.insert("album_name".to_string(), json!(&src.display_name));
                obj.insert("owner".to_string(), json!(&src.owner));
            }
            all_photos.push(item);
        }
    }

    // Stable daily shuffle: sort by SHA-256(seed | album | path), take top 24.
    let seed = format!("{}|{}", username, today);
    all_photos.sort_by_cached_key(|p| {
        let path = p.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let album = p.get("album").and_then(|v| v.as_str()).unwrap_or("");
        let mut h = Sha256::new();
        h.update(seed.as_bytes());
        h.update(b"|");
        h.update(album.as_bytes());
        h.update(b"|");
        h.update(path.as_bytes());
        let result = h.finalize();
        u64::from_le_bytes(result[..8].try_into().unwrap_or([0u8; 8]))
    });
    let photos: Vec<Value> = all_photos.into_iter().take(24).collect();

    Ok(Json(json!({
        "ok": true,
        "date": today,
        "photos": photos,
    })))
}

/// Collect albums shared with `username` by scanning all other users' album dirs.
async fn collect_shared_albums(cfg: &AppState, username: &str, locale: &str) -> Vec<Value> {
    struct Candidate {
        owner: String,
        owner_root: PathBuf,
        dir_name: String,
        display_name: String,
        cover: String,
        description: String,
        can_write: bool,
        can_delete: bool,
    }

    // Phase 1 (sequential, cheap metadata reads): collect albums where username has access.
    let all_users = cfg.user_manager.list_usernames().await;
    let mut candidates: Vec<Candidate> = Vec::new();
    for owner in &all_users {
        if owner == username {
            continue;
        }
        let owner_root = cfg.user_root_path(owner);
        let albums_dir = albums_root(&owner_root);
        let Ok(mut entries) = tokio::fs::read_dir(&albums_dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(ft) = entry.metadata().await else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().to_string();
            let meta = read_album_meta(&entry.path()).await;
            let Some(share) = meta.shares.iter().find(|s| s.username == username) else {
                continue;
            };
            let display_name = if meta.name.is_empty() {
                dir_name.clone()
            } else {
                meta.name.clone()
            };
            candidates.push(Candidate {
                owner: owner.clone(),
                owner_root: owner_root.clone(),
                dir_name,
                display_name,
                cover: meta.cover,
                description: meta.description,
                can_write: share.can_write,
                can_delete: share.can_delete,
            });
        }
    }

    // Indexes are queued in the background when absent, so listing shares
    // remains responsive instead of launching synchronous EXIF scans.
    let mut shared_albums = Vec::with_capacity(candidates.len());
    for c in candidates {
        let index =
            load_or_queue_album_index(cfg, username, &c.owner_root, &c.owner, &c.dir_name, locale)
                .await
                .ok()
                .flatten();
        let photo_count = index.map(|i| i.entries.len()).unwrap_or(0);
        shared_albums.push(json!({
            "dir_name": format!("{}/{}", c.owner, c.dir_name),
            "name": c.display_name,
            "cover": c.cover,
            "description": c.description,
            "photo_count": photo_count,
            "owner": c.owner,
            "can_write": c.can_write,
            "can_delete": c.can_delete,
        }));
    }
    shared_albums
}

#[derive(Debug, Deserialize)]
pub struct ShareAlbumPayload {
    dir_name: String,
    username: String,
    #[serde(default)]
    can_write: bool,
    #[serde(default)]
    can_delete: bool,
}

#[named]
pub async fn api_albums_share(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<ShareAlbumPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(dir_name) = sanitize_album_name(&payload.dir_name) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_album_identifier",
        )));
    };
    let target_user = payload.username.trim();
    if target_user == username {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.share_self",
        )));
    }
    if !cfg.user_manager.user_exists(target_user).await {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.target_user_not_found",
        )));
    }

    let album_dir = albums_root(root_path.as_path()).join(&dir_name);
    if !album_dir.exists() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", dir_name.clone())],
        )));
    }

    let mut meta = read_album_meta(album_dir.as_path()).await;
    if let Some(existing) = meta.shares.iter_mut().find(|s| s.username == target_user) {
        existing.can_write = payload.can_write;
        existing.can_delete = payload.can_delete;
    } else {
        meta.shares.push(AlbumShare {
            username: target_user.to_string(),
            can_write: payload.can_write,
            can_delete: payload.can_delete,
        });
    }
    write_album_meta(album_dir.as_path(), &meta).await?;

    let message = tr(locale.as_str(), "response.album_shared");
    Ok(create_response(true, &message, 0, None))
}

#[derive(Debug, Deserialize)]
pub struct UnshareAlbumPayload {
    dir_name: String,
    username: String,
}

#[named]
pub async fn api_albums_unshare(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<UnshareAlbumPayload>,
) -> Result<Json<Value>, AppError> {
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(dir_name) = sanitize_album_name(&payload.dir_name) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_album_identifier",
        )));
    };

    let album_dir = albums_root(root_path.as_path()).join(&dir_name);
    if !album_dir.exists() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", dir_name.clone())],
        )));
    }

    let mut meta = read_album_meta(album_dir.as_path()).await;
    let target_user = payload.username.trim();
    meta.shares.retain(|s| s.username != target_user);
    write_album_meta(album_dir.as_path(), &meta).await?;

    let message = tr(locale.as_str(), "response.album_unshared");
    Ok(create_response(true, &message, 0, None))
}

// ── Album local-upload ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AlbumUploadFileMeta {
    pub bytes: u64,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct AlbumUploadPayload {
    album: String,
    files: Vec<AlbumUploadFileMeta>,
    #[serde(default)]
    conflict: super::files::ConflictAction,
}

/// Returns the two canonical XMP sidecar names for an image file:
/// `{stem}.xmp` (Lightroom style) and `{filename}.xmp` (Darktable style).
fn possible_xmp_names(image_name: &str) -> [String; 2] {
    let stem = FsPath::new(image_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    [format!("{stem}.xmp"), format!("{image_name}.xmp")]
}

#[named]
pub async fn api_albums_upload(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<AlbumUploadPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(album_name) = sanitize_album_name(&payload.album) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_album_identifier",
        )));
    };
    if payload.files.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.no_photos_selected",
        )));
    }
    let conflict = payload.conflict;

    let staging_uuid = Uuid::new_v4();
    let staging_dir = root_path
        .join("albums")
        .join("__uploading")
        .join(staging_uuid.to_string());

    // Build XMP pairing:
    //   image_to_xmp: image name -> Option<xmp name that is also in the batch>
    //   xmp_to_image: xmp name  -> image name it belongs to
    let all_names: HashSet<&str> = payload.files.iter().map(|f| f.name.as_str()).collect();
    let mut image_to_xmp: HashMap<String, Option<String>> = HashMap::new();
    let mut xmp_to_image: HashMap<String, String> = HashMap::new();
    for f in &payload.files {
        if f.name.to_ascii_lowercase().ends_with(".xmp") {
            continue;
        }
        let matched = possible_xmp_names(&f.name)
            .into_iter()
            .find(|x| all_names.contains(x.as_str()));
        if let Some(ref xmp_name) = matched {
            xmp_to_image.insert(xmp_name.clone(), f.name.clone());
        }
        image_to_xmp.insert(f.name.clone(), matched);
    }

    // Build UploadFileMeta list — absolute host paths inside the staging dir.
    let metas: Vec<super::files::UploadFileMeta> = payload
        .files
        .iter()
        .filter(|f| !f.name.is_empty() && !f.name.contains('/') && !f.name.contains('\\'))
        .map(|f| super::files::UploadFileMeta {
            bytes: f.bytes,
            is_dir: false,
            path: staging_dir.join(&f.name).to_string_lossy().to_string(),
        })
        .collect();

    if metas.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.no_photos_selected",
        )));
    }

    let task_name = trf(
        locale.as_str(),
        "task.upload_album",
        &[("album", album_name.clone())],
    );

    // Capture everything the callback needs.
    let cfg_cb = cfg.clone();
    let root_path_cb = root_path.clone();
    let staging_dir_cb = staging_dir.clone();
    let username_cb = username.clone();
    let album_name_cb = album_name.clone();
    let conflict_cb = conflict;
    let image_to_xmp = Arc::new(image_to_xmp);
    let xmp_to_image = Arc::new(xmp_to_image);

    let on_single_finished = move |finished_path: PathBuf| -> BoxFuture<'static, ()> {
        let cfg = cfg_cb.clone();
        let root_path = root_path_cb.clone();
        let staging_dir = staging_dir_cb.clone();
        let username = username_cb.clone();
        let album_name = album_name_cb.clone();
        let conflict_action = conflict_cb;
        let image_to_xmp = image_to_xmp.clone();
        let xmp_to_image = xmp_to_image.clone();

        Box::pin(async move {
            let filename = finished_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            // Determine which image is ready to import.
            let image_path: Option<PathBuf> = if filename.to_ascii_lowercase().ends_with(".xmp") {
                // XMP finished — import its paired image if it is already on disk.
                xmp_to_image
                    .get(&filename)
                    .map(|img_name| staging_dir.join(img_name))
                    .filter(|p| p.exists())
            } else {
                // Image finished — import now unless we're waiting for its XMP.
                match image_to_xmp.get(&filename) {
                    Some(Some(xmp_name)) if !staging_dir.join(xmp_name).exists() => {
                        None // XMP expected but not yet uploaded
                    }
                    _ => Some(finished_path.clone()),
                }
            };

            if let Some(img) = image_path {
                if let Ok(rel) = img.strip_prefix(&root_path) {
                    let _ = import_album_files(
                        &cfg,
                        root_path.as_path(),
                        &username,
                        &album_name,
                        &[rel.to_string_lossy().to_string()],
                        true,
                        conflict_action,
                        None,
                    )
                    .await;
                }

                // Remove staging dir once it is fully drained.
                if let Ok(mut rd) = tokio::fs::read_dir(&staging_dir).await {
                    if rd.next_entry().await.ok().flatten().is_none() {
                        let _ = tokio::fs::remove_dir_all(&staging_dir).await;
                    }
                }
            }
        })
    };

    let task = Arc::new(
        super::files::UploadFiles::new(task_name, username, root_path.clone(), metas, conflict)
            .with_on_single_finished(on_single_finished),
    );
    cfg.task_mgr.add_task(task.clone()).await;

    let message = tr(locale.as_str(), "response.upload_started");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "task_id": task.uuid().to_string(),
            "staging_prefix": format!("albums/__uploading/{}", staging_uuid)
        })),
    ))
}
