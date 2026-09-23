use std::path::{Path, PathBuf};

use async_trait::async_trait;
use image::DynamicImage;

use super::exif::apply_exif_from_bytes;
use super::formats::{ImageFormat, ImageFormatSet, format_from_hint_or_path, require_format};
use super::{DecodeError, DecodeOptions, DecoderBackendKind, ImageDecoder, apply_max_edge};

/// Full demosaic / XMP grading via `darktable-cli`. Requires darktable-cli on PATH.
#[derive(Debug, Default, Clone, Copy)]
pub struct DarktableImageDecoder;

impl DarktableImageDecoder {
    pub const SUPPORTED: ImageFormatSet = ImageFormatSet::CAMERA_RAW.union(ImageFormatSet::RASTER);

    pub fn is_supported(format: super::formats::ImageFormat) -> bool {
        Self::SUPPORTED.contains(format)
    }

    fn decode_jpeg_bytes(bytes: &[u8], apply_exif: bool) -> Result<DynamicImage, DecodeError> {
        let img = image::load_from_memory(bytes).map_err(|error| DecodeError {
            code: "decode_failed",
            message: error.to_string(),
        })?;
        Ok(if apply_exif {
            apply_exif_from_bytes(img, bytes)
        } else {
            img
        })
    }

    /// Sync path used by `decode_bytes` RAW fallback (already off the async runtime).
    pub(super) fn decode_path_blocking(
        path: &Path,
        options: DecodeOptions,
    ) -> Result<DynamicImage, DecodeError> {
        let _format = format_from_hint_or_path(options.format, Some(path))?;
        let output = run_darktable_cli_blocking(path, options)?;
        let img = Self::decode_jpeg_bytes(&output, options.apply_exif)?;
        Ok(apply_max_edge(img, options.max_edge))
    }
}

/// `{name}.{ext}.xmp` then `{stem}.xmp`.
pub fn find_xmp_sidecar(source: &Path) -> Option<PathBuf> {
    let xmp1 = source.with_extension(format!(
        "{}.xmp",
        source.extension().unwrap_or_default().to_string_lossy()
    ));
    if xmp1.exists() {
        return Some(xmp1);
    }
    let xmp2 = source.with_extension("xmp");
    if xmp2.exists() {
        return Some(xmp2);
    }
    None
}

fn xmp_for_options(path: &Path, options: DecodeOptions) -> Option<PathBuf> {
    if options.use_xmp {
        find_xmp_sidecar(path)
    } else {
        None
    }
}

