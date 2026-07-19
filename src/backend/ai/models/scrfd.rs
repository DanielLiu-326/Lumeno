//! SCRFD runtime backed by burn-onnx generated detector.
//!
//! Decode follows InsightFace `model_zoo/scrfd.py`: letterbox preprocess,
//! FPN strides [8,16,32], 2 anchors, distance→bbox/kps (scores already sigmoid).

use burn::prelude::*;
use image::DynamicImage;
use std::path::Path;

use super::backend::{AiBackend, AiDevice, selected_ai_device};
use super::manifest::ModelArtifactSpec;
use super::preprocess::prepare_scrfd_image;

pub const SCRFD_INPUT_SIZE: u32 = 640;
pub const SCRFD_REVISION: &str = "scrfd-buffalo";
pub const SCRFD_MAX_FACES: usize = 32;
const FEAT_STRIDES: [usize; 3] = [8, 16, 32];
const NUM_ANCHORS: usize = 2;

#[derive(Debug, Clone)]
pub struct ScrfdDetection {
    pub box_xywh: [f32; 4],
    pub landmarks: [[f32; 2]; 5],
    pub score: f32,
}

pub struct ScrfdRuntime {
    #[cfg(ai_onnx_generated)]
    model: super::onnx_gen::scrfd::Model<AiBackend>,
    device: AiDevice,
    pub revision: String,
    pub score_threshold: f32,
    pub nms_threshold: f32,
}

impl ScrfdRuntime {
    pub fn load(root: &Path, spec: &ModelArtifactSpec) -> Result<Self, String> {
        #[cfg(not(ai_onnx_generated))]
        {
            let _ = (root, spec);
            return Err("scrfd onnx model unavailable at build time".to_string());
        }
        #[cfg(ai_onnx_generated)]
        {
            let device = selected_ai_device().device;
            let bpk = resolve_bpk(root, &["scrfd.bpk", "scrfd-2.5g.bpk"])?;
            let model = super::onnx_gen::scrfd::Model::<AiBackend>::from_file(bpk, &device);
            Ok(Self {
                model,
                device,
                revision: spec.revision.clone(),
                score_threshold: 0.5,
                nms_threshold: 0.4,
            })
        }
    }

    pub fn detect(&self, image: &DynamicImage) -> Result<Vec<ScrfdDetection>, String> {
        #[cfg(not(ai_onnx_generated))]
        {
            let _ = image;
            return Err("scrfd onnx model unavailable".to_string());
        }
        #[cfg(ai_onnx_generated)]
        {
            let (chw, det_scale, _orig_w, _orig_h) = prepare_scrfd_image(image, SCRFD_INPUT_SIZE);
            let size = SCRFD_INPUT_SIZE as usize;
            let tensor = Tensor::<AiBackend, 1>::from_floats(chw.as_slice(), &self.device)
                .reshape([1, 3, size, size]);
            let outputs = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.model.forward(tensor)
            }))
            .map_err(|_| "scrfd forward panicked (input must be 1x3x640x640)".to_string())?;
            decode_scrfd_outputs(
                outputs,
                SCRFD_INPUT_SIZE as usize,
                SCRFD_INPUT_SIZE as usize,
                det_scale,
                self.score_threshold,
                self.nms_threshold,
            )
        }
    }
}

