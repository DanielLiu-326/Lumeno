use std::path::Path;
use std::sync::Once;

use async_trait::async_trait;
use image::{DynamicImage, RgbImage};
use rsraw::{RawImage, BIT_DEPTH_8};

use super::exif::apply_exif_from_bytes;
use super::formats::{resolve_format, ImageFormat, ImageFormatSet};
use super::gpu::GpuImageDecoder;
use super::{
    apply_max_edge, DecodeError, DecodeOptions, DecoderBackendKind, DecoderCreateOptions,
    ImageDecoder,
};

static REGISTER_HEIC: Once = Once::new();

fn ensure_heic_decoder_registered() {
    REGISTER_HEIC.call_once(|| {
        libheif_rs::integration::image::register_all_decoding_hooks();
    });
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CpuImageDecoder;

impl CpuImageDecoder {
    pub const SUPPORTED: ImageFormatSet = ImageFormatSet::RASTER
        .union(ImageFormatSet::from_one(ImageFormat::Heic))
        .union(ImageFormatSet::CAMERA_RAW);

    pub fn is_supported(format: ImageFormat) -> bool {
        Self::SUPPORTED.contains(format)
    }

    /// Raster / HEIC decode (used by GPU and darktable post-process).
    pub fn decode_bytes_inner(
        bytes: &[u8],
        format: ImageFormat,
        apply_exif: bool,
    ) -> Result<DynamicImage, DecodeError> {
        if format == ImageFormat::Heic {
            ensure_heic_decoder_registered();
        }
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
}

#[async_trait]
impl ImageDecoder for CpuImageDecoder {
    fn backend_kind(&self) -> DecoderBackendKind {
        DecoderBackendKind::Cpu
    }

    fn supported_formats(&self) -> ImageFormatSet {
        Self::SUPPORTED
    }

    fn decode_bytes(&self, bytes: &[u8], options: DecodeOptions) -> Result<DynamicImage, DecodeError> {
        let format = resolve_format(options.format, None, Some(bytes))?;
        let image = if format.is_camera_raw() {
            decode_libraw_bytes(bytes)?
        } else {
            Self::decode_bytes_inner(bytes, format, options.apply_exif)?
        };
        Ok(apply_max_edge(image, options.max_edge))
    }

    async fn decode_path(
        &self,
        path: &Path,
        options: DecodeOptions,
    ) -> Result<DynamicImage, DecodeError> {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let image = decode_path_blocking(&path, options)?;
            Ok(apply_max_edge(image, options.max_edge))
        })
        .await
        .map_err(|error| DecodeError {
            code: "decode_join_failed",
            message: error.to_string(),
        })?
    }
}

fn decode_path_blocking(path: &Path, options: DecodeOptions) -> Result<DynamicImage, DecodeError> {
    let format = resolve_format(options.format, Some(path), None)?;
    if format.is_camera_raw() {
        return decode_libraw_path(path);
    }
    let bytes = std::fs::read(path).map_err(|error| DecodeError {
        code: "read_failed",
        message: error.to_string(),
    })?;
    CpuImageDecoder::decode_bytes_inner(&bytes, format, options.apply_exif)
}

pub(super) fn select_backend_for_format(
    options: &DecoderCreateOptions,
    format: ImageFormat,
) -> Result<DecoderBackendKind, DecodeError> {
    if options.cpu_only {
        return Ok(DecoderBackendKind::Cpu);
    }

    // Auto: prefer GPU for raster when a device is available; RAW stays on CPU (LibRaw),
    // with darktable fallback handled by AutoImageDecoder.
    Ok(match format {
        format if format.is_camera_raw() => DecoderBackendKind::Cpu,
        _ if options.gpu_device.is_some() && GpuImageDecoder::SUPPORTED.contains(format) => {
            DecoderBackendKind::Gpu
        }
        _ => DecoderBackendKind::Cpu,
    })
}

fn decode_libraw_path(path: &Path) -> Result<DynamicImage, DecodeError> {
    let bytes = std::fs::read(path).map_err(|error| DecodeError {
        code: "read_failed",
        message: error.to_string(),
    })?;
    decode_libraw_bytes(&bytes)
}

fn decode_libraw_bytes(bytes: &[u8]) -> Result<DynamicImage, DecodeError> {
    let mut raw = RawImage::open(bytes).map_err(|error| DecodeError {
        code: "libraw_failed",
        message: error.to_string(),
    })?;
    raw.set_use_camera_wb(true);
    raw.unpack().map_err(|error| DecodeError {
        code: "libraw_failed",
        message: error.to_string(),
    })?;
    let processed = raw
        .process::<BIT_DEPTH_8>()
        .map_err(|error| DecodeError {
            code: "libraw_failed",
            message: error.to_string(),
        })?;
    processed_to_dynamic(&processed)
}

fn processed_to_dynamic(
    processed: &rsraw::ProcessedImage<BIT_DEPTH_8>,
) -> Result<DynamicImage, DecodeError> {
    let width = processed.width();
    let height = processed.height();
    let colors = processed.colors() as usize;
    if colors != 3 {
        return Err(DecodeError {
            code: "libraw_pixel_layout",
            message: format!("unexpected libraw color count: {colors} (expected 3)"),
        });
    }
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(colors))
        .ok_or_else(|| DecodeError {
            code: "libraw_dimensions_overflow",
            message: "libraw output dimensions overflow".to_string(),
        })?;
    let pixels: &[u8] = processed;
    if pixels.len() != expected {
        return Err(DecodeError {
            code: "libraw_pixel_layout",
            message: format!(
                "unexpected libraw RGB buffer size: got {} expected {expected}",
                pixels.len()
            ),
        });
    }
    RgbImage::from_raw(width, height, pixels.to_vec())
        .map(DynamicImage::ImageRgb8)
        .ok_or_else(|| DecodeError {
            code: "libraw_rgb_build_failed",
            message: "failed to build RGB image from libraw output".to_string(),
        })
}
