//! Local model runtime: verified manifest, weights loaded on demand.
//!
//! SigLIP2 vision, SigLIP2 text, SCRFD, and ArcFace each sit in their own slot.
//! A slot stays empty until the matching call, and drops its weights after
//! [`MODEL_IDLE`] with no caller still holding the model.

use image::{DynamicImage, GenericImageView};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use super::arcface::ArcFaceRuntime;
use super::manifest::{ManifestError, ModelArtifactSpec, ModelManifest, VerifiedModelManifest};
use super::preprocess::align_face_112_rgb;
use super::scrfd::ScrfdRuntime;
use super::siglip2::{Siglip2Text, Siglip2Vision};
use crate::backend::ai::embedding::{ARCFACE_EMBEDDING_DIM, SIGLIP2_EMBEDDING_DIM};
use crate::backend::ai::faces::{DetectedFace, normalize_box_xywh, normalize_landmarks};

/// Drop a model this long after its last use, once no call still holds it.
const MODEL_IDLE: Duration = Duration::from_secs(60);
const MODEL_SWEEP_INTERVAL: Duration = Duration::from_secs(30);
/// burn-onnx graphs overflow the default thread stack during Flex init.
const MODEL_LOAD_STACK: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct LocalModelInfo {
    pub revision: String,
    pub image_dim: usize,
    pub face_dim: usize,
    pub model_root: PathBuf,
    pub device_label: String,
}

struct ResidentSlot<T> {
    name: &'static str,
    model: Mutex<Option<Arc<T>>>,
    last_used: Mutex<Option<Instant>>,
    /// One load at a time. Inference can still run on an already-resident model.
    loading: Mutex<()>,
}

impl<T> ResidentSlot<T> {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            model: Mutex::new(None),
            last_used: Mutex::new(None),
            loading: Mutex::new(()),
        }
    }

    fn lock_model(&self) -> std::sync::MutexGuard<'_, Option<Arc<T>>> {
        self.model.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn lock_last_used(&self) -> std::sync::MutexGuard<'_, Option<Instant>> {
        self.last_used
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    fn current(&self) -> Option<Arc<T>> {
        let model = self.lock_model().as_ref().map(Arc::clone);
        if model.is_some() {
            *self.lock_last_used() = Some(Instant::now());
        }
        model
    }

    fn ensure<F>(&self, load: F) -> Result<Arc<T>, String>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        if let Some(model) = self.current() {
            return Ok(model);
        }
        let _loading = self
            .loading
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(model) = self.current() {
            return Ok(model);
        }
        let name = self.name;
        eprintln!("[ai] loading {name}");
        let started = Instant::now();
        let loaded = std::thread::Builder::new()
            .name(format!("ai-model-load-{name}"))
            .stack_size(MODEL_LOAD_STACK)
            .spawn(load)
            .map_err(|error| {
                let message = format!("spawn model load thread: {error}");
                eprintln!("[ai] load {name} failed: {message}");
                message
            })?
            .join()
            .map_err(|_| {
                let message = format!("{name} load thread panicked");
                eprintln!("[ai] load {name} failed: {message}");
                message
            })?
            .map_err(|error| {
                eprintln!("[ai] load {name} failed: {error}");
                error
            })?;
        let model = Arc::new(loaded);
        *self.lock_model() = Some(Arc::clone(&model));
        *self.lock_last_used() = Some(Instant::now());
        eprintln!(
            "[ai] loaded {name} elapsed_ms={}",
            started.elapsed().as_millis()
        );
        Ok(model)
    }

    fn touch(&self) {
        if self.lock_model().is_some() {
            *self.lock_last_used() = Some(Instant::now());
        }
    }

    /// Drop weights when idle and this slot is the only remaining owner.
    fn sweep(&self, idle: Duration) {
        let mut slot = self.lock_model();
        let Some(model) = slot.as_ref() else {
            return;
        };
        if Arc::strong_count(model) > 1 {
            return;
        }
        let idle_for = self.lock_last_used().map(|used| used.elapsed());
        let stale = idle_for.is_some_and(|elapsed| elapsed >= idle);
        if !stale || Arc::strong_count(slot.as_ref().unwrap()) > 1 {
            return;
        }
        let dropped = slot.take();
        drop(slot);
        drop(dropped);
        *self.lock_last_used() = None;
        eprintln!(
            "[ai] unloaded {} idle_ms={}",
            self.name,
            idle_for.map(|elapsed| elapsed.as_millis()).unwrap_or(0)
        );
    }
}

pub struct LocalModelRuntime {
    pub info: LocalModelInfo,
    root: PathBuf,
    image_text: ModelArtifactSpec,
    face_detector: ModelArtifactSpec,
    face_embedder: ModelArtifactSpec,
    tokenizer_path: Option<PathBuf>,
    max_text_len: usize,
    vision: ResidentSlot<Siglip2Vision>,
    text: ResidentSlot<Siglip2Text>,
    scrfd: ResidentSlot<ScrfdRuntime>,
    arcface: ResidentSlot<ArcFaceRuntime>,
}

