use std::path::Path;

use async_trait::async_trait;
use image::DynamicImage;

use super::cpu::{select_backend_for_format, CpuImageDecoder};
use super::darktable::{find_xmp_sidecar, DarktableImageDecoder};
use super::formats::{resolve_format, ImageFormat, ImageFormatSet};
use super::gpu::GpuImageDecoder;
use super::{
    DecodeError, DecodeOptions, DecoderBackendKind, DecoderCreateOptions, ImageDecoder,
};

/// Per-request backend selection for [`DecoderCreateOptions::auto`].
#[derive(Debug, Clone)]
pub struct AutoImageDecoder {
    options: DecoderCreateOptions,
}

impl AutoImageDecoder {
    pub fn new(options: DecoderCreateOptions) -> Self {
        Self { options }
    }

    pub const SUPPORTED: ImageFormatSet = CpuImageDecoder::SUPPORTED;

    pub fn is_supported(format: ImageFormat) -> bool {
        Self::SUPPORTED.contains(format)
    }

    fn decoder_for_format(&self, format: ImageFormat) -> Result<Box<dyn ImageDecoder>, DecodeError> {
        let backend = select_backend_for_format(&self.options, format)?;
        create_concrete_decoder(backend, &self.options)
    }

    fn decode_raw_with_fallback_bytes(
        &self,
        bytes: &[u8],
        options: DecodeOptions,
    ) -> Result<DynamicImage, DecodeError> {
        match CpuImageDecoder.decode_bytes(bytes, options) {
            Ok(decoded) => Ok(decoded),
            Err(cpu_error) => {
                eprintln!(
                    "[decoder] libraw failed ({}); falling back to darktable",
                    cpu_error
                );
                let temp_in =
                    std::env::temp_dir().join(format!("lumeno-raw-in-{}", uuid::Uuid::new_v4()));
                std::fs::write(&temp_in, bytes).map_err(|error| DecodeError {
                    code: "raw_temp_write_failed",
                    message: error.to_string(),
                })?;
                let result = DarktableImageDecoder::decode_path_blocking(&temp_in, options);
                let _ = std::fs::remove_file(&temp_in);
                result.or(Err(cpu_error))
            }
        }
    }

    async fn decode_raw_with_fallback_path(
        &self,
        path: &Path,
        options: DecodeOptions,
    ) -> Result<DynamicImage, DecodeError> {
        match CpuImageDecoder.decode_path(path, options).await {
            Ok(decoded) => Ok(decoded),
            Err(cpu_error) => {
                eprintln!(
                    "[decoder] libraw failed for {} ({}); falling back to darktable",
                    path.display(),
                    cpu_error
                );
                DarktableImageDecoder
                    .decode_path(path, options)
                    .await
                    .or(Err(cpu_error))
            }
        }
    }
}

#[async_trait]
impl ImageDecoder for AutoImageDecoder {
    fn backend_kind(&self) -> DecoderBackendKind {
        DecoderBackendKind::Auto
    }

    fn supported_formats(&self) -> ImageFormatSet {
        Self::SUPPORTED
    }

    fn decode_bytes(&self, bytes: &[u8], options: DecodeOptions) -> Result<DynamicImage, DecodeError> {
        let format = resolve_format(options.format, None, Some(bytes))?;
        let options = DecodeOptions {
            format: Some(format),
            ..options
        };
        if format.is_camera_raw() {
            return self.decode_raw_with_fallback_bytes(bytes, options);
        }
        let decoder = self.decoder_for_format(format)?;
        decoder.decode_bytes(bytes, options)
    }

    async fn decode_path(
        &self,
        path: &Path,
        options: DecodeOptions,
    ) -> Result<DynamicImage, DecodeError> {
        // XMP grading is path-only and only supported via darktable.
        if options.use_xmp && find_xmp_sidecar(path).is_some() {
            return DarktableImageDecoder.decode_path(path, options).await;
        }

        let format = resolve_format(options.format, Some(path), None)?;
        let options = DecodeOptions {
            format: Some(format),
            ..options
        };
        if format.is_camera_raw() {
            return self.decode_raw_with_fallback_path(path, options).await;
        }
        let decoder = self.decoder_for_format(format)?;
        decoder.decode_path(path, options).await
    }
}

fn create_concrete_decoder(
    backend: DecoderBackendKind,
    options: &DecoderCreateOptions,
) -> Result<Box<dyn ImageDecoder>, DecodeError> {
    match backend {
        DecoderBackendKind::Auto => Err(DecodeError {
            code: "invalid_backend",
            message: "Auto backend must use AutoImageDecoder wrapper".to_string(),
        }),
        DecoderBackendKind::Cpu => Ok(Box::new(CpuImageDecoder)),
        DecoderBackendKind::Gpu => Ok(Box::new(GpuImageDecoder::new(
            options
                .gpu_device
                .clone()
                .expect("Gpu decoder requires gpu_device in options"),
        ))),
        DecoderBackendKind::Darktable => Ok(Box::new(DarktableImageDecoder)),
    }
}
