mod api;
mod duplicate;
mod embedding;
mod faces;
mod models;
mod people;
mod provider;
mod scope;
mod store;
mod tasks;

pub use api::{
    api_ai_duplicates, api_ai_index, api_ai_people, api_ai_people_merge, api_ai_people_patch,
    api_ai_people_photos, api_ai_people_split, api_ai_search_similar, api_ai_search_text,
    api_ai_status,
};
pub use models::{AiGenerationStats, OwnerIndexStatus};
pub use provider::{
    AiCapabilities, AiInferenceProvider, AiItemError, AiProviderHealth, AiProviderKind,
    PipelineProgressTx, build_provider_from_settings, progress_channel,
    validate_ai_settings_update, validate_remote_url,
};
pub use store::FileAiIndexStore;
pub use tasks::queue_owner_ai_index;

pub(crate) use models::{AiFaceRecord, AiIndexedPhotoRecord, LoadedOwnerIndex};
pub(crate) use scope::collect_person_photo_paths_in_album;

use embedding::{
    calibrate_siglip_text_image_score, cosine_similarity, normalize_search_text,
    tokenize_search_text,
};
use models::{AiIndexedAlbumSummary, AiSimilarSearchResult, NormalizedAiIndexRequest};
use people::{FaceAssignment, PersonRecord};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

use super::settings::{SettingsChange, SettingsManager};

pub struct AiService {
    provider: RwLock<Arc<dyn AiInferenceProvider>>,
    store: Arc<FileAiIndexStore>,
    models_root: PathBuf,
}

impl AiService {
    pub async fn new(settings_manager: &Arc<SettingsManager>, data_path: &Path) -> Arc<Self> {
        let store = Arc::new(FileAiIndexStore::new(data_path.join("cache").join("ai")));
        let settings = settings_manager.get_all().await;
        let models_root = data_path
            .parent()
            .map(|path| path.join("models"))
            .unwrap_or_else(|| data_path.join("models"));
        let provider = build_provider_from_settings(&settings, models_root.clone()).await;
        let service = Arc::new(Self {
            provider: RwLock::new(provider),
            store,
            models_root,
        });

        let weak = Arc::downgrade(&service);
        settings_manager.register_change_handler(move |change: &SettingsChange| {
            let ai_keys = [
                "ai_provider",
                "ai_remote_url",
                "ai_batch_size",
                "ai_cpu_threads",
                "ai_enabled",
            ];
            if !change
                .changed_keys
                .iter()
                .any(|key| ai_keys.iter().any(|wanted| wanted == key))
            {
                return;
            }
            let Some(service) = weak.upgrade() else {
                return;
            };
            let settings = change.settings.clone();
            tokio::spawn(async move {
                if let Err(error) = service.reload_provider_from_settings(&settings).await {
                    eprintln!("ai provider reload failed: {error}");
                }
            });
        });

        service
    }

    pub fn store(&self) -> &Arc<FileAiIndexStore> {
        &self.store
    }

    pub async fn provider(&self) -> Arc<dyn AiInferenceProvider> {
        self.provider.read().await.clone()
    }

    pub async fn reload_provider_from_settings(&self, settings: &Value) -> Result<(), String> {
        if let Some(url) = settings
            .get("ai_remote_url")
            .and_then(|value| value.as_str())
        {
            validate_remote_url(url)?;
        }
        let provider = build_provider_from_settings(settings, self.models_root.clone()).await;
        let health = provider.health().await;
        if AiProviderKind::from_settings(settings) == AiProviderKind::Remote && !health.healthy {
            return Err(health
                .reason
                .unwrap_or_else(|| "remote provider unhealthy".to_string()));
        }
        *self.provider.write().await = provider;
        Ok(())
    }

    pub(crate) async fn bootstrap_owner_index(
        &self,
        owner: &str,
        cache_id: &str,
        request: &NormalizedAiIndexRequest,
        provider_health: &AiProviderHealth,
        indexed_albums: &[AiIndexedAlbumSummary],
        indexed_photos: &mut [AiIndexedPhotoRecord],
        embeddings: &std::collections::HashMap<String, Vec<f32>>,
        faces: &mut [AiFaceRecord],
        face_embeddings: &[Option<Vec<f32>>],
        people: &[PersonRecord],
        assignments: &[FaceAssignment],
        photos_total: usize,
    ) -> Result<String, String> {
        let provider = self.provider().await;
        let generation_id = self.store.write_generation_manifest(
            owner,
            cache_id,
            request,
            provider.capabilities(),
            provider_health,
            indexed_albums,
            indexed_photos,
            embeddings,
            faces,
            face_embeddings,
            photos_total,
        )?;
        Ok(generation_id)
    }

