use async_trait::async_trait;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use sha2::{Digest, Sha256};

use super::super::album::{
    collect_accessible_ai_scan_targets, collect_album_image_paths, AiAlbumScanTarget,
};
use super::super::album::helpers::{load_existing_album_index, read_exif_cache_if_present};
use super::super::album::media::preview_thumbnail_cache_path;
use super::super::album::types::AlbumPhotoItem;
use super::super::common::AppState;
use super::duplicate::compute_duplicate_hash_pair;
use super::faces::FACE_DETECTOR_REVISION;
use super::super::i18n::{tr, trf};
use super::models::{
    AiFaceRecord, AiIndexedAlbumSummary, AiIndexedPhotoRecord, LoadedOwnerIndex,
    NormalizedAiIndexRequest,
};
use super::people::{
    ClusterFace, DEFAULT_CLUSTER_EPS, DEFAULT_CLUSTER_MIN_SAMPLES, FaceAssignment, PersonRecord,
    rebuild_people_assignments,
};
use super::provider::{progress_channel, AiInferenceProvider, AiItemError, AiProviderHealth};
use super::super::tasks::{
    Task, TaskBase, TaskStatus, format_subtask, task_file_name, wait_while_paused,
};
use serde_json::Value;

pub(crate) fn pending_or_skipped(features: &[String], feature: &str) -> String {
    if features.iter().any(|value| value == feature) {
        "pending".to_string()
    } else {
        "skipped".to_string()
    }
}

fn ai_subtask(phase: &str, path: &Path, current: usize, total: usize) -> String {
    format_subtask(phase, &task_file_name(path), current, total)
}


fn embedding_key_for(cache_id: &str, album_relative: &str) -> String {
    format!("{cache_id}/{album_relative}")
}

fn apply_embed_result(
    indexed_photos: &mut [AiIndexedPhotoRecord],
    indexed_embeddings: &mut HashMap<String, Vec<f32>>,
    embedding_key: &str,
    result: Result<Vec<f32>, AiItemError>,
) {
    let Some(photo) = indexed_photos
        .iter_mut()
        .find(|photo| photo.embedding_key() == embedding_key)
    else {
        return;
    };
    match result {
        Ok(vector) => {
            photo.image_embedding_state = "ready".to_string();
            indexed_embeddings.insert(embedding_key.to_string(), vector);
        }
        Err(error) => {
            photo.image_embedding_state = "error".to_string();
            photo.record_error = Some(error.to_string());
        }
    }
}

/// Relative work weights: embedding/faces dominate wall time vs scan/hash.
const SCAN_COST: u64 = 1;
const EMBED_COST: u64 = 4;
const FACE_COST: u64 = 5;
const WRITE_COST: u64 = 1;

struct ScanAlbumsResult {
    indexed_albums: Vec<AiIndexedAlbumSummary>,
    indexed_photos: Vec<AiIndexedPhotoRecord>,
    indexed_embeddings: HashMap<String, Vec<f32>>,
    pending_image_embeds: Vec<(String, PathBuf, PathBuf)>,
    photo_disk_paths: HashMap<String, PathBuf>,
    photos_total: usize,
}

