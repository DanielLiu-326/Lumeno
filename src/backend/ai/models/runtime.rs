//! Local model runtime: verified manifest → SigLIP2 + SCRFD + ArcFace.

use image::{DynamicImage, GenericImageView};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::arcface::ArcFaceRuntime;
use super::manifest::{ManifestError, ModelManifest, VerifiedModelManifest};
use super::preprocess::align_face_112_rgb;
use super::scrfd::ScrfdRuntime;
use super::siglip2::Siglip2Runtime;
use crate::backend::ai::faces::{DetectedFace, normalize_box_xywh, normalize_landmarks};

#[derive(Debug, Clone)]
pub struct LocalModelInfo {
    pub revision: String,
    pub image_dim: usize,
    pub face_dim: usize,
    pub model_root: PathBuf,
    pub device_label: String,
}

pub struct LocalModelRuntime {
    pub info: LocalModelInfo,
    siglip2: Siglip2Runtime,
    scrfd: ScrfdRuntime,
    arcface: ArcFaceRuntime,
}

impl LocalModelRuntime {
    pub fn try_load(model_root: &Path) -> Result<Arc<Self>, String> {
        // burn-onnx generated graphs are deep; Flex init can overflow the default stack.
        let model_root = model_root.to_path_buf();
        std::thread::Builder::new()
            .name("ai-model-load".into())
            .stack_size(128 * 1024 * 1024)
            .spawn(move || {
                let verified = ModelManifest::verify_in_dir(&model_root).map_err(manifest_err)?;
                Self::from_verified(verified).map(Arc::new)
            })
            .map_err(|error| format!("spawn model load thread: {error}"))?
            .join()
            .map_err(|_| "model load thread panicked".to_string())?
    }

    pub fn from_verified(verified: VerifiedModelManifest) -> Result<Self, String> {
        let root = verified.root.clone();
        let manifest = verified.manifest;
        let tokenizer_path = manifest
            .tokenizer
            .as_ref()
            .map(|spec| root.join(&spec.file));
        let max_text_len = manifest
            .tokenizer
            .as_ref()
            .and_then(|spec| spec.max_length)
            .unwrap_or(64);
        let device = super::selected_ai_device();
        let siglip2 = Siglip2Runtime::load(
            &root,
            &manifest.image_text,
            tokenizer_path.as_deref(),
            max_text_len,
        )?;
        let scrfd = ScrfdRuntime::load(&root, &manifest.face_detector)?;
        let arcface = ArcFaceRuntime::load(&root, &manifest.face_embedder)?;
        let revision = manifest.combined_revision();
        Ok(Self {
            info: LocalModelInfo {
                revision,
                image_dim: manifest.image_text.dimension.max(siglip2.dimension),
                face_dim: manifest.face_embedder.dimension.max(arcface.dimension),
                model_root: root,
                device_label: device.label,
            },
            siglip2,
            scrfd,
            arcface,
        })
    }

    pub fn embed_images(&self, images: &[DynamicImage]) -> Result<Vec<Vec<f32>>, String> {
        self.siglip2.embed_images(images)
    }

    pub fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        self.siglip2.embed_texts(texts)
    }

    pub fn detect_and_embed_faces(
        &self,
        image: &DynamicImage,
    ) -> Result<Vec<DetectedFace>, String> {
        let detections = self.scrfd.detect(image)?;
        let (img_w, img_h) = image.dimensions();
        let owned;
        let rgb = match image {
            DynamicImage::ImageRgb8(rgb) => rgb,
            other => {
                owned = other.to_rgb8();
                &owned
            }
        };

        // Align on CPU (112×112 sample). Uploading the full photo as f32 NCHW for
        // GPU warp blows VRAM on large images; ArcFace only needs the tiny crop.
        let mut faces = Vec::with_capacity(detections.len());
        for det in detections {
            let crop = align_face_112_rgb(rgb, det.landmarks)?;
            let embedding = self.arcface.embed_crop(&crop)?;
            let area = det.box_xywh[2] * det.box_xywh[3];
            faces.push(DetectedFace {
                box_xywh: normalize_box_xywh(det.box_xywh, img_w, img_h),
                landmarks: normalize_landmarks(det.landmarks, img_w, img_h),
                detection_score: det.score,
                quality_score: (det.score * (area / (area + 1.0))).clamp(0.0, 1.0),
                embedding,
            });
        }
        Ok(faces)
    }
}

