//! SigLIP 2 runtime backed by burn-onnx generated vision/text models.
//!
//! burn-onnx exported these graphs with **batch size fixed to 1**. Always run
//! one sample per forward to avoid seq-len / position-embed mismatches.

use burn::prelude::*;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use tokenizers::Tokenizer;

use super::backend::{AiBackend, AiDevice, selected_ai_device};
use super::manifest::ModelArtifactSpec;
use super::preprocess::prepare_siglip_tensor;
use crate::backend::ai::embedding::{SIGLIP2_EMBEDDING_DIM, l2_normalize};

pub const SIGLIP2_IMAGE_SIZE: u32 = 256;
pub const SIGLIP2_MAX_TEXT_LEN: usize = 64;
pub const SIGLIP2_REVISION: &str = "siglip2-base-patch32-256";

pub struct Siglip2Runtime {
    #[cfg(ai_onnx_generated)]
    vision: super::onnx_gen::siglip2_vision::Model<AiBackend>,
    #[cfg(ai_onnx_generated)]
    text: super::onnx_gen::siglip2_text::Model<AiBackend>,
    tokenizer: Option<Tokenizer>,
    device: AiDevice,
    pub dimension: usize,
    pub revision: String,
    pub max_text_len: usize,
}

impl Siglip2Runtime {
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
            let choice = selected_ai_device();
            let device = choice.device.clone();
            let vision_bpk = resolve_bpk(root, &["siglip2_vision.bpk"])?;
            let text_bpk = resolve_bpk(root, &["siglip2_text.bpk"])?;
            let vision =
                super::onnx_gen::siglip2_vision::Model::<AiBackend>::from_file(vision_bpk, &device);
            let text =
                super::onnx_gen::siglip2_text::Model::<AiBackend>::from_file(text_bpk, &device);
            let tokenizer = match tokenizer_path {
                Some(path) => Some(
                    Tokenizer::from_file(path)
                        .map_err(|error| format!("load siglip2 tokenizer: {error}"))?,
                ),
                None => None,
            };
            let dimension = if image_text.dimension > 0 {
                image_text.dimension
            } else {
                SIGLIP2_EMBEDDING_DIM
            };
            Ok(Self {
                vision,
                text,
                tokenizer,
                device,
                dimension,
                revision: image_text.revision.clone(),
                max_text_len: max_text_len.clamp(8, SIGLIP2_MAX_TEXT_LEN),
            })
        }
    }

    pub fn embed_images(&self, images: &[image::DynamicImage]) -> Result<Vec<Vec<f32>>, String> {
        #[cfg(not(ai_onnx_generated))]
        {
            let _ = images;
            return Err("siglip2 onnx model unavailable".to_string());
        }
        #[cfg(ai_onnx_generated)]
        {
            let mut out = Vec::with_capacity(images.len());
            for image in images {
                out.push(self.embed_image_one(image)?);
            }
            Ok(out)
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
    fn embed_image_one(&self, image: &image::DynamicImage) -> Result<Vec<f32>, String> {
        let tensor = prepare_siglip_tensor(image, SIGLIP2_IMAGE_SIZE, &self.device);
        let output =
            catch_unwind(AssertUnwindSafe(|| self.vision.forward(tensor))).map_err(|_| {
                "siglip2 vision forward panicked (check input size 256x256, batch=1)".to_string()
            })?;
        let mut rows = tensor_rows_to_normalized_vectors(output, self.dimension)?;
        rows.pop()
            .ok_or_else(|| "siglip2 vision empty output".to_string())
    }

    #[cfg(ai_onnx_generated)]
    fn embed_text_one(&self, text: &str) -> Result<Vec<f32>, String> {
        let tokenizer = self
            .tokenizer
            .as_ref()
            .ok_or_else(|| "siglip2 tokenizer missing".to_string())?;
        let encoding = tokenizer
            .encode(text, true)
            .map_err(|error| format!("tokenize: {error}"))?;
        let mut token_ids = encoding.get_ids().to_vec();
        if token_ids.len() > self.max_text_len {
            token_ids.truncate(self.max_text_len);
        }
        let pad_id = tokenizer.token_to_id("<pad>").unwrap_or(0);
        while token_ids.len() < self.max_text_len {
            token_ids.push(pad_id);
        }
        let ids: Vec<i32> = token_ids.into_iter().map(|v| v as i32).collect();
        let token_tensor = Tensor::<AiBackend, 1, Int>::from_ints(ids.as_slice(), &self.device)
            .reshape([1, self.max_text_len]);
        let output = catch_unwind(AssertUnwindSafe(|| self.text.forward(token_tensor)))
            .map_err(|_| "siglip2 text forward panicked".to_string())?;
        let mut rows = tensor_rows_to_normalized_vectors(output, self.dimension)?;
        rows.pop()
            .ok_or_else(|| "siglip2 text empty output".to_string())
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
