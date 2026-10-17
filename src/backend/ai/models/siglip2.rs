//! SigLIP 2 runtime backed by burn-onnx generated vision/text models.
//!
//! burn-onnx exported these graphs with **batch size fixed to 1**. Always run
//! one sample per forward to avoid seq-len / position-embed mismatches.
//!
//! Vision and text are separate weights so each can load and unload on its own.
//! Vision runs on `selected_ai_device` (Vulkan/CUDA). Text — including the
//! 256k×768 token embedding gather — always runs on CPU Flex (`cpu_flex_device`).

use burn::prelude::*;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use tokenizers::Tokenizer;

use super::backend::{AiBackend, AiDevice, cpu_flex_device, selected_ai_device};
use super::manifest::ModelArtifactSpec;
use super::preprocess::prepare_siglip_tensor;
use crate::backend::ai::embedding::{
    SIGLIP2_EMBEDDING_DIM, embedding_debug_fingerprint, embedding_debug_query_preview, l2_normalize,
};

pub const SIGLIP2_IMAGE_SIZE: u32 = 256;
pub const SIGLIP2_MAX_TEXT_LEN: usize = 64;
pub const SIGLIP2_REVISION: &str = "siglip2-base-patch32-256";

pub struct Siglip2Vision {
    #[cfg(ai_onnx_generated)]
    model: super::onnx_gen::siglip2_vision::Model<AiBackend>,
    device: AiDevice,
    pub dimension: usize,
}

pub struct Siglip2Text {
    #[cfg(ai_onnx_generated)]
    model: super::onnx_gen::siglip2_text::Model<AiBackend>,
    tokenizer: Tokenizer,
    /// Text tower + embedding table; always Flex CPU (wgpu cannot bind 750 MiB).
    text_device: AiDevice,
    pub dimension: usize,
    pub max_text_len: usize,
}

impl Siglip2Vision {
    pub fn load(root: &Path, image_text: &ModelArtifactSpec) -> Result<Self, String> {
        #[cfg(not(ai_onnx_generated))]
        {
            let _ = (root, image_text);
            return Err(
                "AI ONNX models were not converted at build time (missing dist/models/onnx)"
                    .to_string(),
            );
        }

        #[cfg(ai_onnx_generated)]
        {
            let choice = selected_ai_device();
            let device = choice.device.clone();
            let vision_bpk = resolve_bpk(root, &["siglip2_vision.bpk"])?;
            let model =
                super::onnx_gen::siglip2_vision::Model::<AiBackend>::from_file(vision_bpk, &device);
            eprintln!("[ai] siglip2 vision device={} ({device:?})", choice.label);
            Ok(Self {
                model,
                device,
                dimension: embedding_dim(image_text),
            })
        }
    }

    pub fn embed_images(&self, images: &[image::DynamicImage]) -> Vec<Result<Vec<f32>, String>> {
        #[cfg(not(ai_onnx_generated))]
        {
            let _ = images;
            return images
                .iter()
                .map(|_| Err("siglip2 onnx model unavailable".to_string()))
                .collect();
        }
        #[cfg(ai_onnx_generated)]
        {
            images
                .iter()
                .map(|image| self.embed_image_one(image))
                .collect()
        }
    }

    #[cfg(ai_onnx_generated)]
    fn embed_image_one(&self, image: &image::DynamicImage) -> Result<Vec<f32>, String> {
        let tensor = prepare_siglip_tensor(image, SIGLIP2_IMAGE_SIZE, &self.device);
        let output =
            catch_unwind(AssertUnwindSafe(|| self.model.forward(tensor))).map_err(|_| {
                "siglip2 vision forward panicked (check input size 256x256, batch=1)".to_string()
            })?;
        let mut rows = tensor_rows_to_normalized_vectors(output, self.dimension)?;
        rows.pop()
            .ok_or_else(|| "siglip2 vision empty output".to_string())
    }
}

