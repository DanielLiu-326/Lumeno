mod album_cache;
mod album_faces_api;
mod api;
mod duplicate;
mod embedding;
mod embedding_snapshot;
mod faces;
mod faces_config;
mod models;
mod people;
mod pipeline;
mod pipelines;
mod provider;
mod scope;
mod store;
mod tasks;

pub use album_faces_api::{
    api_album_face_crop, api_album_face_hints_get, api_album_faces_adopt_cluster,
    api_album_faces_delete, api_album_faces_get, api_album_faces_merge_clusters,
    api_album_faces_patch, api_album_faces_remove_center, api_album_faces_split,
};
pub use api::{
    api_ai_duplicates, api_ai_index, api_ai_search_similar, api_ai_search_text, api_ai_status,
};
pub(crate) use faces_config::collect_person_photo_paths_from_config_async;
pub use provider::{
    AiInferenceProvider, AiProviderHealth, AiProviderKind, build_provider_from_settings,
    validate_ai_settings_update, validate_remote_url,
};
pub use store::FileAiIndexStore;
pub use tasks::queue_owner_ai_index;

use embedding::{
    calibrate_siglip_text_image_score, cosine_similarity, embedding_debug_fingerprint,
    embedding_debug_query_preview, normalize_search_text, tokenize_search_text,
};
pub(crate) use models::{AiFaceRecord, AiIndexedPhotoRecord};
use models::{AiIndexedAlbumSummary, AiSimilarSearchResult, NormalizedAiIndexRequest};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

use super::album::helpers::load_existing_album_index;
use super::album::types::ColorLabel;
use super::common::AppState;
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
        indexed_photos: &[AiIndexedPhotoRecord],
        faces: &[AiFaceRecord],
        photos_total: usize,
    ) -> Result<String, String> {
        let provider = self.provider().await;
        let capabilities = provider.capabilities().clone();
        let owner = owner.to_string();
        let cache_id = cache_id.to_string();
        let request = request.clone();
        let provider_health = provider_health.clone();
        let indexed_albums = indexed_albums.to_vec();
        let indexed_photos = indexed_photos.to_vec();
        let faces = faces.to_vec();
        let store = Arc::clone(&self.store);
        tokio::task::spawn_blocking(move || {
            store.write_generation_manifest(
                owner.as_str(),
                cache_id.as_str(),
                &request,
                &capabilities,
                &provider_health,
                &indexed_albums,
                &indexed_photos,
                &faces,
                photos_total,
            )
        })
        .await
        .map_err(|e| format!("generation write join: {e}"))?
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
                    ..AiSimilarSearchResult::empty_meta()
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
        let mut with_embed = 0usize;
        let mut dim_mismatch = 0usize;
        let mut min_raw = f32::INFINITY;
        let mut max_raw = f32::NEG_INFINITY;
        let mut sum_raw = 0_f32;
        let mut scored = records
            .iter()
            .map(|(record, embedding)| {
                let text_score = Self::text_match_score(&tokens, record);
                let raw_embed = embedding
                    .as_ref()
                    .map(|vector| cosine_similarity(query_vector, vector))
                    .unwrap_or(0.0);
                if let Some(vector) = embedding {
                    with_embed += 1;
                    if vector.len() != query_vector.len() {
                        dim_mismatch += 1;
                    }
                    min_raw = min_raw.min(raw_embed);
                    max_raw = max_raw.max(raw_embed);
                    sum_raw += raw_embed;
                }
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
        let mean_raw = if with_embed == 0 {
            0.0
        } else {
            sum_raw / with_embed as f32
        };
        eprintln!(
            "[ai] text search q={:?} candidates={} with_embed={with_embed} dim_mismatch={dim_mismatch} query={} raw_cos min={:.4} max={:.4} mean={:.4} passed={} limit={limit} min_score={min_score:.2}",
            embedding_debug_query_preview(query_text),
            records.len(),
            embedding_debug_fingerprint(query_vector),
            if with_embed == 0 { 0.0 } else { min_raw },
            if with_embed == 0 { 0.0 } else { max_raw },
            mean_raw,
            scored.len(),
        );
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
                    ..AiSimilarSearchResult::empty_meta()
                }
            })
            .collect()
    }
}

/// Copy EXIF, rating, tags, and color labels from the album index onto search
/// hits. One index read per owner/album, same fields as `POST /api/albums/photos`.
pub(crate) async fn attach_album_index_metadata(
    cfg: &AppState,
    results: &mut [AiSimilarSearchResult],
) {
    let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (slot, result) in results.iter().enumerate() {
        if result.owner.is_empty() || result.album.is_empty() || result.path.is_empty() {
            continue;
        }
        groups
            .entry((result.owner.clone(), result.album.clone()))
            .or_default()
            .push(slot);
    }

    for ((owner, album), slots) in groups {
        let root = cfg.user_root_path(&owner);
        let Ok(Some(index)) =
            load_existing_album_index(cfg.data_path.as_path(), root.as_path(), &owner, &album)
                .await
        else {
            continue;
        };
        let by_path = index
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry))
            .collect::<HashMap<_, _>>();
        for slot in slots {
            let Some(result) = results.get_mut(slot) else {
                continue;
            };
            let Some(photo) = by_path.get(result.path.as_str()) else {
                continue;
            };
            if !photo.datetime.is_empty() {
                result.datetime = photo.datetime.clone();
            }
            if !photo.date.is_empty() {
                result.date = photo.date.clone();
            }
            result.camera_make = photo.camera_make.clone();
            result.camera_model = photo.camera_model.clone();
            result.lens = photo.lens.clone();
            result.focal_length = photo.focal_length.clone();
            result.aperture = photo.aperture.clone();
            result.shutter_speed = photo.shutter_speed.clone();
            result.iso = photo.iso;
            if photo.width > 0 {
                result.width = photo.width;
            }
            if photo.height > 0 {
                result.height = photo.height;
            }
            if photo.size > 0 {
                result.size = photo.size;
            }
            result.rating = photo.rating;
            result.color_label = photo
                .color_label
                .iter()
                .filter(|label| **label != ColorLabel::None)
                .map(|label| label.as_str().to_string())
                .collect();
            result.tags = photo.tags.clone();
        }
    }
}