fn run_darktable_cli_blocking(path: &Path, options: DecodeOptions) -> Result<Vec<u8>, DecodeError> {
    let xmp = xmp_for_options(path, options);
    let temp_jpg = std::env::temp_dir().join(format!("lumeno-dt-{}.jpg", uuid::Uuid::new_v4()));
    let config_dir =
        std::env::temp_dir().join(format!("darktable-config-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&config_dir).ok();

    let mut cmd = std::process::Command::new("darktable-cli");
    cmd.arg(path);
    if let Some(ref xmp_path) = xmp {
        cmd.arg(xmp_path);
    }
    cmd.arg(&temp_jpg);
    // When max_edge > 0, ask darktable to downscale (avoids full-res RAM for thumbs).
    // apply_max_edge still runs afterward for backends that ignore this.
    if options.max_edge > 0 {
        cmd.arg("--width")
            .arg(options.max_edge.to_string())
            .arg("--height")
            .arg(options.max_edge.to_string());
    }
    let status = cmd
        .arg("--core")
        .arg("--configdir")
        .arg(&config_dir)
        .arg("--library")
        .arg(":memory:")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|error| DecodeError {
            code: "darktable_missing",
            message: format!("darktable-cli not available: {error}"),
        })?;

    let _ = std::fs::remove_dir_all(&config_dir);

    if !status.success() {
        let _ = std::fs::remove_file(&temp_jpg);
        return Err(DecodeError {
            code: "darktable_failed",
            message: format!("darktable-cli failed for {}", path.display()),
        });
    }

    let bytes = std::fs::read(&temp_jpg).map_err(|error| DecodeError {
        code: "darktable_read_failed",
        message: error.to_string(),
    })?;
    let _ = std::fs::remove_file(&temp_jpg);
    Ok(bytes)
}

async fn run_darktable_cli(path: &Path, options: DecodeOptions) -> Result<Vec<u8>, DecodeError> {
    let xmp = xmp_for_options(path, options);
    let temp_jpg = std::env::temp_dir().join(format!("lumeno-dt-{}.jpg", uuid::Uuid::new_v4()));
    let config_dir =
        std::env::temp_dir().join(format!("darktable-config-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&config_dir).await.ok();

    let started = std::time::Instant::now();
    let mut cmd = tokio::process::Command::new("darktable-cli");
    cmd.arg(path);
    if let Some(ref xmp_path) = xmp {
        cmd.arg(xmp_path);
    }
    cmd.arg(&temp_jpg);
    if options.max_edge > 0 {
        cmd.arg("--width")
            .arg(options.max_edge.to_string())
            .arg("--height")
            .arg(options.max_edge.to_string());
    }
    cmd.arg("--core")
        .arg("--configdir")
        .arg(&config_dir)
        .arg("--library")
        .arg(":memory:")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    eprintln!(
        "[darktable] start operation=decode source={}",
        path.display()
    );
    let status = cmd.status().await.map_err(|error| DecodeError {
        code: "darktable_missing",
        message: format!("darktable-cli not available: {error}"),
    })?;
    eprintln!(
        "[darktable] finish operation=decode status={status} elapsed_ms={} source={}",
        started.elapsed().as_millis(),
        path.display()
    );

    let _ = tokio::fs::remove_dir_all(&config_dir).await;

    if !status.success() {
        let _ = tokio::fs::remove_file(&temp_jpg).await;
        return Err(DecodeError {
            code: "darktable_failed",
            message: format!("darktable-cli failed for {}", path.display()),
        });
    }

    let bytes = tokio::fs::read(&temp_jpg)
        .await
        .map_err(|error| DecodeError {
            code: "darktable_read_failed",
            message: error.to_string(),
        })?;
    let _ = tokio::fs::remove_file(&temp_jpg).await;
    Ok(bytes)
}

/// Export via darktable-cli with required XMP, optional max width and JPEG quality.
///
/// Isolated `--configdir` starts with empty `metadata_flags` (no EXIF). Explicitly set
/// darktable's default export metadata flags so EXIF/geo/tags are written into the file:
/// `DT_META_EXIF|METADATA|GEOTAG|TAG|DT_HISTORY` = `0x2f`.
pub async fn darktable_export(
    source: &Path,
    xmp: &Path,
    output: &Path,
    out_ext: &str,
    quality: u32,
    max_width: u32,
) -> Result<(), DecodeError> {
    let config_dir =
        std::env::temp_dir().join(format!("darktable-config-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&config_dir).await.ok();

    let started = std::time::Instant::now();
    let mut cmd = tokio::process::Command::new("darktable-cli");
    cmd.arg(source).arg(xmp).arg(output);
    if max_width > 0 {
        cmd.arg("--width").arg(max_width.to_string());
    }
    cmd.arg("--core")
        .arg("--configdir")
        .arg(&config_dir)
        // Prefer darktable's own metadata export (not post-hoc exiftool).
        .arg("--conf")
        .arg("plugins/lighttable/export/metadata_flags=2f");
    if out_ext == "jpg" {
        cmd.arg("--conf")
            .arg(format!("plugins/imageio/format/jpeg/quality={quality}"));
    }
    cmd.arg("--library")
        .arg(":memory:")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    eprintln!(
        "[darktable] start operation=export source={}",
        source.display()
    );
    let status = cmd.status().await.map_err(|error| DecodeError {
        code: "darktable_missing",
        message: format!("darktable-cli not available: {error}"),
    })?;
    eprintln!(
        "[darktable] finish operation=export status={status} elapsed_ms={} source={}",
        started.elapsed().as_millis(),
        source.display()
    );

    let _ = tokio::fs::remove_dir_all(&config_dir).await;
    if !status.success() {
        let _ = tokio::fs::remove_file(output).await;
        return Err(DecodeError {
            code: "darktable_failed",
            message: "darktable-cli export failed".to_string(),
        });
    }
    Ok(())
}

#[async_trait]
impl ImageDecoder for DarktableImageDecoder {
    fn backend_kind(&self) -> DecoderBackendKind {
        DecoderBackendKind::Darktable
    }

    fn supported_formats(&self) -> ImageFormatSet {
        Self::SUPPORTED
    }

    fn decode_bytes(
        &self,
        bytes: &[u8],
        options: DecodeOptions,
    ) -> Result<DynamicImage, DecodeError> {
        let format = require_format(options.format)?;
        if format == ImageFormat::Jpeg {
            let img = Self::decode_jpeg_bytes(bytes, options.apply_exif)?;
            return Ok(apply_max_edge(img, options.max_edge));
        }
        Err(DecodeError {
            code: "raw_needs_path",
            message: "darktable decode requires a file path".to_string(),
        })
    }

    async fn decode_path(
        &self,
        path: &Path,
        options: DecodeOptions,
    ) -> Result<DynamicImage, DecodeError> {
        let _format = format_from_hint_or_path(options.format, Some(path))?;
        let bytes = run_darktable_cli(path, options).await?;
        let apply_exif = options.apply_exif;
        let max_edge = options.max_edge;
        tokio::task::spawn_blocking(move || {
            let img = Self::decode_jpeg_bytes(&bytes, apply_exif)?;
            // Darktable already honored max_edge via --width/--height when set;
            // apply again only as a safety net for odd aspect / CLI quirks.
            Ok(apply_max_edge(img, max_edge))
        })
        .await
        .map_err(|error| DecodeError {
            code: "decode_join_failed",
            message: error.to_string(),
        })?
    }
}
