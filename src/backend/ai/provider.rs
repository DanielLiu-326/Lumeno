//! Inference providers: decoded images / texts in, vectors / faces out.
//!
//! Providers must not read paths, decode images, own pipelines, or touch cache.

use async_trait::async_trait;
use image::{DynamicImage, ImageFormat};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::embedding::{
    TEXT_HASH_REVISION, VISUAL_EMBED_REVISION, compute_text_hash_embedding,
    compute_visual_embedding_rgb,
};
use super::faces::{DetectedFace, FACE_DETECTOR_REVISION, detect_and_embed_faces_from_image};
use super::models::runtime::LocalModelRuntime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiProviderKind {
    Local,
    Remote,
}

impl AiProviderKind {
    pub fn from_settings(settings: &Value) -> Self {
        match settings
            .get("ai_provider")
            .and_then(|value| value.as_str())
            .unwrap_or("local")
        {
            "remote" => Self::Remote,
            _ => Self::Local,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AiCapabilities {
    pub image_search: bool,
    pub text_search: bool,
    pub duplicates: bool,
    pub faces: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_batch_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_embedding_dimension: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub face_embedding_dimension: Option<usize>,
}

impl Default for AiCapabilities {
    fn default() -> Self {
        Self {
            image_search: false,
            text_search: false,
            duplicates: true,
            faces: false,
            max_batch_size: None,
            image_embedding_dimension: None,
            face_embedding_dimension: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AiProviderHealth {
    pub kind: String,
    pub backend: String,
    pub healthy: bool,
    pub model_root: String,
    pub model_manifest_present: bool,
    pub revision: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AiItemError {
    pub code: String,
    pub message: String,
}

impl std::fmt::Display for AiItemError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

#[async_trait]
pub trait AiInferenceProvider: Send + Sync {
    fn kind(&self) -> AiProviderKind;
    fn capabilities(&self) -> &AiCapabilities;
    fn model_revision(&self) -> Option<String>;
    async fn health(&self) -> AiProviderHealth;

    /// Embed already-decoded images (order-preserving, per-item errors).
    async fn embed_images(
        &self,
        images: &[DynamicImage],
    ) -> Result<Vec<Result<Vec<f32>, AiItemError>>, String>;

    async fn embed_texts(
        &self,
        texts: &[String],
    ) -> Result<Vec<Result<Vec<f32>, AiItemError>>, String>;

    /// Detect faces on already-decoded images (order-preserving, per-item errors).
    async fn detect_and_embed_faces(
        &self,
        images: &[DynamicImage],
    ) -> Result<Vec<Result<Vec<DetectedFace>, AiItemError>>, String>;
}

enum LocalInferenceMode {
    Real(Arc<LocalModelRuntime>),
    Interim,
    Unavailable(String),
}

pub struct LocalBurnProvider {
    model_root: PathBuf,
    capabilities: AiCapabilities,
    revision: Option<String>,
    mode: LocalInferenceMode,
}

impl LocalBurnProvider {
    pub fn new(model_root: PathBuf, batch_size: usize) -> Self {
        Self::with_options(model_root, batch_size, false)
    }

    pub fn with_interim_fallback(model_root: PathBuf, batch_size: usize) -> Self {
        Self::with_options(model_root, batch_size, true)
    }

    pub fn with_options(model_root: PathBuf, batch_size: usize, fallback_interim: bool) -> Self {
        let max_batch_size = batch_size.clamp(1, 64);
        match LocalModelRuntime::try_load(&model_root) {
            Ok(runtime) => {
                let image_dim = runtime.info.image_dim;
                let face_dim = runtime.info.face_dim;
                let revision = runtime.info.revision.clone();
                Self {
                    model_root,
                    capabilities: AiCapabilities {
                        image_search: true,
                        text_search: true,
                        duplicates: true,
                        faces: true,
                        max_batch_size: Some(max_batch_size),
                        image_embedding_dimension: Some(image_dim),
                        face_embedding_dimension: Some(face_dim),
                    },
                    revision: Some(revision),
                    mode: LocalInferenceMode::Real(runtime),
                }
            }
            Err(_error) if fallback_interim => Self {
                model_root,
                capabilities: AiCapabilities {
                    image_search: true,
                    text_search: true,
                    duplicates: true,
                    faces: true,
                    max_batch_size: Some(max_batch_size),
                    image_embedding_dimension: Some(super::embedding::INTERIM_EMBEDDING_DIM),
                    face_embedding_dimension: Some(super::embedding::INTERIM_EMBEDDING_DIM),
                },
                revision: Some(format!(
                    "{VISUAL_EMBED_REVISION}+{TEXT_HASH_REVISION}+{FACE_DETECTOR_REVISION}"
                )),
                mode: LocalInferenceMode::Interim,
            },
            Err(error) => Self {
                model_root,
                capabilities: AiCapabilities {
                    image_search: false,
                    text_search: false,
                    duplicates: true,
                    faces: false,
                    max_batch_size: Some(max_batch_size),
                    image_embedding_dimension: None,
                    face_embedding_dimension: None,
                },
                revision: None,
                mode: LocalInferenceMode::Unavailable(error),
            },
        }
    }

    fn manifest_path(&self) -> PathBuf {
        self.model_root.join("manifest.json")
    }

    fn backend_name(&self) -> String {
        match &self.mode {
            LocalInferenceMode::Real(runtime) => format!(
                "burn-dispatch-siglip2-scrfd-arcface+{}",
                runtime.info.device_label
            ),
            LocalInferenceMode::Interim => "burn-flex-interim-dct".to_string(),
            LocalInferenceMode::Unavailable(_) => "burn-dispatch-unavailable".to_string(),
        }
    }

    fn unavailable_error(&self) -> AiItemError {
        let message = match &self.mode {
            LocalInferenceMode::Unavailable(reason) => reason.clone(),
            _ => "local AI models unavailable".to_string(),
        };
        AiItemError {
            code: "model_unavailable".to_string(),
            message,
        }
    }
}

#[async_trait]
impl AiInferenceProvider for LocalBurnProvider {
    fn kind(&self) -> AiProviderKind {
        AiProviderKind::Local
    }

    fn capabilities(&self) -> &AiCapabilities {
        &self.capabilities
    }

    fn model_revision(&self) -> Option<String> {
        self.revision.clone()
    }

    async fn health(&self) -> AiProviderHealth {
        let manifest_present = self.manifest_path().exists();
        let (healthy, reason) = match &self.mode {
            LocalInferenceMode::Real(_) => (true, None),
            LocalInferenceMode::Interim => (
                true,
                Some("ai_fallback_interim=true; using Burn Flex DCT / heuristic faces".to_string()),
            ),
            LocalInferenceMode::Unavailable(error) => (
                false,
                Some(format!("local models unavailable (fail-closed): {error}")),
            ),
        };
        AiProviderHealth {
            kind: self.kind().as_str().to_string(),
            backend: self.backend_name(),
            healthy,
            model_root: self.model_root.display().to_string(),
            model_manifest_present: manifest_present,
            revision: self.revision.clone(),
            reason,
        }
    }

    async fn embed_images(
        &self,
        images: &[DynamicImage],
    ) -> Result<Vec<Result<Vec<f32>, AiItemError>>, String> {
        if images.is_empty() {
            return Ok(Vec::new());
        }
        match &self.mode {
            LocalInferenceMode::Unavailable(_) => Ok(images
                .iter()
                .map(|_| Err(self.unavailable_error()))
                .collect()),
            LocalInferenceMode::Interim => {
                let images = images.to_vec();
                tokio::task::spawn_blocking(move || {
                    Ok(images
                        .iter()
                        .map(|image| {
                            compute_visual_embedding_rgb(&image.to_rgb8()).map_err(|message| {
                                AiItemError {
                                    code: "invalid_output".to_string(),
                                    message,
                                }
                            })
                        })
                        .collect())
                })
                .await
                .map_err(|error| format!("interim embed_images join failed: {error}"))?
            }
            LocalInferenceMode::Real(runtime) => {
                let runtime = Arc::clone(runtime);
                let images = images.to_vec();
                tokio::task::spawn_blocking(move || {
                    Ok(runtime
                        .embed_images(&images)
                        .into_iter()
                        .map(|result| {
                            result.map_err(|message| AiItemError {
                                code: "provider_rejected".to_string(),
                                message,
                            })
                        })
                        .collect())
                })
                .await
                .map_err(|error| format!("embed_images join failed: {error}"))?
            }
        }
    }

    async fn embed_texts(
        &self,
        texts: &[String],
    ) -> Result<Vec<Result<Vec<f32>, AiItemError>>, String> {
        match &self.mode {
            LocalInferenceMode::Unavailable(_) => Ok(texts
                .iter()
                .map(|_| Err(self.unavailable_error()))
                .collect()),
            LocalInferenceMode::Interim => {
                let texts = texts.to_vec();
                tokio::task::spawn_blocking(move || {
                    Ok(texts
                        .iter()
                        .map(|text| {
                            compute_text_hash_embedding(text).map_err(|message| AiItemError {
                                code: "invalid_output".to_string(),
                                message,
                            })
                        })
                        .collect())
                })
                .await
                .map_err(|error| format!("interim embed_texts join failed: {error}"))?
            }
            LocalInferenceMode::Real(runtime) => {
                let runtime = Arc::clone(runtime);
                let texts = texts.to_vec();
                tokio::task::spawn_blocking(move || match runtime.embed_texts(&texts) {
                    Ok(vectors) => Ok(vectors.into_iter().map(Ok).collect()),
                    Err(message) => Ok(texts
                        .iter()
                        .map(|_| {
                            Err(AiItemError {
                                code: "provider_rejected".to_string(),
                                message: message.clone(),
                            })
                        })
                        .collect()),
                })
                .await
                .map_err(|error| format!("embed_texts join failed: {error}"))?
            }
        }
    }

    async fn detect_and_embed_faces(
        &self,
        images: &[DynamicImage],
    ) -> Result<Vec<Result<Vec<DetectedFace>, AiItemError>>, String> {
        if images.is_empty() {
            return Ok(Vec::new());
        }
        match &self.mode {
            LocalInferenceMode::Unavailable(_) => Ok(images
                .iter()
                .map(|_| Err(self.unavailable_error()))
                .collect()),
            LocalInferenceMode::Interim => {
                let images = images.to_vec();
                tokio::task::spawn_blocking(move || {
                    Ok(images
                        .iter()
                        .map(|image| {
                            detect_and_embed_faces_from_image(image).map_err(|message| {
                                AiItemError {
                                    code: "provider_rejected".to_string(),
                                    message,
                                }
                            })
                        })
                        .collect())
                })
                .await
                .map_err(|error| format!("interim face inference join failed: {error}"))?
            }
            LocalInferenceMode::Real(runtime) => {
                let runtime = Arc::clone(runtime);
                let images = images.to_vec();
                tokio::task::spawn_blocking(move || {
                    Ok(images
                        .iter()
                        .map(|image| {
                            runtime
                                .detect_and_embed_faces(image)
                                .map_err(|message| AiItemError {
                                    code: "provider_rejected".to_string(),
                                    message,
                                })
                        })
                        .collect())
                })
                .await
                .map_err(|error| format!("detect_and_embed_faces join failed: {error}"))?
            }
        }
    }
}

pub struct RemoteInferenceProvider {
    remote_url: Option<String>,
    token: Option<String>,
    capabilities: AiCapabilities,
    revision: Option<String>,
    client: reqwest::Client,
}

impl RemoteInferenceProvider {
    pub fn new(remote_url: Option<String>) -> Self {
        let token = std::env::var("AI_REMOTE_TOKEN")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        Self::with_auth(remote_url, token)
    }

    pub fn with_auth(remote_url: Option<String>, token: Option<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            remote_url,
            token,
            capabilities: AiCapabilities::default(),
            revision: None,
            client,
        }
    }

    fn base_url(&self) -> Option<String> {
        self.remote_url
            .as_ref()
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| !value.is_empty())
    }

    fn auth_header(&self) -> Option<String> {
        self.token.as_ref().map(|token| format!("Bearer {token}"))
    }

    async fn refresh_capabilities(&mut self) -> Result<(), String> {
        let Some(base) = self.base_url() else {
            return Err("remote url missing".to_string());
        };
        let mut request = self.client.get(format!("{base}/v1/capabilities"));
        if let Some(auth) = self.auth_header() {
            request = request.header(reqwest::header::AUTHORIZATION, auth);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("capabilities request failed: {error}"))?;
        if !response.status().is_success() {
            return Err(format!("capabilities status {}", response.status()));
        }
        let body: RemoteCapabilitiesResponse = response
            .json()
            .await
            .map_err(|error| format!("capabilities decode failed: {error}"))?;
        self.capabilities = AiCapabilities {
            image_search: body.image_embedding,
            text_search: body.text_embedding,
            duplicates: true,
            faces: body.face_detection && body.face_embedding,
            max_batch_size: Some(body.max_batch_size.unwrap_or(8).clamp(1, 64)),
            image_embedding_dimension: body.image_embedding_dimension,
            face_embedding_dimension: body.face_embedding_dimension,
        };
        self.revision = body.revision;
        Ok(())
    }
}

fn encode_image_png(image: &DynamicImage) -> Result<Vec<u8>, AiItemError> {
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|error| AiItemError {
            code: "encode_failed".to_string(),
            message: error.to_string(),
        })?;
    Ok(bytes)
}

#[derive(Debug, Deserialize)]
struct RemoteCapabilitiesResponse {
    #[serde(default)]
    image_embedding: bool,
    #[serde(default)]
    text_embedding: bool,
    #[serde(default)]
    face_detection: bool,
    #[serde(default)]
    face_embedding: bool,
    max_batch_size: Option<usize>,
    image_embedding_dimension: Option<usize>,
    #[serde(default)]
    face_embedding_dimension: Option<usize>,
    revision: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteEmbeddingResponse {
    embeddings: Vec<Vec<f32>>,
    #[serde(default)]
    revision: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteFaceResponse {
    faces: Vec<Vec<RemoteDetectedFace>>,
    #[serde(default)]
    revision: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RemoteDetectedFace {
    box_xywh: [f32; 4],
    #[serde(default)]
    landmarks: Option<[[f32; 2]; 5]>,
    #[serde(default)]
    detection_score: Option<f32>,
    #[serde(default)]
    quality_score: Option<f32>,
    embedding: Vec<f32>,
}

#[async_trait]
impl AiInferenceProvider for RemoteInferenceProvider {
    fn kind(&self) -> AiProviderKind {
        AiProviderKind::Remote
    }

    fn capabilities(&self) -> &AiCapabilities {
        &self.capabilities
    }

    fn model_revision(&self) -> Option<String> {
        self.revision.clone()
    }

    async fn health(&self) -> AiProviderHealth {
        let Some(base) = self.base_url() else {
            return AiProviderHealth {
                kind: self.kind().as_str().to_string(),
                backend: "remote".to_string(),
                healthy: false,
                model_root: String::new(),
                model_manifest_present: false,
                revision: None,
                reason: Some("remote url missing".to_string()),
            };
        };
        let mut request = self.client.get(format!("{base}/v1/health"));
        if let Some(auth) = self.auth_header() {
            request = request.header(reqwest::header::AUTHORIZATION, auth);
        }
        match request.send().await {
            Ok(response) if response.status().is_success() => AiProviderHealth {
                kind: self.kind().as_str().to_string(),
                backend: "remote".to_string(),
                healthy: true,
                model_root: base,
                model_manifest_present: true,
                revision: self.revision.clone(),
                reason: None,
            },
            Ok(response) => AiProviderHealth {
                kind: self.kind().as_str().to_string(),
                backend: "remote".to_string(),
                healthy: false,
                model_root: base,
                model_manifest_present: true,
                revision: self.revision.clone(),
                reason: Some(format!("health status {}", response.status())),
            },
            Err(error) => AiProviderHealth {
                kind: self.kind().as_str().to_string(),
                backend: "remote".to_string(),
                healthy: false,
                model_root: base,
                model_manifest_present: false,
                revision: self.revision.clone(),
                reason: Some(error.to_string()),
            },
        }
    }

    async fn embed_images(
        &self,
        images: &[DynamicImage],
    ) -> Result<Vec<Result<Vec<f32>, AiItemError>>, String> {
        let Some(base) = self.base_url() else {
            return Err("remote url missing".to_string());
        };
        let mut form = reqwest::multipart::Form::new();
        let mut batch_indices = Vec::new();
        let mut results = vec![
            Err(AiItemError {
                code: "encode_failed".to_string(),
                message: "image not submitted".to_string(),
            });
            images.len()
        ];
        for (index, image) in images.iter().enumerate() {
            let bytes = match encode_image_png(image) {
                Ok(bytes) => bytes,
                Err(error) => {
                    results[index] = Err(error);
                    continue;
                }
            };
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name(format!("image-{index}.png"))
                .mime_str("image/png")
                .map_err(|error| format!("multipart part failed: {error}"))?;
            form = form.part(format!("images[{index}]"), part);
            batch_indices.push(index);
        }
        if batch_indices.is_empty() {
            return Ok(results);
        }
        let mut request = self
            .client
            .post(format!("{base}/v1/embeddings/images"))
            .multipart(form);
        if let Some(auth) = self.auth_header() {
            request = request.header(reqwest::header::AUTHORIZATION, auth);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("image embedding request failed: {error}"))?;
        if !response.status().is_success() {
            let message = format!("image embedding status {}", response.status());
            for index in batch_indices {
                results[index] = Err(AiItemError {
                    code: "provider_rejected".to_string(),
                    message: message.clone(),
                });
            }
            return Ok(results);
        }
        let body: RemoteEmbeddingResponse = response
            .json()
            .await
            .map_err(|error| format!("image embedding decode failed: {error}"))?;
        if body.embeddings.len() != batch_indices.len() {
            let message = format!(
                "image embedding count mismatch: got {} expected {}",
                body.embeddings.len(),
                batch_indices.len()
            );
            for index in batch_indices {
                results[index] = Err(AiItemError {
                    code: "invalid_output".to_string(),
                    message: message.clone(),
                });
            }
            return Ok(results);
        }
        for (index, vector) in batch_indices.into_iter().zip(body.embeddings) {
            results[index] = if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
                Err(AiItemError {
                    code: "invalid_output".to_string(),
                    message: "remote embedding invalid".to_string(),
                })
            } else {
                Ok(vector)
            };
        }
        Ok(results)
    }

    async fn embed_texts(
        &self,
        texts: &[String],
    ) -> Result<Vec<Result<Vec<f32>, AiItemError>>, String> {
        let Some(base) = self.base_url() else {
            return Err("remote url missing".to_string());
        };
        let mut request = self
            .client
            .post(format!("{base}/v1/embeddings/texts"))
            .json(&serde_json::json!({ "texts": texts }));
        if let Some(auth) = self.auth_header() {
            request = request.header(reqwest::header::AUTHORIZATION, auth);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("text embedding request failed: {error}"))?;
        if !response.status().is_success() {
            return Err(format!("text embedding status {}", response.status()));
        }
        let body: RemoteEmbeddingResponse = response
            .json()
            .await
            .map_err(|error| format!("text embedding decode failed: {error}"))?;
        if body.embeddings.len() != texts.len() {
            return Err(format!(
                "text embedding count mismatch: got {} expected {}",
                body.embeddings.len(),
                texts.len()
            ));
        }
        Ok(body
            .embeddings
            .into_iter()
            .map(|vector| {
                if vector.is_empty() || vector.iter().any(|value| !value.is_finite()) {
                    Err(AiItemError {
                        code: "invalid_output".to_string(),
                        message: "remote embedding invalid".to_string(),
                    })
                } else {
                    Ok(vector)
                }
            })
            .collect())
    }

    async fn detect_and_embed_faces(
        &self,
        images: &[DynamicImage],
    ) -> Result<Vec<Result<Vec<DetectedFace>, AiItemError>>, String> {
        let Some(base) = self.base_url() else {
            return Err("remote url missing".to_string());
        };
        let mut form = reqwest::multipart::Form::new();
        let mut batch_indices = Vec::new();
        let mut results = vec![
            Err(AiItemError {
                code: "encode_failed".to_string(),
                message: "image not submitted".to_string(),
            });
            images.len()
        ];
        for (index, image) in images.iter().enumerate() {
            let bytes = match encode_image_png(image) {
                Ok(bytes) => bytes,
                Err(error) => {
                    results[index] = Err(error);
                    continue;
                }
            };
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name(format!("image-{index}.png"))
                .mime_str("image/png")
                .map_err(|error| format!("multipart part failed: {error}"))?;
            form = form.part(format!("images[{index}]"), part);
            batch_indices.push(index);
        }
        if batch_indices.is_empty() {
            return Ok(results);
        }
        let mut request = self
            .client
            .post(format!("{base}/v1/faces/analyze"))
            .multipart(form);
        if let Some(auth) = self.auth_header() {
            request = request.header(reqwest::header::AUTHORIZATION, auth);
        }
        let response = request
            .send()
            .await
            .map_err(|error| format!("face analyze request failed: {error}"))?;
        if !response.status().is_success() {
            let message = format!("face analyze status {}", response.status());
            for index in batch_indices {
                results[index] = Err(AiItemError {
                    code: "provider_rejected".to_string(),
                    message: message.clone(),
                });
            }
            return Ok(results);
        }
        let body: RemoteFaceResponse = response
            .json()
            .await
            .map_err(|error| format!("face analyze decode failed: {error}"))?;
        if body.faces.len() != batch_indices.len() {
            let message = format!(
                "face analyze count mismatch: got {} expected {}",
                body.faces.len(),
                batch_indices.len()
            );
            for index in batch_indices {
                results[index] = Err(AiItemError {
                    code: "invalid_output".to_string(),
                    message: message.clone(),
                });
            }
            return Ok(results);
        }
        for (index, faces) in batch_indices.into_iter().zip(body.faces) {
            results[index] = Ok(faces
                .into_iter()
                .map(|face| DetectedFace {
                    box_xywh: face.box_xywh,
                    landmarks: face.landmarks.unwrap_or([[0.0; 2]; 5]),
                    detection_score: face.detection_score.unwrap_or(0.0),
                    quality_score: face.quality_score.unwrap_or(0.0),
                    embedding: face.embedding,
                })
                .collect());
        }
        Ok(results)
    }
}

pub async fn build_provider_from_settings(
    settings: &Value,
    models_root: PathBuf,
) -> Arc<dyn AiInferenceProvider> {
    let batch_size = settings
        .get("ai_batch_size")
        .and_then(|value| value.as_u64())
        .map(|value| value as usize)
        .unwrap_or(64);
    match AiProviderKind::from_settings(settings) {
        AiProviderKind::Local => {
            let fallback_interim = settings
                .get("ai_fallback_interim")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            Arc::new(LocalBurnProvider::with_options(
                models_root,
                batch_size,
                fallback_interim,
            ))
        }
        AiProviderKind::Remote => {
            let remote_url = settings
                .get("ai_remote_url")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            let mut remote = RemoteInferenceProvider::new(remote_url);
            let _ = remote.refresh_capabilities().await;
            Arc::new(remote)
        }
    }
}

pub fn validate_remote_url(url: &str) -> Result<(), String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err("ai_remote_url must start with http:// or https://".to_string());
    }
    Ok(())
}

pub fn validate_ai_settings_update(settings: &Value) -> Result<(), String> {
    if let Some(url) = settings
        .get("ai_remote_url")
        .and_then(|value| value.as_str())
    {
        validate_remote_url(url)?;
    }
    if AiProviderKind::from_settings(settings) == AiProviderKind::Remote {
        let remote_url = settings
            .get("ai_remote_url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty());
        if remote_url.is_none() {
            return Err("ai_remote_url is required when ai_provider is remote".to_string());
        }
    }
    Ok(())
}

#[allow(dead_code)]
pub fn provider_supports_path(_path: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Multipart;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use serde_json::json;
    use std::net::SocketAddr;
    use tokio::net::TcpListener;
    use uuid::Uuid;

    #[tokio::test]
    async fn local_provider_fail_closed_without_manifest() {
        let root = std::env::temp_dir().join(format!("lumeno-ai-empty-{}", Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&root);
        let provider = LocalBurnProvider::new(root.clone(), 2);
        let health = provider.health().await;
        assert!(!health.healthy);
        assert!(!provider.capabilities().image_search);
        assert!(!provider.capabilities().faces);
        assert!(provider.capabilities().duplicates);
        let embeds = provider.embed_texts(&["hello".to_string()]).await.unwrap();
        assert!(embeds[0].is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn local_provider_interim_fallback_when_enabled() {
        let root = std::env::temp_dir().join(format!("lumeno-ai-interim-{}", Uuid::new_v4()));
        let _ = std::fs::create_dir_all(&root);
        let provider = LocalBurnProvider::with_interim_fallback(root.clone(), 2);
        let health = provider.health().await;
        assert!(health.healthy);
        assert!(provider.capabilities().image_search);
        let embeds = provider
            .embed_texts(&["hello world".to_string()])
            .await
            .unwrap();
        assert_eq!(embeds[0].as_ref().unwrap().len(), 128);
        let mut tiny = image::RgbImage::new(32, 32);
        for (x, y, pixel) in tiny.enumerate_pixels_mut() {
            *pixel = image::Rgb([(x * 7 + y * 3) as u8, (x * 5) as u8, (y * 11) as u8]);
        }
        let tiny = DynamicImage::ImageRgb8(tiny);
        let images = provider.embed_images(&[tiny.clone()]).await.unwrap();
        assert!(images[0].as_ref().unwrap().len() > 0);
        let faces = provider.detect_and_embed_faces(&[tiny]).await.unwrap();
        assert!(faces[0].is_ok());
        let _ = std::fs::remove_dir_all(root);
    }

    async fn health() -> Json<Value> {
        Json(json!({"status": "ok", "revision": "remote-test-v1"}))
    }

    async fn capabilities(headers: HeaderMap) -> Result<Json<Value>, StatusCode> {
        if headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            != Some("Bearer test-token")
        {
            return Err(StatusCode::UNAUTHORIZED);
        }
        Ok(Json(json!({
            "image_embedding": true,
            "text_embedding": true,
            "face_detection": true,
            "face_embedding": true,
            "max_batch_size": 4,
            "image_embedding_dimension": 4,
            "revision": "remote-test-v1"
        })))
    }

    async fn embed_images(mut multipart: Multipart) -> Result<Json<Value>, StatusCode> {
        let mut count = 0usize;
        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?
        {
            let _ = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
            count += 1;
        }
        Ok(Json(json!({
            "embeddings": vec![vec![0.5f32, 0.5, 0.5, 0.5]; count],
            "revision": "remote-test-v1"
        })))
    }

    async fn embed_texts(Json(body): Json<Value>) -> Result<Json<Value>, StatusCode> {
        let texts = body
            .get("texts")
            .and_then(|value| value.as_array())
            .ok_or(StatusCode::BAD_REQUEST)?;
        Ok(Json(json!({
            "embeddings": vec![vec![0.25f32, 0.25, 0.25, 0.25]; texts.len()],
            "revision": "remote-test-v1"
        })))
    }

    async fn analyze_faces(mut multipart: Multipart) -> Result<Json<Value>, StatusCode> {
        let mut count = 0usize;
        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|_| StatusCode::BAD_REQUEST)?
        {
            let _ = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
            count += 1;
        }
        Ok(Json(json!({
            "faces": vec![vec![json!({
                "box_xywh": [0.1, 0.1, 0.2, 0.2],
                "embedding": [0.1, 0.2, 0.3, 0.4],
                "detection_score": 0.9,
                "quality_score": 0.8
            })]; count],
            "revision": "remote-test-v1"
        })))
    }

    async fn spawn_mock() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new()
            .route("/v1/health", get(health))
            .route("/v1/capabilities", get(capabilities))
            .route("/v1/embeddings/images", post(embed_images))
            .route("/v1/embeddings/texts", post(embed_texts))
            .route("/v1/faces/analyze", post(analyze_faces));
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}"), handle)
    }

    #[tokio::test]
    async fn remote_provider_roundtrip_decoded_images() {
        let (base, handle) = spawn_mock().await;
        let mut remote =
            RemoteInferenceProvider::with_auth(Some(base), Some("test-token".to_string()));
        remote.refresh_capabilities().await.unwrap();
        assert!(remote.capabilities().image_search);
        assert!(remote.capabilities().faces);
        assert_eq!(remote.model_revision().as_deref(), Some("remote-test-v1"));

        let health = remote.health().await;
        assert!(health.healthy);

        let tiny = DynamicImage::new_rgb8(4, 4);
        let images = remote.embed_images(&[tiny.clone()]).await.unwrap();
        assert_eq!(images.len(), 1);
        assert!(images[0].as_ref().unwrap().len() == 4);

        let texts = remote.embed_texts(&["hello".to_string()]).await.unwrap();
        assert!(texts[0].as_ref().unwrap().len() == 4);

        let faces = remote.detect_and_embed_faces(&[tiny]).await.unwrap();
        assert_eq!(faces[0].as_ref().unwrap().len(), 1);

        handle.abort();
    }

    #[tokio::test]
    async fn remote_provider_rejects_unauthorized_capabilities() {
        let (base, handle) = spawn_mock().await;
        let mut remote =
            RemoteInferenceProvider::with_auth(Some(base), Some("wrong-token".to_string()));
        assert!(remote.refresh_capabilities().await.is_err());
        handle.abort();
    }
}