fn manifest_err(error: ManifestError) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn verifies_dist_manifest_when_present() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dist/models");
        if !root.join("manifest.json").is_file() {
            return;
        }
        ModelManifest::verify_in_dir(&root).expect("manifest should verify");
    }

    #[test]
    #[ignore = "loads multi-GB models + testdata; run: download_ai_testset.py then cargo test -- --ignored smoke_ai_testset"]
    fn smoke_ai_testset() {
        use std::time::Instant;

        let models = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dist/models");
        let testdata = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dist/testdata/ai_smoke");
        if !models.join("manifest.json").is_file() || !cfg!(ai_onnx_generated) {
            eprintln!("skip: models unavailable");
            return;
        }
        if !testdata.join("manifest.json").is_file() {
            eprintln!(
                "skip: testdata missing; run `.venv-ai/bin/python scripts/download_ai_testset.py`"
            );
            return;
        }

        let limit = std::env::var("LUMENO_AI_SMOKE_LIMIT")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(100)
            .clamp(1, 100);
        let split_filter = std::env::var("LUMENO_AI_SMOKE_SPLIT")
            .unwrap_or_else(|_| "all".into())
            .to_ascii_lowercase();

        let runtime = LocalModelRuntime::try_load(&models).expect("load models");
        eprintln!(
            "[smoke] device={} revision={} limit={limit} split={split_filter}",
            runtime.info.device_label, runtime.info.revision
        );

        let meta: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(testdata.join("manifest.json")).expect("read testdata meta"),
        )
        .expect("parse testdata meta");
        let items = meta["items"].as_array().cloned().unwrap_or_default();

        let mut scene_ok = 0usize;
        let mut scene_fail = 0usize;
        let mut face_ok = 0usize;
        let mut face_fail = 0usize;
        let mut faces_detected = 0usize;
        let mut embed_ms = Vec::new();
        let mut detect_ms = Vec::new();
        let mut results = Vec::new();

        let queries = [
            "a photo of a person",
            "a photo of an animal",
            "a photo of food",
            "outdoor landscape",
        ];
        let query_embeds = runtime
            .embed_texts(&queries.iter().map(|q| (*q).to_string()).collect::<Vec<_>>())
            .expect("text embeds");

        let mut processed = 0usize;
        for item in items {
            if processed >= limit {
                break;
            }
            let split = item["split"].as_str().unwrap_or("");
            let rel = item["file"].as_str().unwrap_or("");
            if split_filter != "all" && split != split_filter {
                continue;
            }
            let path = testdata.join(rel);
            if !path.is_file() {
                continue;
            }
            processed += 1;
            let image = match image::open(&path) {
                Ok(img) => img,
                Err(error) => {
                    eprintln!("[smoke] open fail {rel}: {error}");
                    if split == "faces" {
                        face_fail += 1;
                    } else {
                        scene_fail += 1;
                    }
                    continue;
                }
            };

            if split == "faces" {
                let started = Instant::now();
                match runtime.detect_and_embed_faces(&image) {
                    Ok(faces) => {
                        let ms = started.elapsed().as_millis();
                        detect_ms.push(ms);
                        faces_detected += faces.len();
                        face_ok += 1;
                        eprintln!(
                            "[smoke] face {rel}: {} faces in {ms}ms scores={:?}",
                            faces.len(),
                            faces
                                .iter()
                                .map(|f| format!("{:.2}", f.detection_score))
                                .collect::<Vec<_>>()
                        );
                        results.push(serde_json::json!({
                            "file": rel,
                            "split": split,
                            "faces": faces.len(),
                            "ms": ms,
                            "ok": true,
                        }));
                    }
                    Err(error) => {
                        face_fail += 1;
                        eprintln!("[smoke] face fail {rel}: {error}");
                        results.push(serde_json::json!({
                            "file": rel, "split": split, "ok": false, "error": error,
                        }));
                    }
                }
            } else {
                let started = Instant::now();
                match runtime.embed_images(&[image]) {
                    Ok(mut embeds) => {
                        let ms = started.elapsed().as_millis();
                        embed_ms.push(ms);
                        let emb = embeds.pop().expect("embed");
                        assert_eq!(emb.len(), runtime.info.image_dim);
                        scene_ok += 1;
                        let mut best = (0usize, f32::NEG_INFINITY);
                        for (qi, qe) in query_embeds.iter().enumerate() {
                            let score = cosine(&emb, qe);
                            if score > best.1 {
                                best = (qi, score);
                            }
                        }
                        eprintln!(
                            "[smoke] scene {rel}: {ms}ms best='{}' cos={:.3}",
                            queries[best.0], best.1
                        );
                        results.push(serde_json::json!({
                            "file": rel,
                            "split": split,
                            "ms": ms,
                            "best_query": queries[best.0],
                            "cosine": best.1,
                            "ok": true,
                        }));
                    }
                    Err(error) => {
                        scene_fail += 1;
                        eprintln!("[smoke] scene fail {rel}: {error}");
                        results.push(serde_json::json!({
                            "file": rel, "split": split, "ok": false, "error": error,
                        }));
                    }
                }
            }
        }

        let summary = serde_json::json!({
            "device": runtime.info.device_label,
            "revision": runtime.info.revision,
            "processed": processed,
            "scene_ok": scene_ok,
            "scene_fail": scene_fail,
            "face_ok": face_ok,
            "face_fail": face_fail,
            "faces_detected": faces_detected,
            "embed_ms_avg": avg(&embed_ms),
            "detect_ms_avg": avg(&detect_ms),
            "results": results,
        });
        let out_path = testdata.join("results.json");
        std::fs::write(
            &out_path,
            serde_json::to_string_pretty(&summary).unwrap() + "\n",
        )
        .expect("write results");
        eprintln!("[smoke] wrote {}", out_path.display());
        eprintln!(
            "[smoke] summary scenes={scene_ok}/{}/{} faces={face_ok}/{} detected={faces_detected} embed_avg_ms={:?} detect_avg_ms={:?}",
            scene_ok + scene_fail,
            processed,
            face_ok + face_fail,
            avg(&embed_ms),
            avg(&detect_ms)
        );

        assert!(
            scene_fail == 0 && face_fail == 0,
            "smoke failures: scenes={scene_fail} faces={face_fail}"
        );
        assert!(scene_ok + face_ok > 0, "no images processed successfully");
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let n = a.len().min(b.len());
        let mut dot = 0.0_f32;
        let mut na = 0.0_f32;
        let mut nb = 0.0_f32;
        for i in 0..n {
            dot += a[i] * b[i];
            na += a[i] * a[i];
            nb += b[i] * b[i];
        }
        if na == 0.0 || nb == 0.0 {
            0.0
        } else {
            dot / (na.sqrt() * nb.sqrt())
        }
    }

    fn avg(values: &[u128]) -> Option<f64> {
        if values.is_empty() {
            None
        } else {
            Some(values.iter().sum::<u128>() as f64 / values.len() as f64)
        }
    }
}
