//! ArcFace runtime backed by burn-onnx generated IResNet50.

use burn::prelude::*;
use image::RgbImage;
use std::path::Path;

use super::backend::{AiBackend, AiDevice, selected_ai_device};
use super::manifest::ModelArtifactSpec;
use super::preprocess::prepare_arcface_tensor;
use crate::backend::ai::embedding::{ARCFACE_EMBEDDING_DIM, l2_normalize};

pub const ARCFACE_INPUT_SIZE: u32 = 112;
pub const ARCFACE_REVISION: &str = "arcface-r50-w600k";

pub struct ArcFaceRuntime {
    #[cfg(ai_onnx_generated)]
    model: super::onnx_gen::arcface_r50::Model<AiBackend>,
    device: AiDevice,
    pub dimension: usize,
    pub revision: String,
}

impl ArcFaceRuntime {
    pub fn load(root: &Path, spec: &ModelArtifactSpec) -> Result<Self, String> {
        #[cfg(not(ai_onnx_generated))]
        {
            let _ = (root, spec);
            return Err("arcface onnx model unavailable at build time".to_string());
        }
        #[cfg(ai_onnx_generated)]
        {
            let device = selected_ai_device().device;
            let bpk = resolve_bpk(root, &["arcface_r50.bpk", "arcface-r50.bpk"])?;
            let model = super::onnx_gen::arcface_r50::Model::<AiBackend>::from_file(bpk, &device);
            Ok(Self {
                model,
                device,
                dimension: if spec.dimension > 0 {
                    spec.dimension
                } else {
                    ARCFACE_EMBEDDING_DIM
                },
                revision: spec.revision.clone(),
            })
        }
    }

    pub fn embed_crop(&self, crop: &RgbImage) -> Result<Vec<f32>, String> {
        #[cfg(not(ai_onnx_generated))]
        {
            let _ = crop;
            return Err("arcface onnx model unavailable".to_string());
        }
        #[cfg(ai_onnx_generated)]
        {
            let tensor = prepare_arcface_tensor(crop, &self.device);
            let output = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.model.forward(tensor)
            }))
            .map_err(|_| "arcface forward panicked (input must be 1x3x112x112)".to_string())?;
            let mut vector = output
                .into_data()
                .to_vec::<f32>()
                .map_err(|error| format!("arcface output: {error:?}"))?;
            if vector.len() != self.dimension {
                vector.resize(self.dimension, 0.0);
            }
            l2_normalize(&mut vector)?;
            Ok(vector)
        }
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
