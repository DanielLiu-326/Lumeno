use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use sha2::{Digest, Sha256};

use super::super::album::helpers::{load_existing_album_index, read_exif_cache_if_present};
use super::super::album::media::preview_thumbnail_cache_path;
use super::super::album::types::AlbumPhotoItem;
use super::super::album::{
    AiAlbumScanTarget, collect_accessible_ai_scan_targets, collect_album_image_paths,
};
use super::super::common::AppState;
use super::super::constants::{
    AI_EMBED_COST as EMBED_COST, AI_FACE_COST as FACE_COST, AI_SCAN_COST as SCAN_COST,
    AI_WRITE_COST as WRITE_COST,
};
use super::super::i18n::{tr, trf};
use super::super::tasks::{
    Task, TaskBase, TaskStatus, format_subtask, task_file_name, wait_while_paused,
};
use super::album_cache::{AlbumAiCache, embedding_cache_key, face_photo_cache_key};
use super::duplicate::{compute_duplicate_hash_pair, lookup_photo_phash_async};
use super::faces::FACE_DETECTOR_REVISION;
use super::models::{
    AiIndexedAlbumSummary, AiIndexedPhotoRecord, LoadedOwnerIndex, NormalizedAiIndexRequest,
};
use super::pipelines::{
    run_duplicate_pipeline, run_embedding_pipeline, run_face_pipeline, run_finalize_pipeline,
};
use serde_json::Value;

fn ai_subtask(phase: &str, path: &Path, current: usize, total: usize) -> String {
    format_subtask(phase, &task_file_name(path), current, total)
}

fn embedding_key_for(cache_id: &str, album_relative: &str) -> String {
    format!("{cache_id}/{album_relative}")
}

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
    reused_hash: Option<(String, Option<u64>, bool, Option<String>)>,
    skipped_hash_error: Option<String>,
    image_embedding_state: String,
    embedding: Option<Vec<f32>>,
    face_embedding_state: String,
    reused_from_generation: Option<String>,
}

pub(crate) fn build_photo_content_signature(
    size_bytes: u64,
    modified_unix_seconds: Option<u64>,
) -> String {
    match modified_unix_seconds {
        Some(modified) => format!("{size_bytes}:{modified}"),
        None => format!("{size_bytes}:unknown"),
    }
}

