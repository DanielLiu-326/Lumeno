use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::embedding::{
    TEXT_HASH_REVISION, VISUAL_EMBED_REVISION, compute_text_hash_embedding,
    compute_visual_embeddings_rgb_batch,
};
use super::faces::{DetectedFace, FACE_DETECTOR_REVISION, detect_and_embed_faces};
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

    async fn embed_images(
        &self,
        paths: &[PathBuf],
    ) -> Result<Vec<Result<Vec<f32>, AiItemError>>, String>;

    async fn embed_texts(
        &self,
        texts: &[String],
    ) -> Result<Vec<Result<Vec<f32>, AiItemError>>, String>;

    async fn detect_and_embed_faces(
        &self,
        paths: &[PathBuf],
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
    batch_size: usize,
    mode: LocalInferenceMode,
    fallback_interim: bool,
}

impl LocalBurnProvider {
    pub fn new(model_root: PathBuf, batch_size: usize) -> Self {
        Self::with_options(model_root, batch_size, false)
    }

    pub fn with_interim_fallback(model_root: PathBuf, batch_size: usize) -> Self {
        Self::with_options(model_root, batch_size, true)
    }

    pub fn with_options(model_root: PathBuf, batch_size: usize, fallback_interim: bool) -> Self {
        let batch_size = batch_size.clamp(1, 64);
        match LocalModelRuntime::try_load(&model_root) {
            Ok(runtime) => {
                let image_dim = runtime.info.image_dim;
                let face_dim = runtime.info.face_dim;
                let revision = runtime.info.revision.clone();
                // burn-onnx graphs are exported at batch=1; keep provider batch at 1.
                let batch_size = 1;
                Self {
                    model_root,
                    capabilities: AiCapabilities {
                        image_search: true,
                        text_search: true,
                        duplicates: true,
                        faces: true,
                        max_batch_size: Some(batch_size),
                        image_embedding_dimension: Some(image_dim),
                        face_embedding_dimension: Some(face_dim),
                    },
                    revision: Some(revision),
                    batch_size,
                    mode: LocalInferenceMode::Real(runtime),
                    fallback_interim,
                }
            }
            Err(_error) if fallback_interim => Self {
                model_root,
                capabilities: AiCapabilities {
                    image_search: true,
                    text_search: true,
                    duplicates: true,
                    faces: true,
                    max_batch_size: Some(batch_size),
                    image_embedding_dimension: Some(super::embedding::INTERIM_EMBEDDING_DIM),
                    face_embedding_dimension: Some(super::embedding::INTERIM_EMBEDDING_DIM),
                },
                revision: Some(format!(
                    "{VISUAL_EMBED_REVISION}+{TEXT_HASH_REVISION}+{FACE_DETECTOR_REVISION}"
                )),
                batch_size,
                mode: LocalInferenceMode::Interim,
                fallback_interim,
            },
            // keep error message for fail-closed health
            Err(error) => Self {
                model_root,
                capabilities: AiCapabilities {
                    image_search: false,
                    text_search: false,
                    duplicates: true,
                    faces: false,
                    max_batch_size: Some(batch_size),
                    image_embedding_dimension: None,
                    face_embedding_dimension: None,
                },
                revision: None,
                batch_size,
                mode: LocalInferenceMode::Unavailable(error),
                fallback_interim,
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

    fn embed_images_blocking(
        &self,
        paths: &[PathBuf],
    ) -> Result<Vec<Result<Vec<f32>, AiItemError>>, String> {
        match &self.mode {
            LocalInferenceMode::Unavailable(_) => Ok(paths
                .iter()
                .map(|_| Err(self.unavailable_error()))
                .collect()),
            LocalInferenceMode::Real(runtime) => {
                let runtime = Arc::clone(runtime);
                let mut results = Vec::with_capacity(paths.len());
                for chunk in paths.chunks(self.batch_size.max(1)) {
                    let mut opened = Vec::with_capacity(chunk.len());
                    let mut open_errors: Vec<Option<AiItemError>> =
                        Vec::with_capacity(chunk.len());
                    for path in chunk {
                        match image::open(path) {
                            Ok(image) => {
                                open_errors.push(None);
                                opened.push(image);
                            }
                            Err(error) => {
                                open_errors.push(Some(AiItemError {
                                    code: "decode_failed".to_string(),
                                    message: error.to_string(),
                                }));
                            }
                        }
                    }
                    let batch_vectors = if opened.is_empty() {
                        Vec::new()
                    } else {
                        match runtime.embed_images(&opened) {
                            Ok(vectors) => vectors.into_iter().map(Ok).collect::<Vec<_>>(),
                            Err(message) => opened
                                .iter()
                                .map(|_| {
                                    Err(AiItemError {
                                        code: "provider_rejected".to_string(),
                                        message: message.clone(),
                                    })
                                })
                                .collect(),
                        }
                    };
                    let mut batch_iter = batch_vectors.into_iter();
                    for open_error in open_errors {
                        if let Some(error) = open_error {
                            results.push(Err(error));
                            continue;
                        }
                        results.push(batch_iter.next().unwrap_or_else(|| {
                            Err(AiItemError {
                                code: "provider_rejected".to_string(),
                                message: "missing batch embedding".to_string(),
                            })
                        }));
                    }
                }
                Ok(results)
            }
            LocalInferenceMode::Interim => self.embed_images_interim(paths),
        }
    }

    fn embed_images_interim(
        &self,
        paths: &[PathBuf],
    ) -> Result<Vec<Result<Vec<f32>, AiItemError>>, String> {
        let mut results = Vec::with_capacity(paths.len());
        for chunk in paths.chunks(self.batch_size.max(1)) {
            let mut opened = Vec::with_capacity(chunk.len());
            let mut open_errors: Vec<Option<AiItemError>> = Vec::with_capacity(chunk.len());
            for path in chunk {
                match image::open(path) {
                    Ok(image) => {
                        open_errors.push(None);
                        opened.push(image.to_rgb8());
                    }
                    Err(error) => {
                        open_errors.push(Some(AiItemError {
                            code: "decode_failed".to_string(),
                            message: error.to_string(),
                        }));
                    }
                }
            }

            let mut batch_vectors = Vec::new();
            if !opened.is_empty() {
                let refs = opened.iter().collect::<Vec<_>>();
                match compute_visual_embeddings_rgb_batch(&refs) {
                    Ok(vectors) => batch_vectors = vectors,
                    Err(_) => {
                        for rgb in &opened {
                            match compute_visual_embedding_from_prepared(rgb) {
                                Ok(vector) => batch_vectors.push(vector),
                                Err(_) => batch_vectors.push(Vec::new()),
                            }
                        }
                    }
                }
            }

            let mut batch_iter = batch_vectors.into_iter();
            for open_error in open_errors {
                if let Some(error) = open_error {
                    results.push(Err(error));
                    continue;
                }
                match batch_iter.next() {
                    Some(vector) if !vector.is_empty() => results.push(Ok(vector)),
                    Some(_) => results.push(Err(AiItemError {
                        code: "invalid_output".to_string(),
                        message: "empty embedding".to_string(),
                    })),
                    None => results.push(Err(AiItemError {
                        code: "provider_rejected".to_string(),
                        message: "missing batch embedding".to_string(),
                    })),
                }
            }
        }
        Ok(results)
    }

    fn embed_texts_blocking(
        &self,
        texts: &[String],
    ) -> Result<Vec<Result<Vec<f32>, AiItemError>>, String> {
        match &self.mode {
            LocalInferenceMode::Unavailable(_) => Ok(texts
                .iter()
                .map(|_| Err(self.unavailable_error()))
                .collect()),
            LocalInferenceMode::Real(runtime) => match runtime.embed_texts(texts) {
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
            },
            LocalInferenceMode::Interim => Ok(texts
                .iter()
                .map(|text| {
                    compute_text_hash_embedding(text).map_err(|message| AiItemError {
                        code: "invalid_output".to_string(),
                        message,
                    })
                })
                .collect()),
        }
    }

    fn detect_faces_blocking(
        &self,
        paths: &[PathBuf],
    ) -> Result<Vec<Result<Vec<DetectedFace>, AiItemError>>, String> {
        match &self.mode {
            LocalInferenceMode::Unavailable(_) => Ok(paths
                .iter()
                .map(|_| Err(self.unavailable_error()))
                .collect()),
            LocalInferenceMode::Real(runtime) => {
                use rayon::prelude::*;
                let runtime = Arc::clone(runtime);
                Ok(paths
                    .par_iter()
                    .map(|path| {
                        let image = image::open(path).map_err(|error| AiItemError {
                            code: "decode_failed".to_string(),
                            message: error.to_string(),
                        })?;
                        runtime
                            .detect_and_embed_faces(&image)
                            .map_err(|message| AiItemError {
                                code: "provider_rejected".to_string(),
                                message,
                            })
                    })
                    .collect())
            }
            LocalInferenceMode::Interim => {
                use rayon::prelude::*;
                Ok(paths
                    .par_iter()
                    .map(|path| {
                        detect_and_embed_faces(path.as_path()).map_err(|message| AiItemError {
                            code: "provider_rejected".to_string(),
                            message,
                        })
                    })
                    .collect())
            }
        }
    }
}

fn compute_visual_embedding_from_prepared(rgb: &image::RgbImage) -> Result<Vec<f32>, String> {
    super::embedding::compute_visual_embedding_rgb(rgb)
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
                Some(
                    "ai_fallback_interim=true; using Burn Flex DCT / heuristic faces".to_string(),
                ),
            ),
            LocalInferenceMode::Unavailable(error) => (
                false,
                Some(format!(
                    "local models unavailable (fail-closed): {error}"
                )),
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
        paths: &[PathBuf],
    ) -> Result<Vec<Result<Vec<f32>, AiItemError>>, String> {
        let paths = paths.to_vec();
        let batch_size = self.batch_size;
        let model_root = self.model_root.clone();
        let fallback_interim = self.fallback_interim;
        let mode_hint = match &self.mode {
            LocalInferenceMode::Real(runtime) => LocalModeHint::Real(Arc::clone(runtime)),
            LocalInferenceMode::Interim => LocalModeHint::Interim,
            LocalInferenceMode::Unavailable(error) => LocalModeHint::Unavailable(error.clone()),
        };
        tokio::task::spawn_blocking(move || {
            let provider = LocalBurnProvider {
                model_root,
                capabilities: AiCapabilities::default(),
                revision: None,
                batch_size,
                mode: match mode_hint {
                    LocalModeHint::Real(runtime) => LocalInferenceMode::Real(runtime),
                    LocalModeHint::Interim => LocalInferenceMode::Interim,
                    LocalModeHint::Unavailable(error) => LocalInferenceMode::Unavailable(error),
                },
                fallback_interim,
            };
            provider.embed_images_blocking(&paths)
        })
        .await
        .map_err(|error| format!("embed_images join failed: {error}"))?
    }

    async fn embed_texts(
        &self,
        texts: &[String],
    ) -> Result<Vec<Result<Vec<f32>, AiItemError>>, String> {
        let texts = texts.to_vec();
        let batch_size = self.batch_size;
        let model_root = self.model_root.clone();
        let fallback_interim = self.fallback_interim;
        let mode_hint = match &self.mode {
            LocalInferenceMode::Real(runtime) => LocalModeHint::Real(Arc::clone(runtime)),
            LocalInferenceMode::Interim => LocalModeHint::Interim,
            LocalInferenceMode::Unavailable(error) => LocalModeHint::Unavailable(error.clone()),
        };
        tokio::task::spawn_blocking(move || {
            let provider = LocalBurnProvider {
                model_root,
                capabilities: AiCapabilities::default(),
                revision: None,
                batch_size,
                mode: match mode_hint {
                    LocalModeHint::Real(runtime) => LocalInferenceMode::Real(runtime),
                    LocalModeHint::Interim => LocalInferenceMode::Interim,
                    LocalModeHint::Unavailable(error) => LocalInferenceMode::Unavailable(error),
                },
                fallback_interim,
            };
            provider.embed_texts_blocking(&texts)
        })
        .await
        .map_err(|error| format!("embed_texts join failed: {error}"))?
    }

    async fn detect_and_embed_faces(
        &self,
        paths: &[PathBuf],
    ) -> Result<Vec<Result<Vec<DetectedFace>, AiItemError>>, String> {
        let paths = paths.to_vec();
        let batch_size = self.batch_size;
        let model_root = self.model_root.clone();
        let fallback_interim = self.fallback_interim;
        let mode_hint = match &self.mode {
            LocalInferenceMode::Real(runtime) => LocalModeHint::Real(Arc::clone(runtime)),
            LocalInferenceMode::Interim => LocalModeHint::Interim,
            LocalInferenceMode::Unavailable(error) => LocalModeHint::Unavailable(error.clone()),
        };
        tokio::task::spawn_blocking(move || {
            let provider = LocalBurnProvider {
                model_root,
                capabilities: AiCapabilities::default(),
                revision: None,
                batch_size,
                mode: match mode_hint {
                    LocalModeHint::Real(runtime) => LocalInferenceMode::Real(runtime),
                    LocalModeHint::Interim => LocalInferenceMode::Interim,
                    LocalModeHint::Unavailable(error) => LocalInferenceMode::Unavailable(error),
                },
                fallback_interim,
            };
            provider.detect_faces_blocking(&paths)
        })
        .await
        .map_err(|error| format!("detect_and_embed_faces join failed: {error}"))?
    }
}

enum LocalModeHint {
    Real(Arc<LocalModelRuntime>),
    Interim,
    Unavailable(String),
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
        self.token
            .as_ref()
            .map(|token| format!("Bearer {token}"))
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
        paths: &[PathBuf],
    ) -> Result<Vec<Result<Vec<f32>, AiItemError>>, String> {
        let Some(base) = self.base_url() else {
            return Err("remote url missing".to_string());
        };
        let mut form = reqwest::multipart::Form::new();
        for (index, path) in paths.iter().enumerate() {
            let bytes = tokio::fs::read(path)
                .await
                .map_err(|error| format!("read image failed: {error}"))?;
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name(
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("image.bin")
                        .to_string(),
                )
                .mime_str("application/octet-stream")
                .map_err(|error| format!("multipart part failed: {error}"))?;
            form = form.part(format!("images[{index}]"), part);
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
            return Err(format!("image embedding status {}", response.status()));
        }
        let body: RemoteEmbeddingResponse = response
            .json()
            .await
            .map_err(|error| format!("image embedding decode failed: {error}"))?;
        if body.embeddings.len() != paths.len() {
            return Err(format!(
                "image embedding count mismatch: got {} expected {}",
                body.embeddings.len(),
                paths.len()
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
        paths: &[PathBuf],
    ) -> Result<Vec<Result<Vec<DetectedFace>, AiItemError>>, String> {
        let Some(base) = self.base_url() else {
            return Err("remote url missing".to_string());
        };
        let mut form = reqwest::multipart::Form::new();
        for (index, path) in paths.iter().enumerate() {
            let bytes = tokio::fs::read(path)
                .await
                .map_err(|error| format!("read image failed: {error}"))?;
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name(
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("image.bin")
                        .to_string(),
                )
                .mime_str("application/octet-stream")
                .map_err(|error| format!("multipart part failed: {error}"))?;
            form = form.part(format!("images[{index}]"), part);
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
            return Err(format!("face analyze status {}", response.status()));
        }
        let body: RemoteFaceResponse = response
            .json()
            .await
            .map_err(|error| format!("face analyze decode failed: {error}"))?;
        if body.faces.len() != paths.len() {
            return Err(format!(
                "face analyze count mismatch: got {} expected {}",
                body.faces.len(),
                paths.len()
            ));
        }
        Ok(body
            .faces
            .into_iter()
            .map(|faces| {
                Ok(faces
                    .into_iter()
                    .map(|face| DetectedFace {
                        box_xywh: face.box_xywh,
                        landmarks: face.landmarks.unwrap_or([[0.0; 2]; 5]),
                        detection_score: face.detection_score.unwrap_or(0.0),
                        quality_score: face.quality_score.unwrap_or(0.0),
                        embedding: face.embedding,
                    })
                    .collect())
            })
            .collect())
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
        .unwrap_or(4);
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
    use axum::{Json, Router};
    use axum::extract::Multipart;
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};
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
        let embeds = provider
            .embed_texts(&["hello".to_string()])
            .await
            .unwrap();
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
        while let Some(field) = multipart.next_field().await.map_err(|_| StatusCode::BAD_REQUEST)? {
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

    async fn faces_analyze(mut multipart: Multipart) -> Result<Json<Value>, StatusCode> {
        let mut count = 0usize;
        while let Some(field) = multipart.next_field().await.map_err(|_| StatusCode::BAD_REQUEST)? {
            let _ = field.bytes().await.map_err(|_| StatusCode::BAD_REQUEST)?;
            count += 1;
        }
        let faces = (0..count)
            .map(|_| {
                json!([{
                    "box_xywh": [0.1, 0.1, 0.2, 0.2],
                    "detection_score": 0.9,
                    "quality_score": 0.8,
                    "embedding": [0.1, 0.2, 0.3, 0.4]
                }])
            })
            .collect::<Vec<_>>();
        Ok(Json(json!({
            "faces": faces,
            "revision": "remote-test-v1"
        })))
    }

    async fn spawn_mock() -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new()
            .route("/v1/health", get(health))
            .route("/v1/capabilities", get(capabilities))
            .route("/v1/embeddings/images", post(embed_images))
            .route("/v1/embeddings/texts", post(embed_texts))
            .route("/v1/faces/analyze", post(faces_analyze));
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
    async fn validate_ai_settings_requires_remote_url() {
        assert!(validate_ai_settings_update(&json!({
            "ai_provider": "remote",
            "ai_remote_url": ""
        }))
        .is_err());
        assert!(validate_ai_settings_update(&json!({
            "ai_provider": "remote",
            "ai_remote_url": "ftp://bad"
        }))
        .is_err());
        assert!(validate_ai_settings_update(&json!({
            "ai_provider": "remote",
            "ai_remote_url": "https://ai.example.com"
        }))
        .is_ok());
    }

    #[tokio::test]
    async fn remote_provider_contract_roundtrip() {
        let (base, handle) = spawn_mock().await;
        let mut remote =
            RemoteInferenceProvider::with_auth(Some(base), Some("test-token".to_string()));
        remote.refresh_capabilities().await.unwrap();
        assert!(remote.capabilities().image_search);
        assert!(remote.capabilities().faces);
        assert_eq!(remote.model_revision().as_deref(), Some("remote-test-v1"));

        let health = remote.health().await;
        assert!(health.healthy);

        let dir = tempfile_dir();
        let image_path = dir.join("a.png");
        write_tiny_png(&image_path);

        let images = remote.embed_images(&[image_path.clone()]).await.unwrap();
        assert_eq!(images.len(), 1);
        assert!(images[0].as_ref().unwrap().len() == 4);

        let texts = remote
            .embed_texts(&["hello".to_string()])
            .await
            .unwrap();
        assert!(texts[0].as_ref().unwrap().len() == 4);

        let faces = remote
            .detect_and_embed_faces(&[image_path])
            .await
            .unwrap();
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

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("album-ai-remote-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_tiny_png(path: &Path) {
        // 1x1 PNG
        let bytes: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x05, 0xFE,
            0xD4, 0xEF, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::fs::write(path, bytes).unwrap();
    }
}