impl Siglip2Text {
    pub fn load(
        root: &Path,
        image_text: &ModelArtifactSpec,
        tokenizer_path: Option<&Path>,
        max_text_len: usize,
    ) -> Result<Self, String> {
        #[cfg(not(ai_onnx_generated))]
        {
            let _ = (root, image_text, tokenizer_path, max_text_len);
            return Err(
                "AI ONNX models were not converted at build time (missing dist/models/onnx)"
                    .to_string(),
            );
        }

        #[cfg(ai_onnx_generated)]
        {
            let tokenizer_path =
                tokenizer_path.ok_or_else(|| "siglip2 tokenizer missing".to_string())?;
            let tokenizer = Tokenizer::from_file(tokenizer_path)
                .map_err(|error| format!("load siglip2 tokenizer: {error}"))?;
            // Token embedding gather must not run on Vulkan: the table is 750 MiB
            // and exceeds `max_storage_buffer_binding_size` (128 MiB on Intel iGPU).
            let text_device = cpu_flex_device();
            let text_bpk = resolve_bpk(root, &["siglip2_text.bpk"])?;
            let model = super::onnx_gen::siglip2_text::Model::<AiBackend>::from_file(
                text_bpk,
                &text_device,
            );
            eprintln!("[ai] siglip2 text device=cpu-flex (embedding gather)");
            Ok(Self {
                model,
                tokenizer,
                text_device,
                dimension: embedding_dim(image_text),
                max_text_len: max_text_len.clamp(8, SIGLIP2_MAX_TEXT_LEN),
            })
        }
    }

    pub fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        #[cfg(not(ai_onnx_generated))]
        {
            let _ = texts;
            return Err("siglip2 onnx model unavailable".to_string());
        }
        #[cfg(ai_onnx_generated)]
        {
            let mut out = Vec::with_capacity(texts.len());
            for text in texts {
                out.push(self.embed_text_one(text)?);
            }
            Ok(out)
        }
    }

    #[cfg(ai_onnx_generated)]
    fn embed_text_one(&self, text: &str) -> Result<Vec<f32>, String> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|error| format!("tokenize: {error}"))?;
        let mut token_ids = encoding.get_ids().to_vec();
        if token_ids.len() > self.max_text_len {
            token_ids.truncate(self.max_text_len);
        }
        let pad_id = self.tokenizer.token_to_id("<pad>").unwrap_or(0);
        while token_ids.len() < self.max_text_len {
            token_ids.push(pad_id);
        }
        let head_ids: Vec<u32> = token_ids.iter().copied().take(8).collect();
        let ids: Vec<i32> = token_ids.into_iter().map(|v| v as i32).collect();
        // `text_device` is Flex CPU so gather hits host RAM, not a 750 MiB Vulkan bind.
        let token_tensor =
            Tensor::<AiBackend, 1, Int>::from_ints(ids.as_slice(), &self.text_device)
                .reshape([1, self.max_text_len]);
        let output = catch_unwind(AssertUnwindSafe(|| self.model.forward(token_tensor)))
            .map_err(|_| "siglip2 text forward panicked".to_string())?;
        let mut rows = tensor_rows_to_normalized_vectors(output, self.dimension)?;
        let vector = rows
            .pop()
            .ok_or_else(|| "siglip2 text empty output".to_string())?;
        eprintln!(
            "[ai] text embed device=cpu q={:?} tokens={} pad_id={pad_id} head_ids={head_ids:?} {}",
            embedding_debug_query_preview(text),
            ids.len(),
            embedding_debug_fingerprint(&vector),
        );
        Ok(vector)
    }
}

fn embedding_dim(image_text: &ModelArtifactSpec) -> usize {
    if image_text.dimension > 0 {
        image_text.dimension
    } else {
        SIGLIP2_EMBEDDING_DIM
    }
}

fn resolve_bpk(root: &Path, names: &[&str]) -> Result<std::path::PathBuf, String> {
    for name in names {
        let path = root.join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    for name in names {
        let path = Path::new(env!("OUT_DIR")).join("model").join(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(format!(
        "burnpack missing under {} (tried {names:?})",
        root.display()
    ))
}

#[cfg(ai_onnx_generated)]
fn tensor_rows_to_normalized_vectors(
    tensor: Tensor<AiBackend, 2>,
    dim: usize,
) -> Result<Vec<Vec<f32>>, String> {
    let [batch, width] = tensor.dims();
    let data = tensor
        .into_data()
        .to_vec::<f32>()
        .map_err(|error| format!("siglip2 output download: {error:?}"))?;
    let mut out = Vec::with_capacity(batch);
    for row in 0..batch {
        let start = row * width;
        let mut vector = data[start..start + width].to_vec();
        if vector.len() != dim {
            vector.resize(dim, 0.0);
        }
        l2_normalize(&mut vector)?;
        out.push(vector);
    }
    Ok(out)
}