impl LocalModelRuntime {
    /// Verify the manifest and remember where weights live. Does not read them.
    pub fn try_load(model_root: &Path) -> Result<Arc<Self>, String> {
        let verified = ModelManifest::verify_in_dir(model_root).map_err(manifest_err)?;
        Ok(Arc::new(Self::from_verified(verified)))
    }

    pub fn from_verified(verified: VerifiedModelManifest) -> Self {
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
        // Vision/faces on `device`; SigLIP text is always Flex CPU (embedding table).
        let device_label = if device.label == "cpu" {
            device.label
        } else {
            format!("{}+flex-text", device.label)
        };
        let image_dim = if manifest.image_text.dimension > 0 {
            manifest.image_text.dimension
        } else {
            SIGLIP2_EMBEDDING_DIM
        };
        let face_dim = if manifest.face_embedder.dimension > 0 {
            manifest.face_embedder.dimension
        } else {
            ARCFACE_EMBEDDING_DIM
        };
        Self {
            info: LocalModelInfo {
                revision: manifest.combined_revision(),
                image_dim,
                face_dim,
                model_root: root.clone(),
                device_label,
            },
            root,
            image_text: manifest.image_text,
            face_detector: manifest.face_detector,
            face_embedder: manifest.face_embedder,
            tokenizer_path,
            max_text_len,
            vision: ResidentSlot::new("siglip2 vision"),
            text: ResidentSlot::new("siglip2 text"),
            scrfd: ResidentSlot::new("scrfd"),
            arcface: ResidentSlot::new("arcface"),
        }
    }

    /// Stop sweeping once `runtime` is dropped (provider reload or shutdown).
    pub fn spawn_idle_sweep(runtime: &Arc<Self>) {
        let weak: Weak<Self> = Arc::downgrade(runtime);
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            eprintln!("[ai] idle unload disabled: no tokio runtime");
            return;
        };
        handle.spawn(async move {
            loop {
                tokio::time::sleep(MODEL_SWEEP_INTERVAL).await;
                let Some(runtime) = weak.upgrade() else {
                    break;
                };
                if tokio::task::spawn_blocking(move || runtime.sweep_idle())
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
    }

    fn sweep_idle(&self) {
        self.vision.sweep(MODEL_IDLE);
        self.text.sweep(MODEL_IDLE);
        self.scrfd.sweep(MODEL_IDLE);
        self.arcface.sweep(MODEL_IDLE);
    }

    pub fn embed_images(&self, images: &[DynamicImage]) -> Vec<Result<Vec<f32>, String>> {
        if images.is_empty() {
            return Vec::new();
        }
        let root = self.root.clone();
        let spec = self.image_text.clone();
        let vision = match self
            .vision
            .ensure(move || Siglip2Vision::load(&root, &spec))
        {
            Ok(vision) => vision,
            Err(error) => {
                return images.into_iter().map(|_| Err(error.clone())).collect();
            }
        };
        let embedded = vision.embed_images(images);
        self.vision.touch();
        embedded
    }

    pub fn embed_texts(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let root = self.root.clone();
        let spec = self.image_text.clone();
        let tokenizer = self.tokenizer_path.clone();
        let max_text_len = self.max_text_len;
        let text = self
            .text
            .ensure(move || Siglip2Text::load(&root, &spec, tokenizer.as_deref(), max_text_len))?;
        let embedded = text.embed_texts(texts);
        self.text.touch();
        embedded
    }

    pub fn detect_and_embed_faces(
        &self,
        image: &DynamicImage,
    ) -> Result<Vec<DetectedFace>, String> {
        let root = self.root.clone();
        let detector = self.face_detector.clone();
        let scrfd = self
            .scrfd
            .ensure(move || ScrfdRuntime::load(&root, &detector))?;
        let root = self.root.clone();
        let embedder = self.face_embedder.clone();
        let arcface = self
            .arcface
            .ensure(move || ArcFaceRuntime::load(&root, &embedder))?;

        let detected = (|| {
            let detections = scrfd.detect(image)?;
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
                let embedding = arcface.embed_crop(&crop)?;
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
        })();
        self.scrfd.touch();
        self.arcface.touch();
        detected
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
                match runtime.embed_images(&[image]).into_iter().next() {
                    Some(Ok(emb)) => {
                        let ms = started.elapsed().as_millis();
                        embed_ms.push(ms);
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
                    Some(Err(error)) => {
                        scene_fail += 1;
                        eprintln!("[smoke] scene fail {rel}: {error}");
                        results.push(serde_json::json!({
                            "file": rel, "split": split, "ok": false, "error": error,
                        }));
                    }
                    None => {
                        scene_fail += 1;
                        eprintln!("[smoke] scene fail {rel}: empty embed result");
                        results.push(serde_json::json!({
                            "file": rel, "split": split, "ok": false, "error": "empty embed result",
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
