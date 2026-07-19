use async_trait::async_trait;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use sha2::{Digest, Sha256};

use super::super::album::{collect_accessible_ai_scan_targets, collect_album_image_paths};
use super::super::common::AppState;
use super::duplicate::{compute_photo_phash, compute_photo_sha256};
use super::faces::FACE_DETECTOR_REVISION;
use super::super::i18n::{tr, trf};
use super::models::{
    AiFaceRecord, AiIndexedAlbumSummary, AiIndexedPhotoRecord, LoadedOwnerIndex,
    NormalizedAiIndexRequest,
};
use super::people::{
    ClusterFace, DEFAULT_CLUSTER_EPS, DEFAULT_CLUSTER_MIN_SAMPLES, rebuild_people_assignments,
};
use super::provider::{AiInferenceProvider, AiItemError};
use super::super::tasks::{Task, TaskBase, TaskStatus};

pub(crate) fn pending_or_skipped(features: &[String], feature: &str) -> String {
    if features.iter().any(|value| value == feature) {
        "pending".to_string()
    } else {
        "skipped".to_string()
    }
}

pub(crate) fn build_photo_content_signature(size_bytes: u64, modified_unix_seconds: Option<u64>) -> String {
    match modified_unix_seconds {
        Some(modified) => format!("{size_bytes}:{modified}"),
        None => format!("{size_bytes}:unknown"),
    }
}