#[cfg(ai_onnx_generated)]
fn decode_scrfd_outputs(
    outputs: (
        Tensor<AiBackend, 2>,
        Tensor<AiBackend, 2>,
        Tensor<AiBackend, 2>,
        Tensor<AiBackend, 2>,
        Tensor<AiBackend, 2>,
        Tensor<AiBackend, 2>,
        Tensor<AiBackend, 2>,
        Tensor<AiBackend, 2>,
        Tensor<AiBackend, 2>,
    ),
    input_w: usize,
    input_h: usize,
    det_scale: f32,
    score_threshold: f32,
    nms_threshold: f32,
) -> Result<Vec<ScrfdDetection>, String> {
    // burn-onnx / ONNX order: score×3, bbox×3, kps×3 for strides 8,16,32.
    let score_tensors = [outputs.0, outputs.1, outputs.2];
    let bbox_tensors = [outputs.3, outputs.4, outputs.5];
    let kps_tensors = [outputs.6, outputs.7, outputs.8];

    let mut detections = Vec::new();
    for (idx, &stride) in FEAT_STRIDES.iter().enumerate() {
        let scores = tensor_to_vec2(score_tensors[idx].clone())?;
        let mut bboxes = tensor_to_vec2(bbox_tensors[idx].clone())?;
        let mut kps = tensor_to_vec2(kps_tensors[idx].clone())?;
        for row in &mut bboxes {
            for v in row.iter_mut() {
                *v *= stride as f32;
            }
        }
        for row in &mut kps {
            for v in row.iter_mut() {
                *v *= stride as f32;
            }
        }

        let height = input_h / stride;
        let width = input_w / stride;
        let centers = anchor_centers(height, width, stride, NUM_ANCHORS);
        if scores.len() != centers.len() {
            return Err(format!(
                "scrfd score/center mismatch stride={stride}: {} vs {}",
                scores.len(),
                centers.len()
            ));
        }

        for i in 0..scores.len() {
            let score = scores[i][0];
            if score < score_threshold {
                continue;
            }
            let (x1, y1, x2, y2) = distance2bbox(centers[i], &bboxes[i]);
            let landmarks = distance2kps(centers[i], &kps[i]);
            // Map from letterboxed network space back to original image.
            let x1 = x1 / det_scale;
            let y1 = y1 / det_scale;
            let x2 = x2 / det_scale;
            let y2 = y2 / det_scale;
            let mut lm = landmarks;
            for point in &mut lm {
                point[0] /= det_scale;
                point[1] /= det_scale;
            }
            let bw = (x2 - x1).max(1.0);
            let bh = (y2 - y1).max(1.0);
            detections.push(ScrfdDetection {
                box_xywh: [x1, y1, bw, bh],
                landmarks: lm,
                score,
            });
        }
    }

    detections.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut kept = nms_xywh(detections, nms_threshold);
    if kept.len() > SCRFD_MAX_FACES {
        kept.truncate(SCRFD_MAX_FACES);
    }
    Ok(kept)
}

#[cfg(ai_onnx_generated)]
fn tensor_to_vec2(tensor: Tensor<AiBackend, 2>) -> Result<Vec<Vec<f32>>, String> {
    let [rows, cols] = tensor.dims();
    let data = tensor
        .into_data()
        .to_vec::<f32>()
        .map_err(|error| format!("scrfd output: {error:?}"))?;
    let mut out = Vec::with_capacity(rows);
    for r in 0..rows {
        let start = r * cols;
        out.push(data[start..start + cols].to_vec());
    }
    Ok(out)
}

fn anchor_centers(height: usize, width: usize, stride: usize, num_anchors: usize) -> Vec<[f32; 2]> {
    // np.mgrid[:H,:W][::-1] → x varies along width, y along height.
    let mut centers = Vec::with_capacity(height * width * num_anchors);
    for y in 0..height {
        for x in 0..width {
            let cx = (x * stride) as f32;
            let cy = (y * stride) as f32;
            for _ in 0..num_anchors {
                centers.push([cx, cy]);
            }
        }
    }
    centers
}

fn distance2bbox(center: [f32; 2], distance: &[f32]) -> (f32, f32, f32, f32) {
    let x1 = center[0] - distance[0];
    let y1 = center[1] - distance[1];
    let x2 = center[0] + distance[2];
    let y2 = center[1] + distance[3];
    (x1, y1, x2, y2)
}

fn distance2kps(center: [f32; 2], distance: &[f32]) -> [[f32; 2]; 5] {
    let mut out = [[0.0_f32; 2]; 5];
    for k in 0..5 {
        out[k][0] = center[0] + distance[k * 2];
        out[k][1] = center[1] + distance[k * 2 + 1];
    }
    out
}

fn iou_xywh(a: &[f32; 4], b: &[f32; 4]) -> f32 {
    let ax2 = a[0] + a[2];
    let ay2 = a[1] + a[3];
    let bx2 = b[0] + b[2];
    let by2 = b[1] + b[3];
    let ix1 = a[0].max(b[0]);
    let iy1 = a[1].max(b[1]);
    let ix2 = ax2.min(bx2);
    let iy2 = ay2.min(by2);
    let iw = (ix2 - ix1).max(0.0);
    let ih = (iy2 - iy1).max(0.0);
    let inter = iw * ih;
    let union = a[2] * a[3] + b[2] * b[3] - inter;
    if union <= 0.0 {
        0.0
    } else {
        inter / union
    }
}

fn nms_xywh(detections: Vec<ScrfdDetection>, threshold: f32) -> Vec<ScrfdDetection> {
    let mut suppressed = vec![false; detections.len()];
    let mut keep = Vec::new();
    for i in 0..detections.len() {
        if suppressed[i] {
            continue;
        }
        keep.push(detections[i].clone());
        for j in (i + 1)..detections.len() {
            if suppressed[j] {
                continue;
            }
            if iou_xywh(&detections[i].box_xywh, &detections[j].box_xywh) > threshold {
                suppressed[j] = true;
            }
        }
    }
    keep
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
