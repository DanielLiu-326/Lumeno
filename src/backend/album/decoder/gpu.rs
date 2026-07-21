use std::path::Path;

use async_trait::async_trait;
use burn::Dispatch;
use burn::DispatchDevice;
use image::DynamicImage;

use super::cpu::CpuImageDecoder;
use super::formats::{resolve_format, ImageFormat, ImageFormatSet};
use super::{apply_max_edge, DecodeError, DecodeOptions, DecoderBackendKind, ImageDecoder};

pub type GpuBackend = Dispatch;
pub type GpuDevice = DispatchDevice;

/// GPU decoder placeholder: raster decode on CPU until tensor output is supported.
#[derive(Debug, Clone)]
pub struct GpuImageDecoder {
    #[allow(dead_code)]
    device: GpuDevice,
}

impl GpuImageDecoder {
    pub const SUPPORTED: ImageFormatSet = ImageFormatSet::RASTER;

    pub fn is_supported(format: ImageFormat) -> bool {
        Self::SUPPORTED.contains(format)
    }

    pub fn new(device: GpuDevice) -> Self {
        Self { device }
    }
}

#[async_trait]
impl ImageDecoder for GpuImageDecoder {
    fn backend_kind(&self) -> DecoderBackendKind {
        DecoderBackendKind::Gpu
    }

    fn supported_formats(&self) -> ImageFormatSet {
        Self::SUPPORTED
    }

    fn decode_bytes(&self, bytes: &[u8], options: DecodeOptions) -> Result<DynamicImage, DecodeError> {
        let format = resolve_format(options.format, None, Some(bytes))?;
        let image = CpuImageDecoder::decode_bytes_inner(bytes, format, options.apply_exif)?;
        Ok(apply_max_edge(image, options.max_edge))
    }

    async fn decode_path(
        &self,
        path: &Path,
        options: DecodeOptions,
    ) -> Result<DynamicImage, DecodeError> {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let bytes = std::fs::read(&path).map_err(|error| DecodeError {
                code: "read_failed",
                message: error.to_string(),
            })?;
            let format = resolve_format(options.format, Some(&path), Some(bytes.as_slice()))?;
            let image = CpuImageDecoder::decode_bytes_inner(&bytes, format, options.apply_exif)?;
            Ok(apply_max_edge(image, options.max_edge))
        })
        .await
        .map_err(|error| DecodeError {
            code: "decode_join_failed",
            message: error.to_string(),
        })?
    }
}