pub(crate) fn resolve_indexed_photo_path(cfg: &AppState, record: &AiIndexedPhotoRecord) -> Option<PathBuf> {
    let prefix = format!("albums/{}/", record.album);
    let relative = record.path.strip_prefix(&prefix)?;
    let user_root = cfg.user_root_path(&record.owner);
    let path = user_root
        .join("albums")
        .join(&record.source_key)
        .join(relative);
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

pub(crate) async fn build_faces_for_photos(
    cfg: &AppState,
    provider: &dyn AiInferenceProvider,
    faces_requested: bool,
    indexed_photos: &mut [AiIndexedPhotoRecord],
    previous_index: &Option<LoadedOwnerIndex>,
    photo_paths: &HashMap<String, PathBuf>,
) -> (Vec<AiFaceRecord>, Vec<Option<Vec<f32>>>) {
    let mut face_records = Vec::new();
    let mut face_embeddings = Vec::new();
    if !faces_requested {
        return (face_records, face_embeddings);
    }

    let mut detect_jobs = Vec::new();
    for photo in indexed_photos.iter_mut() {
        if photo.face_embedding_state == "skipped" {
            continue;
        }

        let reuse_faces = previous_index.as_ref().and_then(|loaded| {
            if photo.face_embedding_state != "ready" {
                return None;
            }
            let face_ids = loaded.faces_by_stable_key.get(&photo.stable_key)?;
            Some(
                face_ids
                    .iter()
                    .filter_map(|face_id| loaded.faces.get(face_id))
                    .cloned()
                    .collect::<Vec<_>>(),
            )
        });

        if let Some(mut reused_faces) = reuse_faces {
            photo.face_embedding_state = "ready".to_string();
            for mut face in reused_faces.drain(..) {
                face.photo_id = photo.photo_id;
                face.stable_key = photo.stable_key.clone();
                let embedding = previous_index
                    .as_ref()
                    .and_then(|loaded| loaded.face_embeddings.get(&face.face_id))
                    .cloned();
                face.embedding_state = if embedding.is_some() {
                    "ready".to_string()
                } else {
                    "error".to_string()
                };
                face_records.push(face);
                face_embeddings.push(embedding);
            }
            continue;
        }

        let Some(photo_path) = photo_paths
            .get(&photo.stable_key)
            .cloned()
            .or_else(|| resolve_indexed_photo_path(cfg, photo))
        else {
            photo.face_embedding_state = "error".to_string();
            photo
                .record_error
                .get_or_insert_with(|| "photo path missing for face detection".to_string());
            continue;
        };

        detect_jobs.push((photo.photo_id, photo.stable_key.clone(), photo_path));
    }

    let batch_size = provider
        .capabilities()
        .max_batch_size
        .unwrap_or(4)
        .clamp(1, 64);
    let mut detected_results = Vec::with_capacity(detect_jobs.len());
    for chunk in detect_jobs.chunks(batch_size) {
        let paths = chunk
            .iter()
            .map(|(_, _, path)| path.clone())
            .collect::<Vec<_>>();
        match provider.detect_and_embed_faces(&paths).await {
            Ok(results) => detected_results.extend(results),
            Err(error) => {
                for _ in paths {
                    detected_results.push(Err(AiItemError {
                        code: "provider_rejected".to_string(),
                        message: error.clone(),
                    }));
                }
            }
        }
    }

    let mut photo_by_id = indexed_photos
        .iter_mut()
        .map(|photo| (photo.photo_id, photo))
        .collect::<HashMap<_, _>>();

    for ((photo_id, stable_key, _), result) in detect_jobs.into_iter().zip(detected_results) {
        let Some(photo) = photo_by_id.get_mut(&photo_id) else {
            continue;
        };
        match result {
            Ok(detected_faces) => {
                photo.face_embedding_state = "ready".to_string();
                for detected in detected_faces {
                    face_records.push(AiFaceRecord {
                        face_id: Uuid::new_v4().to_string(),
                        photo_id,
                        stable_key: stable_key.clone(),
                        box_xywh: detected.box_xywh,
                        landmarks: detected.landmarks,
                        detection_score: detected.detection_score,
                        quality_score: detected.quality_score,
                        detector_revision: FACE_DETECTOR_REVISION.to_string(),
                        embedding_state: "ready".to_string(),
                        embedding_offset: None,
                    });
                    face_embeddings.push(Some(detected.embedding));
                }
            }
            Err(error) => {
                photo.face_embedding_state = "error".to_string();
                photo.record_error = Some(error.to_string());
            }
        }
    }

    (face_records, face_embeddings)
}

pub(crate) fn build_photo_stable_identity(owner: &str, source_key: &str, path: &str) -> (String, String) {
    let stable_key = format!("{owner}|{source_key}|{path}");
    let mut hasher = Sha256::new();
    hasher.update(stable_key.as_bytes());
    let stable_id = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    (stable_key, stable_id)
}

pub(crate) fn read_photo_dimensions(path: &Path) -> Option<(u32, u32)> {
    image::image_dimensions(path).ok()
}


pub struct AiIndexTask {
    base: TaskBase,
    cfg: Arc<AppState>,
    locale: String,
    request: NormalizedAiIndexRequest,
    cancel: AtomicBool,
}

impl AiIndexTask {
    pub(crate) fn new(
        name: String,
        owner: String,
        cfg: Arc<AppState>,
        locale: String,
        request: NormalizedAiIndexRequest,
    ) -> Self {
        Self {
            base: TaskBase::new(name, owner),
            cfg,
            locale,
            request,
            cancel: AtomicBool::new(false),
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    async fn abort_if_cancelled(&self) -> bool {
        if !self.is_cancelled() {
            return false;
        }
        self.base
            .set_cur_task("cancelled before publishing index".to_string())
            .await;
        self.base.set_status(TaskStatus::Cancelled).await;
        true
    }
}

#[async_trait]
impl Task for AiIndexTask {
    fn base(&self) -> &TaskBase {
        &self.base
    }

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Running).await;
        self.base
            .set_cur_task("scanning accessible albums".to_string())
            .await;

        let _permit = self.cfg.ai_limiter.acquire().await;
        let provider = self.cfg.ai_service.provider().await;
        let provider_health = provider.health().await;
        let provider_revision = provider_health.revision.clone();
        let duplicates_requested = self
            .request
            .features
            .iter()
            .any(|value| value == "duplicates");
        let image_requested = self.request.features.iter().any(|value| value == "image");
        let faces_requested = self.request.features.iter().any(|value| value == "faces");
        if self.abort_if_cancelled().await {
            return;
        }
        let previous_index = self
            .cfg
            .ai_service
            .store()
            .load_current_index(self.base.owner.as_str());

        let targets = match collect_accessible_ai_scan_targets(
            self.cfg.as_ref(),
            self.base.owner.as_str(),
            self.request.album.as_deref(),
            self.locale.as_str(),
        )
        .await
        {
            Ok(targets) => targets,
            Err(error) => {
                self.base.set_error_msg(error.to_string()).await;
                self.base.set_status(TaskStatus::Failed).await;
                return;
            }
        };
        let targets = targets
            .into_iter()
            .filter(|target| target.owner == self.base.owner)
            .collect::<Vec<_>>();
        if self.abort_if_cancelled().await {
            return;
        }
        if self.request.album.is_some() && targets.is_empty() {
            self.base
                .set_error_msg(tr(self.locale.as_str(), "error.ai_index_owner_only"))
                .await;
            self.base.set_status(TaskStatus::Failed).await;
            return;
        }

        self.base.set_total_count(targets.len().max(1));

        let mut indexed_albums = Vec::with_capacity(targets.len());
        let mut indexed_photos = Vec::new();
        let mut indexed_embeddings = Vec::new();
        let mut pending_image_embeds = Vec::<(String, PathBuf)>::new();
        let mut photo_disk_paths = HashMap::<String, PathBuf>::new();
        let mut photos_total = 0usize;
        let indexed_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        for target in targets {
            if self.abort_if_cancelled().await {
                return;
            }
            self.base
                .set_cur_task(format!("scanning {}", target.album_name))
                .await;
            let photo_paths = collect_album_image_paths(target.album_dir.as_path()).await;
            let photo_count = photo_paths.len();
            photos_total += photo_count;
            let source_key = target.album_name.clone();
            for photo_path in photo_paths {
                let Ok(metadata) = std::fs::metadata(&photo_path) else {
                    continue;
                };
                let relative_path = photo_path
                    .strip_prefix(target.album_dir.as_path())
                    .ok()
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|| {
                        photo_path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or_default()
                            .to_string()
                    });
                let display_path = format!("albums/{}/{relative_path}", target.album_name);
                let legacy_stable_key = format!("{source_key}|{display_path}");
                let (stable_key, stable_id) = build_photo_stable_identity(
                    target.owner.as_str(),
                    source_key.as_str(),
                    display_path.as_str(),
                );
                let modified_unix_seconds = metadata
                    .modified()
                    .ok()
                    .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                    .map(|value| value.as_secs());
                let content_signature =
                    build_photo_content_signature(metadata.len(), modified_unix_seconds);
                let current_dimensions = read_photo_dimensions(&photo_path);
                let previous_record = previous_index.as_ref().and_then(|loaded| {
                    loaded
                        .photos
                        .get(&stable_key)
                        .or_else(|| loaded.photos.get(&legacy_stable_key))
                });
                let reusable_record = previous_record.filter(|record| {
                    !record.content_signature.is_empty()
                        && record.content_signature == content_signature
                        && record.provider_revision == provider_revision
                });
                let (width, height) = match reusable_record {
                    Some(record) => (
                        record.width.or(current_dimensions.map(|value| value.0)),
                        record.height.or(current_dimensions.map(|value| value.1)),
                    ),
                    None => (
                        current_dimensions.map(|value| value.0),
                        current_dimensions.map(|value| value.1),
                    ),
                };
                let scan_state = if reusable_record.is_some() {
                    "reused".to_string()
                } else {
                    "discovered".to_string()
                };
                let (
                    duplicate_hash_state,
                    duplicate_sha256_hex,
                    duplicate_phash_u64,
                    duplicate_phash_valid,
                    record_error,
                ) = if let Some(record) = reusable_record
                    && record.duplicate_hash_state == "ready"
                    && record.duplicate_sha256_hex.is_some()
                    && record.duplicate_phash_valid
                    && record.duplicate_phash_u64.is_some()
                {
                    (
                        "ready".to_string(),
                        record.duplicate_sha256_hex.clone(),
                        record.duplicate_phash_u64,
                        record.duplicate_phash_valid,
                        record.record_error.clone(),
                    )
                } else if duplicates_requested {
                    match compute_photo_sha256(&photo_path) {
                        Ok(sha256_hex) => match compute_photo_phash(&photo_path) {
                            Ok(phash) => (
                                "ready".to_string(),
                                Some(sha256_hex),
                                Some(phash),
                                true,
                                None,
                            ),
                            Err(error) => (
                                "ready".to_string(),
                                Some(sha256_hex),
                                None,
                                false,
                                Some(error),
                            ),
                        },
                        Err(error) => ("error".to_string(), None, None, false, Some(error)),
                    }
                } else {
                    (
                        "skipped".to_string(),
                        None,
                        None,
                        false,
                        reusable_record.and_then(|record| record.record_error.clone()),
                    )
                };
                let reused_embedding = reusable_record.and_then(|record| {
                    previous_index
                        .as_ref()
                        .and_then(|loaded| {
                            loaded
                                .embeddings
                                .get(&record.stable_key)
                                .or_else(|| loaded.embeddings.get(&stable_key))
                                .or_else(|| loaded.embeddings.get(&legacy_stable_key))
                        })
                        .cloned()
                        .filter(|_| record.image_embedding_state == "ready")
                });
                let (image_embedding_state, embedding) = if let Some(vector) = reused_embedding {
                    ("ready".to_string(), Some(vector))
                } else if image_requested {
                    pending_image_embeds.push((stable_key.clone(), photo_path.clone()));
                    ("pending".to_string(), None)
                } else {
                    (
                        reusable_record
                            .map(|record| record.image_embedding_state.clone())
                            .unwrap_or_else(|| "skipped".to_string()),
                        None,
                    )
                };
                let face_embedding_state = reusable_record
                    .map(|record| record.face_embedding_state.clone())
                    .unwrap_or_else(|| pending_or_skipped(&self.request.features, "faces"));
                let reused_from_generation = reusable_record.and_then(|record| {
                    previous_index
                        .as_ref()
                        .map(|loaded| loaded.generation.clone())
                        .or_else(|| record.reused_from_generation.clone())
                });
                photo_disk_paths.insert(stable_key.clone(), photo_path);
                indexed_photos.push(AiIndexedPhotoRecord {
                    photo_id: indexed_photos.len(),
                    owner: target.owner.clone(),
                    album: target.album_name.clone(),
                    source_key: source_key.clone(),
                    path: display_path,
                    stable_key,
                    stable_id,
                    size_bytes: metadata.len(),
                    modified_unix_seconds,
                    content_signature,
                    width,
                    height,
                    provider_revision: provider_revision.clone(),
                    scan_state,
                    image_embedding_state,
                    image_embedding_offset: None,
                    duplicate_hash_state,
                    duplicate_sha256_hex,
                    duplicate_sha256_offset: None,
                    duplicate_phash_u64,
                    duplicate_phash_offset: None,
                    duplicate_phash_valid,
                    face_embedding_state,
                    reused_from_generation,
                    record_error,
                    indexed_at_unix_ms,
                });
                indexed_embeddings.push(embedding);
            }
            indexed_albums.push(AiIndexedAlbumSummary {
                source_key,
                owner: target.owner,
                album: target.album_name,
                photo_count,
            });
            self.base.add_processed_count(1);
        }

        if self.request.album.is_some() {
            let scanned_sources = indexed_albums
                .iter()
                .map(|album| album.source_key.clone())
                .collect::<BTreeSet<_>>();
            if let Some(previous) = previous_index.as_ref() {
                let mut retained_counts = HashMap::<String, usize>::new();
                for mut record in previous
                    .photos
                    .values()
                    .filter(|record| {
                        record.owner == self.base.owner
                            && !scanned_sources.contains(&record.source_key)
                    })
                    .cloned()
                {
                    *retained_counts.entry(record.album.clone()).or_default() += 1;
                    let embedding = previous.embeddings.get(&record.stable_key).cloned();
                    let (stable_key, stable_id) = build_photo_stable_identity(
                        record.owner.as_str(),
                        record.source_key.as_str(),
                        record.path.as_str(),
                    );
                    record.stable_key = stable_key;
                    record.stable_id = stable_id;
                    record.scan_state = "reused".to_string();
                    record.reused_from_generation = Some(previous.generation.clone());
                    record.duplicate_sha256_offset = None;
                    record.duplicate_phash_offset = None;
                    record.image_embedding_offset = None;
                    indexed_embeddings.push(embedding);
                    indexed_photos.push(record);
                }
                for (album, photo_count) in retained_counts {
                    indexed_albums.push(AiIndexedAlbumSummary {
                        owner: self.base.owner.clone(),
                        source_key: album.clone(),
                        album,
                        photo_count,
                    });
                    photos_total += photo_count;
                }
            }
        }

        indexed_albums.sort_by(|left, right| left.source_key.cmp(&right.source_key));
        let mut paired = indexed_photos
            .into_iter()
            .zip(indexed_embeddings.into_iter())
            .collect::<Vec<_>>();
        paired.sort_by(|left, right| left.0.stable_key.cmp(&right.0.stable_key));
        let mut indexed_photos = Vec::with_capacity(paired.len());
        let mut indexed_embeddings = Vec::with_capacity(paired.len());
        for (photo_id, (mut photo, embedding)) in paired.into_iter().enumerate() {
            photo.photo_id = photo_id;
            indexed_photos.push(photo);
            indexed_embeddings.push(embedding);
        }

        if self.abort_if_cancelled().await {
            return;
        }

        if image_requested && !pending_image_embeds.is_empty() {
            self.base.set_cur_task("embedding images".to_string()).await;
            let batch_size = provider
                .capabilities()
                .max_batch_size
                .unwrap_or(4)
                .clamp(1, 64);
            let mut photo_by_key = indexed_photos
                .iter_mut()
                .map(|photo| (photo.stable_key.clone(), photo))
                .collect::<HashMap<_, _>>();
            for chunk in pending_image_embeds.chunks(batch_size) {
                if self.abort_if_cancelled().await {
                    return;
                }
                let paths = chunk
                    .iter()
                    .map(|(_, path)| path.clone())
                    .collect::<Vec<_>>();
                match provider.embed_images(&paths).await {
                    Ok(results) => {
                        for ((stable_key, _), result) in chunk.iter().zip(results.into_iter()) {
                            let Some(photo) = photo_by_key.get_mut(stable_key) else {
                                continue;
                            };
                            let photo_id = photo.photo_id;
                            match result {
                                Ok(vector) => {
                                    photo.image_embedding_state = "ready".to_string();
                                    if let Some(slot) = indexed_embeddings.get_mut(photo_id) {
                                        *slot = Some(vector);
                                    }
                                }
                                Err(error) => {
                                    photo.image_embedding_state = "error".to_string();
                                    photo.record_error = Some(error.to_string());
                                }
                            }
                        }
                    }
                    Err(error) => {
                        for (stable_key, _) in chunk {
                            if let Some(photo) = photo_by_key.get_mut(stable_key) {
                                photo.image_embedding_state = "error".to_string();
                                photo.record_error = Some(error.clone());
                            }
                        }
                    }
                }
            }
        }

        if self.abort_if_cancelled().await {
            return;
        }

        self.base
            .set_cur_task("detecting and embedding faces".to_string())
            .await;

        let (mut face_records, face_embeddings, people, assignments) = if faces_requested {
            let (face_records, face_embeddings) = build_faces_for_photos(
                self.cfg.as_ref(),
                provider.as_ref(),
                faces_requested,
                &mut indexed_photos,
                &previous_index,
                &photo_disk_paths,
            )
            .await;
            self.base.set_cur_task("clustering people".to_string()).await;
            let cluster_faces = face_records
                .iter()
                .zip(face_embeddings.iter())
                .filter_map(|(face, embedding)| {
                    embedding.as_ref().map(|vector| ClusterFace {
                        face_id: face.face_id.clone(),
                        embedding: vector.clone(),
                    })
                })
                .collect::<Vec<_>>();
            let previous_people = previous_index
                .as_ref()
                .map(|loaded| loaded.people.values().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            let previous_assignments = previous_index
                .as_ref()
                .map(|loaded| loaded.assignments.values().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            let (people, assignments) = rebuild_people_assignments(
                &cluster_faces,
                previous_people,
                previous_assignments,
                DEFAULT_CLUSTER_EPS,
                DEFAULT_CLUSTER_MIN_SAMPLES,
            );
            (face_records, face_embeddings, people, assignments)
        } else if let Some(previous) = previous_index.as_ref() {
            let face_records = previous.faces.values().cloned().collect::<Vec<_>>();
            let face_embeddings = face_records
                .iter()
                .map(|face| previous.face_embeddings.get(&face.face_id).cloned())
                .collect::<Vec<_>>();
            let people = previous.people.values().cloned().collect::<Vec<_>>();
            let assignments = previous.assignments.values().cloned().collect::<Vec<_>>();
            (face_records, face_embeddings, people, assignments)
        } else {
            (Vec::new(), Vec::new(), Vec::new(), Vec::new())
        };

        if self.abort_if_cancelled().await {
            return;
        }

        self.base
            .set_cur_task("writing ai generation manifest".to_string())
            .await;

        match self
            .cfg
            .ai_service
            .bootstrap_owner_index(
                self.base.owner.as_str(),
                &self.request,
                &provider_health,
                &indexed_albums,
                &mut indexed_photos,
                &indexed_embeddings,
                &mut face_records,
                &face_embeddings,
                &people,
                &assignments,
                photos_total,
            )
            .await
        {
            Ok(generation_id) => {
                self.base
                    .set_cur_task(format!(
                        "committed generation {generation_id} with {photos_total} photos"
                    ))
                    .await;
                self.base.set_status(TaskStatus::Finished).await;
            }
            Err(error) => {
                self.base.set_error_msg(error).await;
                self.base.set_status(TaskStatus::Failed).await;
            }
        }
    }

    async fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn cancellable(&self) -> bool {
        true
    }

    fn deduplication_key(&self) -> Option<String> {
        let album = self.request.album.as_deref().unwrap_or("all");
        let features = self.request.features.join("+");
        Some(format!(
            "ai-index:{}:{}:{}:{}",
            self.base.owner, album, self.request.mode, features
        ))
    }
}

pub async fn queue_owner_ai_index(
    cfg: Arc<AppState>,
    owner: String,
    album: Option<String>,
    locale: &str,
) -> Option<Uuid> {
    let ai_enabled = cfg.settings_manager.ai_enabled().await;
    let has_index = cfg.ai_service.store().owner_has_current_index(owner.as_str());
    if !ai_enabled && !has_index {
        return None;
    }

    let request = NormalizedAiIndexRequest {
        album: album.clone(),
        mode: "incremental".to_string(),
        features: vec![
            "image".to_string(),
            "duplicates".to_string(),
            "faces".to_string(),
        ],
    };
    let target = album
        .clone()
        .unwrap_or_else(|| "all owned albums".to_string());
    let task = Arc::new(AiIndexTask::new(
        trf(locale, "task.ai_index", &[("target", target)]),
        owner,
        cfg.clone(),
        locale.to_string(),
        request,
    ));
    let (task_id, _) = cfg.task_mgr.add_task_if_absent(task).await;
    Some(task_id)
}