/// Prepared during album scan before parallel duplicate hashing.
struct PreparedScanPhoto {
    photo_path: PathBuf,
    display_path: String,
    relative_path: String,
    stable_key: String,
    stable_id: String,
    size_bytes: u64,
    modified_unix_seconds: Option<u64>,
    content_signature: String,
    width: Option<u32>,
    height: Option<u32>,
    scan_state: String,
    hash_preview: Option<PathBuf>,
    reused_hash: Option<(String, Option<String>, Option<u64>, bool, Option<String>)>,
    skipped_hash_error: Option<String>,
    image_embedding_state: String,
    embedding: Option<Vec<f32>>,
    face_embedding_state: String,
    reused_from_generation: Option<String>,
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
    progress: Option<&TaskBase>,
    face_cost: u64,
    cancel: Option<&AtomicBool>,
    paused: Option<&AtomicBool>,
) -> Result<(Vec<AiFaceRecord>, Vec<Option<Vec<f32>>>), ()> {
    let mut face_records = Vec::new();
    let mut face_embeddings = Vec::new();
    if !faces_requested {
        return Ok((face_records, face_embeddings));
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
            if let Some(base) = progress {
                base.add_processed_cost(face_cost);
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
            if let Some(base) = progress {
                base.add_processed_cost(face_cost);
            }
            continue;
        };

        detect_jobs.push((photo.stable_key.clone(), photo_path));
    }

    let detect_paths: Vec<PathBuf> = detect_jobs.iter().map(|(_, path)| path.clone()).collect();
    let detect_total = detect_paths.len().max(1);
    if detect_paths.is_empty() {
        return Ok((face_records, face_embeddings));
    }

    if let (Some(cancel), Some(paused)) = (cancel, paused)
        && wait_while_paused(cancel, paused).await
    {
        return Err(());
    }
    if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
        return Err(());
    }

    let progress_cap = provider
        .capabilities()
        .max_batch_size
        .unwrap_or(8)
        .saturating_mul(4)
        .max(8);
    let (progress_tx, mut progress_rx) = progress_channel(progress_cap);
    let progress_base = progress;
    let progress_task = async {
        let mut done = 0usize;
        while let Some((_, path)) = progress_rx.recv().await {
            done += 1;
            if let Some(base) = progress_base {
                base
                    .set_cur_task(ai_subtask(
                        "detecting faces",
                        path.as_path(),
                        done,
                        detect_total,
                    ))
                    .await;
                base.add_processed_cost(face_cost);
            }
        }
    };

    let (detected_result, _) = tokio::join!(
        provider.detect_and_embed_faces_progress(
            detect_paths.as_slice(),
            Some(progress_tx),
            cancel,
        ),
        progress_task,
    );
    let detected_results = match detected_result {
        Ok(results) => results,
        Err(error) => detect_jobs
            .iter()
            .map(|_| {
                Err(AiItemError {
                    code: "provider_rejected".to_string(),
                    message: error.clone(),
                })
            })
            .collect(),
    };

    let mut photo_by_stable_key = indexed_photos
        .iter_mut()
        .map(|photo| (photo.stable_key.clone(), photo))
        .collect::<HashMap<_, _>>();

    for ((stable_key, _), result) in detect_jobs.into_iter().zip(detected_results) {
        let Some(photo) = photo_by_stable_key.get_mut(&stable_key) else {
            continue;
        };
        match result {
            Ok(detected_faces) => {
                photo.face_embedding_state = "ready".to_string();
                let photo_id = photo.photo_id;
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

    Ok((face_records, face_embeddings))
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

fn dimensions_from_album_index(
    entry: Option<&AlbumPhotoItem>,
    size_bytes: u64,
    modified_unix_seconds: Option<u64>,
) -> Option<(u32, u32)> {
    let entry = entry?;
    if entry.size != size_bytes || entry.modified != modified_unix_seconds {
        return None;
    }
    if entry.width == 0 || entry.height == 0 {
        return None;
    }
    Some((entry.width, entry.height))
}

fn resolve_scan_dimensions(
    data_path: &Path,
    photo_path: &Path,
    metadata: &std::fs::Metadata,
    size_bytes: u64,
    modified_unix_seconds: Option<u64>,
    index_entry: Option<&AlbumPhotoItem>,
    reusable: Option<&super::models::AiIndexedPhotoRecord>,
) -> (Option<u32>, Option<u32>) {
    let from_index = dimensions_from_album_index(index_entry, size_bytes, modified_unix_seconds);
    let from_exif = read_exif_cache_if_present(data_path, photo_path, metadata).and_then(|exif| {
        if exif.width > 0 && exif.height > 0 {
            Some((exif.width, exif.height))
        } else {
            None
        }
    });
    let fallback = || {
        from_index
            .or(from_exif)
            .or_else(|| read_photo_dimensions(photo_path))
    };
    match reusable {
        Some(record) => (
            record.width.or_else(|| fallback().map(|value| value.0)),
            record.height.or_else(|| fallback().map(|value| value.1)),
        ),
        None => match fallback() {
            Some((width, height)) => (Some(width), Some(height)),
            None => (None, None),
        },
    }
}


pub struct AiIndexTask {
    base: TaskBase,
    cfg: Arc<AppState>,
    locale: String,
    request: NormalizedAiIndexRequest,
    cancel: AtomicBool,
    paused: AtomicBool,
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
            paused: AtomicBool::new(false),
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    async fn wait_if_paused(&self) -> bool {
        wait_while_paused(&self.cancel, &self.paused).await
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

    /// Wait out pause, then abort if cancelled. Always sets Cancelled status on abort
    /// (fixes cancel-while-paused / short-circuit leaving status stuck).
    async fn should_stop(&self) -> bool {
        let _ = self.wait_if_paused().await;
        self.abort_if_cancelled().await
    }

    async fn fail(&self, error: impl Into<String>) {
        self.base.set_error_msg(error.into()).await;
        self.base.set_status(TaskStatus::Failed).await;
    }

    async fn collect_scan_targets(&self) -> Result<Vec<AiAlbumScanTarget>, ()> {
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
                self.fail(error.to_string()).await;
                return Err(());
            }
        };
        let targets = targets
            .into_iter()
            .filter(|target| target.owner == self.base.owner)
            .collect::<Vec<_>>();
        if self.abort_if_cancelled().await {
            return Err(());
        }
        if self.request.album.is_some() && targets.is_empty() {
            self.fail(tr(self.locale.as_str(), "error.ai_index_owner_only"))
                .await;
            return Err(());
        }
        Ok(targets)
    }

    async fn list_album_photo_paths(
        &self,
        targets: Vec<AiAlbumScanTarget>,
    ) -> Result<(Vec<(AiAlbumScanTarget, Vec<PathBuf>)>, usize), ()> {
        let mut album_photo_paths = Vec::with_capacity(targets.len());
        for target in targets {
            if self.abort_if_cancelled().await {
                return Err(());
            }
            self.base
                .set_cur_task(format!("listing · {}", target.album_name))
                .await;
            let photo_paths = collect_album_image_paths(target.album_dir.as_path()).await;
            album_photo_paths.push((target, photo_paths));
        }
        let photos_listed = album_photo_paths
            .iter()
            .map(|(_, paths)| paths.len())
            .sum::<usize>();
        Ok((album_photo_paths, photos_listed))
    }

    fn init_estimated_progress(
        &self,
        photos_listed: usize,
        image_requested: bool,
        faces_requested: bool,
    ) {
        let mut estimated_total = photos_listed as u64 * SCAN_COST + WRITE_COST;
        if image_requested {
            estimated_total += photos_listed as u64 * EMBED_COST;
        }
        if faces_requested {
            estimated_total += photos_listed as u64 * FACE_COST;
        }
        self.base.set_total_count(photos_listed.max(1));
        self.base.set_processed_count(0);
        self.base.set_total_cost(estimated_total.max(1));
        self.base.set_processed_cost(0);
    }

    async fn scan_albums(
        &self,
        album_photo_paths: Vec<(AiAlbumScanTarget, Vec<PathBuf>)>,
        previous_index: &Option<LoadedOwnerIndex>,
        provider_revision: &Option<String>,
        duplicates_requested: bool,
        image_requested: bool,
    ) -> Result<ScanAlbumsResult, ()> {
        let mut indexed_albums = Vec::with_capacity(album_photo_paths.len());
        let mut indexed_photos = Vec::new();
        let mut indexed_embeddings = HashMap::<String, Vec<f32>>::new();
        let mut pending_image_embeds = Vec::<(String, PathBuf, PathBuf)>::new();
        let mut photo_disk_paths = HashMap::<String, PathBuf>::new();
        let mut photos_total = 0usize;
        let indexed_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();

        for (target, photo_paths) in album_photo_paths {
            if self.abort_if_cancelled().await {
                return Err(());
            }
            self.base
                .set_cur_task(format!("scanning · {}", target.album_name))
                .await;
            let photo_count = photo_paths.len();
            photos_total += photo_count;
            let source_key = target.album_name.clone();
            let owner_root = self.cfg.user_root_path(target.owner.as_str());
            let album_index_by_path = load_existing_album_index(
                self.cfg.data_path.as_path(),
                owner_root.as_path(),
                target.owner.as_str(),
                target.album_name.as_str(),
            )
            .await
            .ok()
            .flatten()
            .map(|index| {
                index
                    .entries
                    .into_iter()
                    .map(|entry| (entry.path.clone(), entry))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();

            let prepared = self
                .prepare_album_photos(
                    &target,
                    photo_paths,
                    &source_key,
                    &album_index_by_path,
                    previous_index,
                    provider_revision,
                    duplicates_requested,
                    image_requested,
                    &mut pending_image_embeds,
                )
                .await?;
            let hash_results = self.hash_prepared_photos(&target.album_name, &prepared).await?;
            Self::commit_prepared_photos(
                &target,
                &source_key,
                provider_revision,
                indexed_at_unix_ms,
                prepared,
                hash_results,
                &mut indexed_photos,
                &mut indexed_embeddings,
                &mut photo_disk_paths,
            );
            indexed_albums.push(AiIndexedAlbumSummary {
                source_key,
                owner: target.owner.clone(),
                album: target.album_name.clone(),
                cache_id: target.cache_id.clone(),
                photo_count,
            });
        }

        Ok(ScanAlbumsResult {
            indexed_albums,
            indexed_photos,
            indexed_embeddings,
            pending_image_embeds,
            photo_disk_paths,
            photos_total,
        })
    }

    async fn prepare_album_photos(
        &self,
        target: &AiAlbumScanTarget,
        photo_paths: Vec<PathBuf>,
        source_key: &str,
        album_index_by_path: &HashMap<String, AlbumPhotoItem>,
        previous_index: &Option<LoadedOwnerIndex>,
        provider_revision: &Option<String>,
        duplicates_requested: bool,
        image_requested: bool,
        pending_image_embeds: &mut Vec<(String, PathBuf, PathBuf)>,
    ) -> Result<Vec<PreparedScanPhoto>, ()> {
        let mut prepared = Vec::with_capacity(photo_paths.len());
        let scan_phase = format!("scanning · {}", target.album_name);
        let scan_total = photo_paths.len().max(1);
        for (scan_index, photo_path) in photo_paths.into_iter().enumerate() {
            if self.should_stop().await {
                return Err(());
            }
            self.base
                .set_cur_task(ai_subtask(
                    &scan_phase,
                    photo_path.as_path(),
                    scan_index + 1,
                    scan_total,
                ))
                .await;
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
                source_key,
                display_path.as_str(),
            );
            let modified_unix_seconds = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                .map(|value| value.as_secs());
            let content_signature =
                build_photo_content_signature(metadata.len(), modified_unix_seconds);
            let index_entry = album_index_by_path.get(&display_path);
            let previous_record = previous_index.as_ref().and_then(|loaded| {
                loaded
                    .photos
                    .get(&stable_key)
                    .or_else(|| loaded.photos.get(&legacy_stable_key))
            });
            let reusable_record = previous_record.filter(|record| {
                !record.content_signature.is_empty()
                    && record.content_signature == content_signature
                    && record.provider_revision == *provider_revision
            });
            let (width, height) = resolve_scan_dimensions(
                self.cfg.data_path.as_path(),
                &photo_path,
                &metadata,
                metadata.len(),
                modified_unix_seconds,
                index_entry,
                reusable_record,
            );
            let scan_state = if reusable_record.is_some() {
                "reused".to_string()
            } else {
                "discovered".to_string()
            };
            let (hash_preview, reused_hash, skipped_hash_error) = if let Some(record) =
                reusable_record
                && record.duplicate_hash_state == "ready"
                && record.duplicate_phash_valid
                && record.duplicate_phash_u64.is_some()
            {
                (
                    None,
                    Some((
                        "ready".to_string(),
                        None, // exact SHA256 disabled
                        record.duplicate_phash_u64,
                        record.duplicate_phash_valid,
                        record.record_error.clone(),
                    )),
                    None,
                )
            } else if duplicates_requested {
                let preview = preview_thumbnail_cache_path(
                    self.cfg.data_path.as_path(),
                    target.owner.as_str(),
                    &photo_path,
                    &metadata,
                );
                (Some(preview), None, None)
            } else {
                (
                    None,
                    None,
                    reusable_record.and_then(|record| record.record_error.clone()),
                )
            };
            let reused_embedding = reusable_record.and_then(|record| {
                let embed_key = embedding_key_for(target.cache_id.as_str(), &relative_path);
                previous_index
                    .as_ref()
                    .and_then(|loaded| {
                        loaded
                            .embeddings
                            .get(&embed_key)
                            .or_else(|| loaded.embeddings.get(&record.stable_key))
                            .or_else(|| loaded.embeddings.get(&stable_key))
                            .or_else(|| loaded.embeddings.get(&legacy_stable_key))
                    })
                    .cloned()
                    .filter(|_| record.image_embedding_state == "ready")
            });
            let (image_embedding_state, embedding) = if let Some(vector) = reused_embedding {
                ("ready".to_string(), Some(vector))
            } else if image_requested {
                // Prefer 360px preview for SigLIP (256 input); fall back to original.
                let preview = preview_thumbnail_cache_path(
                    self.cfg.data_path.as_path(),
                    target.owner.as_str(),
                    &photo_path,
                    &metadata,
                );
                let decode_path = if preview.is_file() {
                    preview
                } else {
                    photo_path.clone()
                };
                pending_image_embeds.push((
                    embedding_key_for(target.cache_id.as_str(), &relative_path),
                    photo_path.clone(),
                    decode_path,
                ));
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
            prepared.push(PreparedScanPhoto {
                photo_path,
                display_path,
                relative_path,
                stable_key,
                stable_id,
                size_bytes: metadata.len(),
                modified_unix_seconds,
                content_signature,
                width,
                height,
                scan_state,
                hash_preview,
                reused_hash,
                skipped_hash_error,
                image_embedding_state,
                embedding,
                face_embedding_state,
                reused_from_generation,
            });
        }
        Ok(prepared)
    }

    async fn hash_prepared_photos(
        &self,
        album_name: &str,
        prepared: &[PreparedScanPhoto],
    ) -> Result<
        Vec<Option<(String, Option<String>, Option<u64>, bool, Option<String>)>>,
        (),
    > {
        let hash_phase = format!("hashing · {album_name}");
        let hash_concurrency = std::thread::available_parallelism()
            .map(|value| value.get())
            .unwrap_or(1)
            .clamp(1, 16);
        let hash_total = prepared.len().max(1);
        let hash_progress = AtomicUsize::new(0);
        let hash_jobs = prepared
            .iter()
            .enumerate()
            .map(|(index, photo)| {
                (
                    index,
                    photo.photo_path.clone(),
                    photo.hash_preview.clone(),
                    photo.reused_hash.is_some(),
                )
            })
            .collect::<Vec<_>>();
        use futures::StreamExt;
        let mut indexed = futures::stream::iter(hash_jobs)
            .map(|(index, photo_path, preview, reused)| {
                let hash_phase = hash_phase.clone();
                let hash_progress = &hash_progress;
                async move {
                    if self.should_stop().await {
                        return (index, None);
                    }
                    let current = hash_progress.fetch_add(1, Ordering::Relaxed) + 1;
                    self.base
                        .set_cur_task(ai_subtask(
                            hash_phase.as_str(),
                            photo_path.as_path(),
                            current,
                            hash_total,
                        ))
                        .await;
                    let computed = if reused {
                        None
                    } else if let Some(preview) = preview {
                        tokio::task::spawn_blocking(move || {
                            compute_duplicate_hash_pair(
                                photo_path.as_path(),
                                Some(preview.as_path()),
                            )
                        })
                        .await
                        .ok()
                    } else {
                        None
                    };
                    self.base.add_processed_count(1);
                    self.base.add_processed_cost(SCAN_COST);
                    (index, computed)
                }
            })
            .buffer_unordered(hash_concurrency)
            .collect::<Vec<_>>()
            .await;
        indexed.sort_by_key(|(index, _)| *index);
        Ok(indexed
            .into_iter()
            .map(|(_, computed)| computed)
            .collect())
    }

    fn commit_prepared_photos(
        target: &AiAlbumScanTarget,
        source_key: &str,
        provider_revision: &Option<String>,
        indexed_at_unix_ms: u128,
        prepared: Vec<PreparedScanPhoto>,
        hash_results: Vec<Option<(String, Option<String>, Option<u64>, bool, Option<String>)>>,
        indexed_photos: &mut Vec<AiIndexedPhotoRecord>,
        indexed_embeddings: &mut HashMap<String, Vec<f32>>,
        photo_disk_paths: &mut HashMap<String, PathBuf>,
    ) {
        for (photo, computed_hash) in prepared.into_iter().zip(hash_results.into_iter()) {
            let (
                duplicate_hash_state,
                duplicate_sha256_hex,
                duplicate_phash_u64,
                duplicate_phash_valid,
                record_error,
            ) = if let Some(reused) = photo.reused_hash {
                reused
            } else if let Some(computed) = computed_hash {
                computed
            } else {
                (
                    "skipped".to_string(),
                    None,
                    None,
                    false,
                    photo.skipped_hash_error,
                )
            };
            photo_disk_paths.insert(photo.stable_key.clone(), photo.photo_path);
            if let Some(vector) = photo.embedding {
                indexed_embeddings.insert(
                    embedding_key_for(target.cache_id.as_str(), &photo.relative_path),
                    vector,
                );
            }
            indexed_photos.push(AiIndexedPhotoRecord {
                photo_id: indexed_photos.len(),
                owner: target.owner.clone(),
                album: target.album_name.clone(),
                source_key: source_key.to_string(),
                cache_id: target.cache_id.clone(),
                path: photo.display_path,
                stable_key: photo.stable_key,
                stable_id: photo.stable_id,
                size_bytes: photo.size_bytes,
                modified_unix_seconds: photo.modified_unix_seconds,
                content_signature: photo.content_signature,
                width: photo.width,
                height: photo.height,
                provider_revision: provider_revision.clone(),
                scan_state: photo.scan_state,
                image_embedding_state: photo.image_embedding_state,
                image_embedding_offset: None,
                duplicate_hash_state,
                duplicate_sha256_hex,
                duplicate_sha256_offset: None,
                duplicate_phash_u64,
                duplicate_phash_offset: None,
                duplicate_phash_valid,
                face_embedding_state: photo.face_embedding_state,
                reused_from_generation: photo.reused_from_generation,
                record_error,
                indexed_at_unix_ms,
            });
        }
    }

    fn sort_and_reassign_photo_ids(
        indexed_albums: &mut Vec<AiIndexedAlbumSummary>,
        mut indexed_photos: Vec<AiIndexedPhotoRecord>,
    ) -> Vec<AiIndexedPhotoRecord> {
        indexed_albums.sort_by(|left, right| {
            left.cache_id
                .cmp(&right.cache_id)
                .then(left.source_key.cmp(&right.source_key))
        });
        indexed_photos.sort_by(|left, right| left.stable_key.cmp(&right.stable_key));
        for (photo_id, photo) in indexed_photos.iter_mut().enumerate() {
            photo.photo_id = photo_id;
        }
        indexed_photos
    }

    /// Persist image embeddings right after the embed phase (before faces).
    async fn checkpoint_image_embeddings(
        &self,
        provider_health: &AiProviderHealth,
        indexed_albums: &[AiIndexedAlbumSummary],
        indexed_photos: &[AiIndexedPhotoRecord],
        indexed_embeddings: &HashMap<String, Vec<f32>>,
    ) -> Result<(), ()> {
        let mut cache_ids = indexed_photos
            .iter()
            .map(|photo| photo.cache_id.clone())
            .filter(|cache_id| !cache_id.is_empty())
            .collect::<BTreeSet<_>>();
        if cache_ids.is_empty() {
            for album in indexed_albums {
                if !album.cache_id.is_empty() {
                    cache_ids.insert(album.cache_id.clone());
                }
            }
        }
        if cache_ids.is_empty() {
            return Ok(());
        }

        let write_total = cache_ids.len().max(1);
        for (write_index, cache_id) in cache_ids.into_iter().enumerate() {
            if self.should_stop().await {
                return Err(());
            }
            let album_label = indexed_albums
                .iter()
                .find(|album| album.cache_id == cache_id)
                .map(|album| album.album.as_str())
                .unwrap_or(cache_id.as_str());
            self.base
                .set_cur_task(format_subtask(
                    "checkpoint embeddings",
                    album_label,
                    write_index + 1,
                    write_total,
                ))
                .await;

            let mut album_photos = Vec::new();
            for photo in indexed_photos.iter().cloned() {
                if photo.cache_id != cache_id {
                    continue;
                }
                let mut photo = photo;
                photo.photo_id = album_photos.len();
                album_photos.push(photo);
            }

            let album_summaries = indexed_albums
                .iter()
                .filter(|album| album.cache_id == cache_id)
                .cloned()
                .collect::<Vec<_>>();
            let album_photo_count = album_photos.len();
            let mut empty_faces: Vec<AiFaceRecord> = Vec::new();
            let empty_face_embeddings: Vec<Option<Vec<f32>>> = Vec::new();
            if let Err(error) = self
                .cfg
                .ai_service
                .bootstrap_owner_index(
                    self.base.owner.as_str(),
                    cache_id.as_str(),
                    &self.request,
                    provider_health,
                    &album_summaries,
                    &mut album_photos,
                    indexed_embeddings,
                    &mut empty_faces,
                    &empty_face_embeddings,
                    &[],
                    &[],
                    album_photo_count,
                )
                .await
            {
                self.fail(error).await;
                return Err(());
            }
        }
        Ok(())
    }

    fn recalibrate_progress(
        &self,
        photos_total: usize,
        pending_image_embeds_len: usize,
        face_work: usize,
    ) {
        let scan_done = photos_total as u64 * SCAN_COST;
        let remaining = pending_image_embeds_len as u64 * EMBED_COST
            + face_work as u64 * FACE_COST
            + WRITE_COST;
        // Count stays photo-based for the sidebar; cost drives the progress bar.
        self.base.set_total_count(photos_total.max(1));
        self.base.set_processed_count(photos_total);
        self.base.set_total_cost(scan_done + remaining);
        self.base.set_processed_cost(scan_done);
    }

    async fn embed_pending_images(
        &self,
        provider: &dyn AiInferenceProvider,
        pending_image_embeds: &[(String, PathBuf, PathBuf)],
        indexed_photos: &mut [AiIndexedPhotoRecord],
        indexed_embeddings: &mut HashMap<String, Vec<f32>>,
    ) -> Result<(), ()> {
        if pending_image_embeds.is_empty() {
            return Ok(());
        }
        if self.should_stop().await {
            return Err(());
        }

        let embed_total = pending_image_embeds.len().max(1);
        let paths = pending_image_embeds
            .iter()
            .map(|(_, _, decode_path)| decode_path.clone())
            .collect::<Vec<_>>();
        let embedding_keys = pending_image_embeds
            .iter()
            .map(|(embedding_key, _, _)| embedding_key.clone())
            .collect::<Vec<_>>();

        let progress_cap = provider
        .capabilities()
        .max_batch_size
        .unwrap_or(8)
        .saturating_mul(4)
        .max(8);
    let (progress_tx, mut progress_rx) = progress_channel(progress_cap);
        let base = &self.base;
        let progress_task = async {
            let mut done = 0usize;
            while let Some((index, path)) = progress_rx.recv().await {
                done += 1;
                // Prefer original photo path for UI; decode_path may be a preview thumbnail.
                let display = pending_image_embeds
                    .get(index)
                    .map(|(_, photo_path, _)| photo_path.as_path())
                    .unwrap_or(path.as_path());
                base
                    .set_cur_task(ai_subtask(
                        "embedding images",
                        display,
                        done,
                        embed_total,
                    ))
                    .await;
                base.add_processed_cost(EMBED_COST);
            }
        };

        let (results, _) = tokio::join!(
            provider.embed_images_progress(
                paths.as_slice(),
                Some(progress_tx),
                Some(&self.cancel),
            ),
            progress_task,
        );

        match results {
            Ok(results) => {
                for (embedding_key, result) in embedding_keys.iter().zip(results) {
                    apply_embed_result(
                        indexed_photos,
                        indexed_embeddings,
                        embedding_key,
                        result,
                    );
                }
            }
            Err(error) => {
                for embedding_key in embedding_keys {
                    apply_embed_result(
                        indexed_photos,
                        indexed_embeddings,
                        embedding_key.as_str(),
                        Err(AiItemError {
                            code: "provider_rejected".to_string(),
                            message: error.clone(),
                        }),
                    );
                }
            }
        }
        Ok(())
    }

    async fn build_faces_and_people(
        &self,
        provider: &dyn AiInferenceProvider,
        faces_requested: bool,
        indexed_photos: &mut [AiIndexedPhotoRecord],
        previous_index: &mut Option<LoadedOwnerIndex>,
        photo_disk_paths: &HashMap<String, PathBuf>,
    ) -> Result<
        (
            Vec<AiFaceRecord>,
            Vec<Option<Vec<f32>>>,
            Vec<PersonRecord>,
            Vec<FaceAssignment>,
        ),
        (),
    > {
        if faces_requested {
            let (face_records, face_embeddings) = match build_faces_for_photos(
                self.cfg.as_ref(),
                provider,
                faces_requested,
                indexed_photos,
                previous_index,
                photo_disk_paths,
                Some(&self.base),
                FACE_COST,
                Some(&self.cancel),
                Some(&self.paused),
            )
            .await
            {
                Ok(value) => value,
                Err(()) => {
                    let _ = self.abort_if_cancelled().await;
                    return Err(());
                }
            };
            if self.should_stop().await {
                return Err(());
            }
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
            // Finished reusing prior faces/people — free the merged old index before DBSCAN.
            *previous_index = None;
            let (people, assignments) = rebuild_people_assignments(
                &cluster_faces,
                previous_people,
                previous_assignments,
                DEFAULT_CLUSTER_EPS,
                DEFAULT_CLUSTER_MIN_SAMPLES,
            );
            Ok((face_records, face_embeddings, people, assignments))
        } else if let Some(previous) = previous_index.as_ref() {
            let face_records = previous.faces.values().cloned().collect::<Vec<_>>();
            let face_embeddings = face_records
                .iter()
                .map(|face| previous.face_embeddings.get(&face.face_id).cloned())
                .collect::<Vec<_>>();
            let people = previous.people.values().cloned().collect::<Vec<_>>();
            let assignments = previous.assignments.values().cloned().collect::<Vec<_>>();
            Ok((face_records, face_embeddings, people, assignments))
        } else {
            Ok((Vec::new(), Vec::new(), Vec::new(), Vec::new()))
        }
    }

    async fn write_generations_and_finish(
        &self,
        provider_health: &AiProviderHealth,
        indexed_albums: &[AiIndexedAlbumSummary],
        indexed_photos: &[AiIndexedPhotoRecord],
        indexed_embeddings: &HashMap<String, Vec<f32>>,
        face_records: &[AiFaceRecord],
        face_embeddings: &[Option<Vec<f32>>],
        people: &[PersonRecord],
        assignments: &[FaceAssignment],
    ) {
        let mut cache_ids = indexed_photos
            .iter()
            .map(|photo| photo.cache_id.clone())
            .filter(|cache_id| !cache_id.is_empty())
            .collect::<BTreeSet<_>>();
        if cache_ids.is_empty() {
            for album in indexed_albums {
                if !album.cache_id.is_empty() {
                    cache_ids.insert(album.cache_id.clone());
                }
            }
        }

        let write_total = cache_ids.len().max(1);
        let mut write_error = None;
        let mut committed = Vec::new();
        for (write_index, cache_id) in cache_ids.into_iter().enumerate() {
            if self.should_stop().await {
                return;
            }
            let album_label = indexed_albums
                .iter()
                .find(|album| album.cache_id == cache_id)
                .map(|album| album.album.as_str())
                .unwrap_or(cache_id.as_str());
            self.base
                .set_cur_task(format_subtask(
                    "writing index",
                    album_label,
                    write_index + 1,
                    write_total,
                ))
                .await;
            let mut album_photos = Vec::new();
            let mut old_photo_id_to_new = HashMap::<usize, usize>::new();
            for photo in indexed_photos.iter().cloned() {
                if photo.cache_id != cache_id {
                    continue;
                }
                let new_id = album_photos.len();
                old_photo_id_to_new.insert(photo.photo_id, new_id);
                let mut photo = photo;
                photo.photo_id = new_id;
                album_photos.push(photo);
            }
            let mut album_faces = Vec::new();
            let mut album_face_embeddings = Vec::new();
            for (face, embedding) in face_records.iter().zip(face_embeddings.iter()) {
                let Some(&new_photo_id) = old_photo_id_to_new.get(&face.photo_id) else {
                    continue;
                };
                let mut face = face.clone();
                face.photo_id = new_photo_id;
                album_faces.push(face);
                album_face_embeddings.push(embedding.clone());
            }
            let album_summaries = indexed_albums
                .iter()
                .filter(|album| album.cache_id == cache_id)
                .cloned()
                .collect::<Vec<_>>();
            let album_photo_count = album_photos.len();
            match self
                .cfg
                .ai_service
                .bootstrap_owner_index(
                    self.base.owner.as_str(),
                    cache_id.as_str(),
                    &self.request,
                    provider_health,
                    &album_summaries,
                    &mut album_photos,
                    indexed_embeddings,
                    &mut album_faces,
                    &album_face_embeddings,
                    &[],
                    &[],
                    album_photo_count,
                )
                .await
            {
                Ok(generation_id) => committed.push((cache_id, generation_id, album_photo_count)),
                Err(error) => {
                    write_error = Some(error);
                    break;
                }
            }
        }

        if write_error.is_none() {
            if let Err(error) = self
                .cfg
                .ai_service
                .bootstrap_people_index(self.base.owner.as_str(), people, assignments)
                .await
            {
                write_error = Some(error);
            }
        }

        if self.abort_if_cancelled().await {
            return;
        }

        match write_error {
            None => {
                self.base.add_processed_cost(WRITE_COST);
                let summary = committed
                    .iter()
                    .map(|(cache_id, generation_id, count)| {
                        format!("{cache_id}:{generation_id}({count})")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                self.base
                    .set_cur_task(format!(
                        "committed {} album index(es): {summary}",
                        committed.len()
                    ))
                    .await;
                self.base.set_status(TaskStatus::Finished).await;
            }
            Some(error) => {
                self.fail(error).await;
            }
        }
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
        let mut previous_index = self
            .cfg
            .ai_service
            .store()
            .load_current_index(self.base.owner.as_str());

        let Ok(targets) = self.collect_scan_targets().await else {
            return;
        };
        let Ok((album_photo_paths, photos_listed)) = self.list_album_photo_paths(targets).await
        else {
            return;
        };
        self.init_estimated_progress(photos_listed, image_requested, faces_requested);

        let Ok(scan) = self
            .scan_albums(
                album_photo_paths,
                &previous_index,
                &provider_revision,
                duplicates_requested,
                image_requested,
            )
            .await
        else {
            return;
        };
        let ScanAlbumsResult {
            mut indexed_albums,
            indexed_photos,
            mut indexed_embeddings,
            pending_image_embeds,
            photo_disk_paths,
            photos_total,
        } = scan;

        let mut indexed_photos =
            Self::sort_and_reassign_photo_ids(&mut indexed_albums, indexed_photos);

        // Reuse of prior image vectors is finished; drop them so RSS does not keep
        // both old+new embeddings for the whole embed/face/write phases.
        if let Some(previous) = previous_index.as_mut() {
            previous.embeddings.clear();
            previous.embeddings.shrink_to_fit();
        }

        if self.abort_if_cancelled().await {
            return;
        }

        // Recalibrate remaining work: reused embeddings/faces shrink the total instead of
        // leaving the bar stuck near 100% while heavy phases still run.
        let face_work = if faces_requested {
            indexed_photos
                .iter()
                .filter(|photo| photo.face_embedding_state != "skipped")
                .count()
        } else {
            0
        };
        self.recalibrate_progress(photos_total, pending_image_embeds.len(), face_work);

        if image_requested {
            let Ok(()) = self
                .embed_pending_images(
                    provider.as_ref(),
                    &pending_image_embeds,
                    &mut indexed_photos,
                    &mut indexed_embeddings,
                )
                .await
            else {
                return;
            };
        }

        // Durable checkpoint: next run can reuse image embeddings if we die during faces.
        if image_requested {
            let Ok(()) = self
                .checkpoint_image_embeddings(
                    &provider_health,
                    &indexed_albums,
                    &indexed_photos,
                    &indexed_embeddings,
                )
                .await
            else {
                return;
            };
        }

        if self.abort_if_cancelled().await {
            return;
        }

        let Ok((face_records, face_embeddings, people, assignments)) = self
            .build_faces_and_people(
                provider.as_ref(),
                faces_requested,
                &mut indexed_photos,
                &mut previous_index,
                &photo_disk_paths,
            )
            .await
        else {
            return;
        };

        if self.should_stop().await {
            return;
        }

        self.write_generations_and_finish(
            &provider_health,
            &indexed_albums,
            &indexed_photos,
            &indexed_embeddings,
            &face_records,
            &face_embeddings,
            &people,
            &assignments,
        )
        .await;
    }

    async fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.paused.store(false, Ordering::Relaxed);
        self.base.set_cur_task("Cancelled".to_string()).await;
        self.base.set_status(TaskStatus::Cancelled).await;
    }

    async fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
        self.base.set_status(TaskStatus::Paused).await;
        self.base.set_cur_task("Paused".to_string()).await;
    }

    async fn resume(&self, _extra_info: Value) {
        self.paused.store(false, Ordering::Relaxed);
        self.base.set_status(TaskStatus::Running).await;
        self.base.set_cur_task("Resuming".to_string()).await;
    }

    fn cancellable(&self) -> bool {
        true
    }

    fn pausable(&self) -> bool {
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

