//! Unified image decode: CPU and GPU backends behind [`ImageDecoder`].

mod auto;
mod cpu;
mod darktable;
mod exif;
mod formats;
mod gpu;

pub use auto::AutoImageDecoder;
pub use cpu::CpuImageDecoder;
pub use darktable::{DarktableImageDecoder, darktable_export, find_xmp_sidecar};
pub use formats::{ImageFormat, ImageFormatSet, format_from_path, is_raw_path};
pub use gpu::GpuDevice;

use std::path::Path;

use async_trait::async_trait;
use image::DynamicImage;

/// Concrete decode backend (fixed implementation; not part of the public feature API).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DecoderBackendKind {
    /// Per-request auto selection (see [`DecoderCreateOptions`]).
    Auto,
    /// Standard raster / HEIC / RAW (LibRaw) on CPU.
    Cpu,
    /// CPU raster decode (GPU upload deferred; no tensor output yet).
    Gpu,
    /// Decode via darktable-cli (RAW demosaic / XMP grading).
    Darktable,
}

/// Options for [`create_decoder`]. Focus on features, not concrete backends.
#[derive(Debug, Clone)]
pub struct DecoderCreateOptions {
    /// Force CPU-only decoding (no GPU preference, no darktable fallback).
    pub(crate) cpu_only: bool,
    /// Force darktable-cli for all supported formats.
    pub(crate) force_darktable: bool,
    /// Optional Burn device; when set in auto mode, raster formats prefer GPU.
    pub(crate) gpu_device: Option<GpuDevice>,
}

impl Default for DecoderCreateOptions {
    fn default() -> Self {
        Self::auto()
    }
}

impl DecoderCreateOptions {
    /// Prefer GPU when available; RAW falls back to darktable on LibRaw failure.
    pub fn auto() -> Self {
        Self {
            cpu_only: false,
            force_darktable: false,
            gpu_device: None,
        }
    }

    /// CPU-only path (LibRaw for RAW; no darktable fallback).
    pub fn cpu() -> Self {
        Self {
            cpu_only: true,
            force_darktable: false,
            gpu_device: None,
        }
    }

    /// Always decode via darktable-cli (thumbnails / XMP preview).
    pub fn darktable() -> Self {
        Self {
            cpu_only: false,
            force_darktable: true,
            gpu_device: None,
        }
    }
}

/// Per-decode options. Set [`Self::format`] explicitly (via [`Self::for_path`]) before decode.
#[derive(Debug, Clone, Copy)]
pub struct DecodeOptions {
    /// Image format. Required for [`ImageDecoder::decode_bytes`]; set via [`Self::for_path`] or [`decode_bytes_with_path`].
    pub format: Option<ImageFormat>,
    /// Apply EXIF orientation when present (recommended for display / face boxes).
    pub apply_exif: bool,
    /// When true, `decode_path` applies an XMP sidecar if present (darktable only).
    pub use_xmp: bool,
    /// Longest output edge in pixels. `0` means full resolution (no downscale).
    pub max_edge: u32,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl DecodeOptions {
    /// Oriented decode without XMP grading, full resolution.
    pub const fn new() -> Self {
        Self {
            format: None,
            apply_exif: true,
            use_xmp: false,
            max_edge: 0,
        }
    }

    /// Oriented decode; if an XMP sidecar exists, apply it via darktable.
    pub const fn with_xmp(mut self) -> Self {
        self.use_xmp = true;
        self
    }

    /// Raw pixels: no EXIF orientation, no XMP, full resolution.
    pub const fn raw_pixels() -> Self {
        Self {
            format: None,
            apply_exif: false,
            use_xmp: false,
            max_edge: 0,
        }
    }

    pub const fn with_max_edge(mut self, max_edge: u32) -> Self {
        self.max_edge = max_edge;
        self
    }

    pub const fn with_format(mut self, format: ImageFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// Build options with format taken from the file extension.
    pub fn for_path(path: &Path) -> Result<Self, DecodeError> {
        Ok(Self::new().with_format(format_from_path(path)?))
    }
}

pub(crate) fn apply_max_edge(image: DynamicImage, max_edge: u32) -> DynamicImage {
    if max_edge == 0 {
        image
    } else {
        image.thumbnail(max_edge, max_edge)
    }
}

#[derive(Debug, Clone)]
pub struct DecodeError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for DecodeError {}

#[async_trait]
pub trait ImageDecoder: Send + Sync {
    fn backend_kind(&self) -> DecoderBackendKind;

    fn supported_formats(&self) -> ImageFormatSet;

    fn is_supported(&self, format: ImageFormat) -> bool {
        format != ImageFormat::Unknown && self.supported_formats().contains(format)
    }

    fn decode_bytes(
        &self,
        bytes: &[u8],
        options: DecodeOptions,
    ) -> Result<DynamicImage, DecodeError>;

    async fn decode_path(
        &self,
        path: &Path,
        options: DecodeOptions,
    ) -> Result<DynamicImage, DecodeError>;
}

pub fn create_decoder(options: DecoderCreateOptions) -> Result<Box<dyn ImageDecoder>, DecodeError> {
    if options.force_darktable {
        Ok(Box::new(DarktableImageDecoder))
    } else if options.cpu_only {
        Ok(Box::new(CpuImageDecoder))
    } else {
        Ok(Box::new(AutoImageDecoder::new(options)))
    }
}

/// Decode image bytes; `path` supplies the format via file extension.
pub fn decode_bytes_with_path(
    decoder: &dyn ImageDecoder,
    path: &Path,
    bytes: &[u8],
    options: DecodeOptions,
) -> Result<DynamicImage, DecodeError> {
    let format = formats::format_from_hint_or_path(options.format, Some(path))?;
    decoder.decode_bytes(
        bytes,
        DecodeOptions {
            format: Some(format),
            ..options
        },
    )
}