pub(crate) fn build_photo_stable_identity(
    owner: &str,
    source_key: &str,
    path: &str,
) -> (String, String) {
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
    cancel: Arc<AtomicBool>,
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
            cancel: Arc::new(AtomicBool::new(false)),
            paused: AtomicBool::new(false),
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    async fn wait_if_paused(&self) -> bool {
        wait_while_paused(self.cancel.as_ref(), &self.paused).await
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
        let mut estimated_total = photos_listed as u64 * (SCAN_COST + WRITE_COST);
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
            let hash_results = self
                .hash_prepared_photos(&target.album_name, &prepared)
                .await?;
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
            let photo_path_for_meta = photo_path.clone();
            let Some(metadata) =
                tokio::task::spawn_blocking(move || std::fs::metadata(photo_path_for_meta))
                    .await
                    .ok()
                    .and_then(|r| r.ok())
            else {
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
            let previous_record = previous_index
                .as_ref()
                .and_then(|loaded| loaded.photos.get(&stable_key));
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
            let (hash_preview, reused_hash, skipped_hash_error) = {
                let cached_phash = if duplicates_requested && !target.cache_id.is_empty() {
                    lookup_photo_phash_async(
                        self.cfg.ai_service.store().root(),
                        target.owner.as_str(),
                        target.cache_id.as_str(),
                        &stable_key,
                        &content_signature,
                    )
                    .await
                } else {
                    None
                };
                if let Some(phash) = cached_phash {
                    (
                        None,
                        Some(("ready".to_string(), Some(phash), true, None)),
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
                }
            };
            let reused_embedding = if image_requested && !target.cache_id.is_empty() {
                let revision = provider_revision.as_deref().unwrap_or("unknown");
                let key = embedding_cache_key(&stable_key, &content_signature, revision);
                let cache = AlbumAiCache::new(
                    self.cfg.ai_service.store().root(),
                    target.owner.as_str(),
                    target.cache_id.as_str(),
                );
                cache
                    .read_embedding_async(&key)
                    .await
                    .and_then(|file| file.decode_vector().ok())
            } else {
                None
            };
            let (image_embedding_state, embedding) = if let Some(vector) = reused_embedding {
                ("ready".to_string(), Some(vector))
            } else if image_requested {
                // Prefer preview thumbs (THUMBNAIL_SIZE) for SigLIP (256 input); fall back to original.
                let preview = preview_thumbnail_cache_path(
                    self.cfg.data_path.as_path(),
                    target.owner.as_str(),
                    &photo_path,
                    &metadata,
                );
                let decode_path = if tokio::fs::try_exists(&preview).await.unwrap_or(false) {
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
                ("skipped".to_string(), None)
            };
            // Face reuse must come from faces/*.json (or prior ready state), never from
            // embedding-only file-cache stubs that defaulted to "skipped".
            let face_embedding_state = if !self
                .request
                .features
                .iter()
                .any(|feature| feature == "faces")
            {
                "skipped".to_string()
            } else if !target.cache_id.is_empty() {
                let face_key = face_photo_cache_key(
                    &stable_key,
                    &content_signature,
                    provider_revision
                        .as_deref()
                        .unwrap_or(FACE_DETECTOR_REVISION),
                );
                let cache = AlbumAiCache::new(
                    self.cfg.ai_service.store().root(),
                    target.owner.as_str(),
                    target.cache_id.as_str(),
                );
                if cache.has_face_photo_async(&face_key).await {
                    "ready".to_string()
                } else {
                    "pending".to_string()
                }
            } else {
                "pending".to_string()
            };
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
    ) -> Result<Vec<Option<(String, Option<u64>, bool, Option<String>)>>, ()> {
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
        Ok(indexed.into_iter().map(|(_, computed)| computed).collect())
    }

    fn commit_prepared_photos(
        target: &AiAlbumScanTarget,
        source_key: &str,
        provider_revision: &Option<String>,
        indexed_at_unix_ms: u128,
        prepared: Vec<PreparedScanPhoto>,
        hash_results: Vec<Option<(String, Option<u64>, bool, Option<String>)>>,
        indexed_photos: &mut Vec<AiIndexedPhotoRecord>,
        indexed_embeddings: &mut HashMap<String, Vec<f32>>,
        photo_disk_paths: &mut HashMap<String, PathBuf>,
    ) {
        for (photo, computed_hash) in prepared.into_iter().zip(hash_results.into_iter()) {
            let (duplicate_hash_state, duplicate_phash_u64, duplicate_phash_valid, record_error) =
                if let Some(reused) = photo.reused_hash {
                    reused
                } else if let Some(computed) = computed_hash {
                    computed
                } else {
                    ("skipped".to_string(), None, false, photo.skipped_hash_error)
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
                duplicate_hash_state,
                duplicate_phash_u64,
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

    fn recalibrate_progress(
        &self,
        photos_total: usize,
        pending_image_embeds_len: usize,
        face_work: usize,
    ) {
        let scan_done = photos_total as u64 * SCAN_COST;
        let remaining = pending_image_embeds_len as u64 * EMBED_COST
            + face_work as u64 * FACE_COST
            + photos_total as u64 * WRITE_COST;
        // Count stays photo-based for the sidebar; cost drives the progress bar.
        self.base.set_total_count(photos_total.max(1));
        self.base.set_processed_count(photos_total);
        self.base.set_total_cost(scan_done + remaining);
        self.base.set_processed_cost(scan_done);
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
            .load_current_index_async(self.base.owner.as_str())
            .await;

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

        let decode_parallelism = self.cfg.settings_manager.ai_cpu_threads().await.max(1);

        if duplicates_requested
            || indexed_photos
                .iter()
                .any(|photo| photo.duplicate_phash_valid)
        {
            if run_duplicate_pipeline(
                self.cfg.ai_service.store().root(),
                self.base.owner.as_str(),
                &indexed_albums,
                &indexed_photos,
                &self.base,
                &self.cancel,
                &self.paused,
            )
            .await
            .is_err()
            {
                if !self.abort_if_cancelled().await {
                    self.fail("failed to publish duplicates index".to_string())
                        .await;
                }
                return;
            }
        }

        if image_requested {
            if run_embedding_pipeline(
                self.cfg.ai_service.store().root(),
                Arc::clone(&provider),
                &pending_image_embeds,
                &mut indexed_photos,
                &mut indexed_embeddings,
                &self.base,
                Arc::clone(&self.cancel),
                decode_parallelism,
            )
            .await
            .is_err()
            {
                let _ = self.abort_if_cancelled().await;
                return;
            }
            // Per-photo embeddings/*.json are already durable; no mid-run generation rewrite.
        }

        if self.abort_if_cancelled().await {
            return;
        }

        let face_out = match run_face_pipeline(
            self.cfg.as_ref(),
            Arc::clone(&provider),
            faces_requested,
            &mut indexed_photos,
            &mut previous_index,
            &photo_disk_paths,
            &self.base,
            Arc::clone(&self.cancel),
            &self.paused,
            decode_parallelism,
        )
        .await
        {
            Ok(value) => value,
            Err(()) => {
                let _ = self.abort_if_cancelled().await;
                return;
            }
        };

        if self.should_stop().await {
            return;
        }

        match run_finalize_pipeline(
            self.cfg.as_ref(),
            self.base.owner.as_str(),
            &self.request,
            &provider_health,
            &indexed_albums,
            &indexed_photos,
            &face_out.face_records,
            &self.base,
            &self.cancel,
        )
        .await
        {
            Ok(()) => {}
            Err(error) if error == "cancelled" => {
                let _ = self.abort_if_cancelled().await;
            }
            Err(error) => {
                self.fail(error).await;
            }
        }
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
    let has_index = cfg
        .ai_service
        .store()
        .owner_has_current_index(owner.as_str());
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