    pub(crate) async fn bootstrap_people_index(
        &self,
        owner: &str,
        people: &[PersonRecord],
        assignments: &[FaceAssignment],
    ) -> Result<String, String> {
        self.store
            .write_people_generation(owner, people, assignments)
    }

    pub(crate) fn search_similar_in_records(
        query_vector: &[f32],
        records: &[(AiIndexedPhotoRecord, Vec<f32>)],
        exclude_stable_id: &str,
        min_score: f32,
        limit: usize,
    ) -> Vec<AiSimilarSearchResult> {
        let mut scored = records
            .iter()
            .filter(|(record, _)| record.stable_id != exclude_stable_id)
            .map(|(record, vector)| {
                let score = cosine_similarity(query_vector, vector);
                (score, record)
            })
            .filter(|(score, _)| *score >= min_score)
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .0
                .partial_cmp(&left.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.1.stable_key.cmp(&right.1.stable_key))
        });
        scored
            .into_iter()
            .take(limit)
            .map(|(score, record)| {
                let name = Path::new(record.path.as_str())
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or(record.path.as_str())
                    .to_string();
                let date = record
                    .modified_unix_seconds
                    .map(|seconds| {
                        chrono::DateTime::from_timestamp(seconds as i64, 0)
                            .map(|value| value.format("%Y-%m-%d").to_string())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                AiSimilarSearchResult {
                    owner: record.owner.clone(),
                    album: record.album.clone(),
                    path: record.path.clone(),
                    name,
                    date,
                    size: record.size_bytes,
                    width: record.width.unwrap_or(0),
                    height: record.height.unwrap_or(0),
                    score,
                    reason: "visual_similarity".to_string(),
                }
            })
            .collect()
    }

    pub(crate) fn text_match_score(tokens: &[String], record: &AiIndexedPhotoRecord) -> f32 {
        if tokens.is_empty() {
            return 0.0;
        }
        let path_normalized = normalize_search_text(record.path.as_str());
        let album_normalized = normalize_search_text(record.album.as_str());
        let filename = Path::new(record.path.as_str())
            .file_name()
            .and_then(|value| value.to_str())
            .map(normalize_search_text)
            .unwrap_or_default();
        let mut total = 0.0_f32;
        for token in tokens {
            if filename.contains(token.as_str()) {
                total += 1.0;
            } else if album_normalized.contains(token.as_str()) {
                total += 0.75;
            } else if path_normalized.contains(token.as_str()) {
                total += 0.5;
            }
        }
        (total / tokens.len() as f32).clamp(0.0, 1.0)
    }

    pub(crate) fn search_text_in_records(
        query_text: &str,
        query_vector: &[f32],
        records: &[(AiIndexedPhotoRecord, Option<Vec<f32>>)],
        min_score: f32,
        limit: usize,
    ) -> Vec<AiSimilarSearchResult> {
        let tokens = tokenize_search_text(query_text);
        let mut scored = records
            .iter()
            .map(|(record, embedding)| {
                let text_score = Self::text_match_score(&tokens, record);
                let raw_embed = embedding
                    .as_ref()
                    .map(|vector| cosine_similarity(query_vector, vector))
                    .unwrap_or(0.0);
                // SigLIP absolute cosines are low; calibrate before blending/thresholding.
                let embed_score = calibrate_siglip_text_image_score(raw_embed);
                let score = if embedding.is_some() {
                    (0.35 * text_score + 0.65 * embed_score).clamp(0.0, 1.0)
                } else {
                    text_score
                };
                let reason = if text_score > 0.0 && embed_score > 0.0 {
                    "text_match+embedding".to_string()
                } else if embed_score > 0.0 {
                    "text_embedding".to_string()
                } else {
                    "text_match".to_string()
                };
                (score, reason, record)
            })
            .filter(|(score, _, _)| *score >= min_score)
            .collect::<Vec<_>>();
        scored.sort_by(|left, right| {
            right
                .0
                .partial_cmp(&left.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.2.stable_key.cmp(&right.2.stable_key))
        });
        scored
            .into_iter()
            .take(limit)
            .map(|(score, reason, record)| {
                let name = Path::new(record.path.as_str())
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or(record.path.as_str())
                    .to_string();
                let date = record
                    .modified_unix_seconds
                    .map(|seconds| {
                        chrono::DateTime::from_timestamp(seconds as i64, 0)
                            .map(|value| value.format("%Y-%m-%d").to_string())
                            .unwrap_or_default()
                    })
                    .unwrap_or_default();
                AiSimilarSearchResult {
                    owner: record.owner.clone(),
                    album: record.album.clone(),
                    path: record.path.clone(),
                    name,
                    date,
                    size: record.size_bytes,
                    width: record.width.unwrap_or(0),
                    height: record.height.unwrap_or(0),
                    score,
                    reason,
                }
            })
            .collect()
    }
}
