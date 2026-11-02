//! Face domain pipeline: face cache lookup → decode → batch detect → save each photo.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::backend::album::decoder::{DecoderCreateOptions, ImageDecoder, create_decoder};
use crate::backend::album::{ensure_cached_face_crop, write_cached_face_crop_from_image};
use crate::backend::common::AppState;
use crate::backend::constants::{
    AI_FACE_COST, DEFAULT_CLUSTER_MIN_SAMPLES, FACE_CROP_SIZE, cluster_eps_from_similarity,
};
use crate::backend::tasks::{TaskBase, format_subtask, task_file_name, wait_while_paused};

use super::super::album_cache::{
    AlbumAiCache, FaceCacheEntry, FaceIndexEntry, FacePhotoCacheFile,
    build_clusters_cache_with_matches_params, decode_f32_vector_b64, encode_f32_vector_b64,
    face_id, face_photo_cache_key, hints_from_clusters,
};
use super::super::faces::FACE_DETECTOR_REVISION;
use super::super::models::{AiFaceRecord, AiIndexedPhotoRecord, AiItemState, LoadedOwnerIndex};
use super::super::people::ClusterFace;
use super::super::pipeline::{ImageJob, ImagePipeline, PipelineConfig, progress_channel};
use super::super::provider::AiInferenceProvider;

struct FaceMeta {
    stable_key: String,
    photo_id: usize,
    content_signature: String,
    album_relative: String,
    owner: String,
    cache_id: String,
    album_dir: PathBuf,
}

struct CollectedFace {
    record: AiFaceRecord,
    embedding: Vec<f32>,
}

pub(crate) struct FacePipelineOutput {
    pub face_records: Vec<AiFaceRecord>,
}

fn resolve_photo_path(cfg: &AppState, record: &AiIndexedPhotoRecord) -> Option<PathBuf> {
    let prefix = format!("albums/{}/", record.album);
    let relative = record.path.strip_prefix(&prefix)?;
    let path = cfg
        .user_root_path(&record.owner)
        .join("albums")
        .join(&record.source_key)
        .join(relative);
    path.exists().then_some(path)
}

pub(crate) async fn run(
    cfg: &AppState,
    provider: Arc<dyn AiInferenceProvider>,
    faces_requested: bool,
    indexed_photos: &mut [AiIndexedPhotoRecord],
    previous_index: &mut Option<LoadedOwnerIndex>,
    photo_paths: &HashMap<String, PathBuf>,
    progress: &TaskBase,
    cancel: Arc<AtomicBool>,
    paused: &AtomicBool,
    decode_parallelism: usize,
) -> Result<FacePipelineOutput, ()> {
    if !faces_requested {
        if let Some(previous) = previous_index.as_ref() {
            let face_records = previous.faces.values().cloned().collect::<Vec<_>>();
            return Ok(FacePipelineOutput { face_records });
        }
        return Ok(FacePipelineOutput {
            face_records: Vec::new(),
        });
    }

    let ai_root = cfg.ai_service.store().root().to_path_buf();
    let face_revision = provider
        .model_revision()
        .unwrap_or_else(|| FACE_DETECTOR_REVISION.to_string());
    let collected = Arc::new(Mutex::new(Vec::<CollectedFace>::new()));
    let photo_states = Arc::new(Mutex::new(HashMap::<String, Result<(), String>>::new()));
    let mut detect_jobs = Vec::new();

    for photo in indexed_photos.iter_mut() {
        if wait_while_paused(cancel.as_ref(), paused).await {
            return Err(());
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(());
        }
        if photo.face_embedding_state == AiItemState::Skipped {
            continue;
        }
        if photo.cache_id.is_empty() {
            photo.face_embedding_state = AiItemState::Error;
            photo
                .record_error
                .get_or_insert_with(|| "cache_id missing for face detection".to_string());
            progress.add_processed_cost(AI_FACE_COST);
            continue;
        }

        let cache = AlbumAiCache::new(&ai_root, photo.owner.as_str(), photo.cache_id.as_str());
        let key = face_photo_cache_key(&photo.stable_key, &photo.content_signature, &face_revision);
        if cache.has_face_photo_async(&key).await {
            if let Some(cached) = cache.read_face_photo_async(&key).await {
                let album_dir = cfg
                    .user_root_path(photo.owner.as_str())
                    .join("albums")
                    .join(photo.source_key.as_str());
                let source = cached.source.clone();
                // Backfill missing crops under face_id = hash(stable_key|box).
                for entry in &cached.faces {
                    let fid = face_id(&photo.stable_key, &entry.box_xywh);
                    let Some(crop_path) = cache.face_crop_path_by_id(&fid) else {
                        continue;
                    };
                    if let Err(error) = ensure_cached_face_crop(
                        &album_dir,
                        &crop_path,
                        &source,
                        entry.box_xywh,
                        FACE_CROP_SIZE,
                        &cfg.thumbnail_limiter,
                        &cfg.darktable_limiter,
                    )
                    .await
                    {
                        eprintln!(
                            "face crop backfill failed for {}: {error}",
                            photo.stable_key
                        );
                    }
                }
                photo.face_embedding_state = AiItemState::Ready;
                let mut guard = collected.lock().unwrap_or_else(|e| e.into_inner());
                for entry in cached.faces {
                    let Ok(vector) =
                        decode_f32_vector_b64(&entry.embedding_b64, Some(entry.dimension))
                    else {
                        continue;
                    };
                    guard.push(CollectedFace {
                        record: AiFaceRecord {
                            face_id: face_id(&photo.stable_key, &entry.box_xywh),
                            photo_id: photo.photo_id,
                            stable_key: photo.stable_key.clone(),
                            box_xywh: entry.box_xywh,
                            landmarks: entry.landmarks,
                            detection_score: entry.detection_score,
                            quality_score: entry.quality_score,
                            detector_revision: face_revision.clone(),
                            embedding_state: AiItemState::Ready,
                        },
                        embedding: vector,
                    });
                }
                progress.add_processed_cost(AI_FACE_COST);
                continue;
            }
        }

        let Some(photo_path) = photo_paths
            .get(&photo.stable_key)
            .cloned()
            .or_else(|| resolve_photo_path(cfg, photo))
        else {
            photo.face_embedding_state = AiItemState::Error;
            photo
                .record_error
                .get_or_insert_with(|| "photo path missing for face detection".to_string());
            progress.add_processed_cost(AI_FACE_COST);
            continue;
        };

        let album_dir = cfg
            .user_root_path(photo.owner.as_str())
            .join("albums")
            .join(photo.source_key.as_str());

        detect_jobs.push(ImageJob {
            index: detect_jobs.len(),
            path: photo_path,
            meta: FaceMeta {
                stable_key: photo.stable_key.clone(),
                photo_id: photo.photo_id,
                content_signature: photo.content_signature.clone(),
                album_relative: photo.album_relative_path(),
                owner: photo.owner.clone(),
                cache_id: photo.cache_id.clone(),
                album_dir,
            },
        });
    }

    let detect_total = detect_jobs.len();
    if detect_total > 0 {
        let gpu_batch = provider.capabilities().max_batch_size.unwrap_or(8).max(1);
        let config = PipelineConfig::new("face", decode_parallelism.max(1), gpu_batch);
        let (progress_tx, mut progress_rx) = progress_channel(config.progress_queue_capacity());
        let progress_task = async {
            let mut done = 0usize;
            while let Some((_index, path)) = progress_rx.recv().await {
                done += 1;
                progress
                    .set_cur_task(format_subtask(
                        "detecting faces",
                        &task_file_name(path.as_path()),
                        done,
                        detect_total,
                    ))
                    .await;
                progress.add_processed_cost(AI_FACE_COST);
            }
        };

        let decoder: Arc<dyn ImageDecoder> =
            Arc::from(create_decoder(DecoderCreateOptions::auto()).expect("ai face decoder"));
        let collected_save = Arc::clone(&collected);
        let states_save = Arc::clone(&photo_states);
        let provider_infer = Arc::clone(&provider);
        let ai_root_save = ai_root.clone();
        let face_revision_save = face_revision.clone();
        let thumbnail_limiter = Arc::clone(&cfg.thumbnail_limiter);
        let darktable_limiter = Arc::clone(&cfg.darktable_limiter);

        let pipeline = ImagePipeline::new(
            config,
            decoder,
            Some(Arc::clone(&cancel)),
            Some(progress_tx),
        )
        .run(
            detect_jobs,
            move |images| {
                let provider = Arc::clone(&provider_infer);
                async move {
                    let results = provider.detect_and_embed_faces(&images).await?;
                    Ok((images, results))
                }
            },
            move |meta, result, image| {
                let ai_root = ai_root_save.clone();
                let collected_save = Arc::clone(&collected_save);
                let states_save = Arc::clone(&states_save);
                let face_revision_save = face_revision_save.clone();
                let thumbnail_limiter = Arc::clone(&thumbnail_limiter);
                let darktable_limiter = Arc::clone(&darktable_limiter);
                async move {
                    match result {
                        Ok(detected_faces) => {
                            let source = meta.album_relative.clone();
                            let mut entries = Vec::with_capacity(detected_faces.len());
                            let mut crop_boxes = Vec::with_capacity(detected_faces.len());
                            let mut new_faces = Vec::with_capacity(detected_faces.len());
                            for detected in detected_faces {
                                let face_id = face_id(&meta.stable_key, &detected.box_xywh);
                                entries.push(FaceCacheEntry {
                                    face_id: face_id.clone(),
                                    box_xywh: detected.box_xywh,
                                    landmarks: detected.landmarks,
                                    detection_score: detected.detection_score,
                                    quality_score: detected.quality_score,
                                    dimension: detected.embedding.len(),
                                    embedding_b64: encode_f32_vector_b64(&detected.embedding),
                                });
                                crop_boxes.push((face_id.clone(), detected.box_xywh));
                                new_faces.push(CollectedFace {
                                    record: AiFaceRecord {
                                        face_id,
                                        photo_id: meta.photo_id,
                                        stable_key: meta.stable_key.clone(),
                                        box_xywh: detected.box_xywh,
                                        landmarks: detected.landmarks,
                                        detection_score: detected.detection_score,
                                        quality_score: detected.quality_score,
                                        detector_revision: face_revision_save.clone(),
                                        embedding_state: AiItemState::Ready,
                                    },
                                    embedding: detected.embedding,
                                });
                            }

                            let cache = AlbumAiCache::new(
                                &ai_root,
                                meta.owner.as_str(),
                                meta.cache_id.as_str(),
                            );
                            let key = face_photo_cache_key(
                                &meta.stable_key,
                                &meta.content_signature,
                                &face_revision_save,
                            );
                            let mut file = FacePhotoCacheFile::new(
                                &source,
                                &meta.stable_key,
                                &meta.content_signature,
                                &face_revision_save,
                                entries,
                            );
                            if let Some(image) = image.as_ref() {
                                file.width = Some(image.width());
                                file.height = Some(image.height());
                            }
                            if let Err(error) = cache.write_face_photo_async(&key, &file).await {
                                eprintln!(
                                    "face cache write failed for {}: {error}",
                                    meta.stable_key
                                );
                            } else {
                                // Keep box and path paired: an invalid id drops its own
                                // box instead of shifting the rest.
                                let crop_targets = crop_boxes
                                    .iter()
                                    .filter_map(|(fid, box_xywh)| {
                                        cache
                                            .face_crop_path_by_id(fid)
                                            .map(|path| (*box_xywh, path))
                                    })
                                    .collect::<Vec<_>>();
                                if let Some(image) = image {
                                    let stable_key = meta.stable_key.clone();
                                    let write_result = tokio::task::spawn_blocking(move || {
                                        for (box_xywh, crop_path) in crop_targets {
                                            write_cached_face_crop_from_image(
                                                &image,
                                                &crop_path,
                                                FACE_CROP_SIZE,
                                                box_xywh,
                                            )?;
                                        }
                                        Ok::<(), String>(())
                                    })
                                    .await;
                                    if let Err(error) = write_result
                                        .unwrap_or_else(|e| Err(format!("face crop join: {e}")))
                                    {
                                        eprintln!(
                                            "face crop write failed for {stable_key}: {error}"
                                        );
                                    }
                                } else {
                                    for (box_xywh, crop_path) in crop_targets {
                                        if let Err(error) = ensure_cached_face_crop(
                                            &meta.album_dir,
                                            &crop_path,
                                            &source,
                                            box_xywh,
                                            FACE_CROP_SIZE,
                                            &thumbnail_limiter,
                                            &darktable_limiter,
                                        )
                                        .await
                                        {
                                            eprintln!(
                                                "face crop write failed for {}: {error}",
                                                meta.stable_key
                                            );
                                        }
                                    }
                                }
                            }

                            collected_save
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .extend(new_faces);
                            states_save
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(meta.stable_key, Ok(()));
                        }
                        Err(error) => {
                            states_save
                                .lock()
                                .unwrap_or_else(|e| e.into_inner())
                                .insert(meta.stable_key, Err(error.to_string()));
                        }
                    }
                }
            },
        );

        let (pipeline_result, _) = tokio::join!(pipeline, progress_task);
        if pipeline_result.is_err() {
            return Err(());
        }

        let states = photo_states
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        for photo in indexed_photos.iter_mut() {
            match states.get(&photo.stable_key) {
                Some(Ok(())) => photo.face_embedding_state = AiItemState::Ready,
                Some(Err(error)) => {
                    photo.face_embedding_state = AiItemState::Error;
                    photo.record_error = Some(error.clone());
                }
                None => {}
            }
        }
    }

    if cancel.load(Ordering::Relaxed) {
        return Err(());
    }

    let (face_records, face_embeddings): (Vec<_>, Vec<_>) = collected
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
        .map(|item| (item.record, Some(item.embedding)))
        .unzip();

    progress
        .set_cur_task("building face indexes".to_string())
        .await;
    *previous_index = None;

    write_face_indexes(
        cfg,
        &ai_root,
        &face_revision,
        indexed_photos,
        &face_records,
        &face_embeddings,
    )
    .await;

    Ok(FacePipelineOutput { face_records })
}

/// Collect every unassigned face in the album cache for a full DBSCAN rebuild.
async fn collect_unassigned_faces_for_clustering(
    cache: &AlbumAiCache,
    assigned: &super::super::faces_config::AssignedFaceFilter,
) -> (Vec<ClusterFace>, Vec<FaceIndexEntry>) {
    let mut cluster_faces = Vec::new();
    let mut face_metas = Vec::new();
    let Ok(listed) = cache.list_face_photos_async().await else {
        return (cluster_faces, face_metas);
    };
    for (photo_key, file) in listed {
        for entry in file.faces {
            if assigned.is_assigned(
                file.source.as_str(),
                &entry.box_xywh,
                Some(entry.face_id.as_str()),
            ) {
                continue;
            }
            let Ok(embedding) = entry.decode_embedding() else {
                continue;
            };
            cluster_faces.push(ClusterFace {
                face_id: entry.face_id.clone(),
                embedding,
            });
            face_metas.push(FaceIndexEntry {
                face_id: entry.face_id,
                stable_key: file.stable_key.clone(),
                source: file.source.clone(),
                box_xywh: entry.box_xywh,
                quality_score: entry.quality_score,
                photo_key: photo_key.clone(),
            });
        }
    }
    (cluster_faces, face_metas)
}

/// Rebuild unnamed suggestion groups for one album using its saved similarity.
pub(crate) async fn rebuild_album_face_clusters(
    cache: &AlbumAiCache,
    faces_config: &super::super::faces_config::FacesConfig,
) -> Result<(), String> {
    let assigned = super::super::faces_config::AssignedFaceFilter::from_config(faces_config);
    let named_catalog = super::super::faces_config::collect_named_person_centers(faces_config);
    let (cluster_faces, face_metas) =
        collect_unassigned_faces_for_clustering(cache, &assigned).await;
    let clusters = build_clusters_cache_with_matches_params(
        &cluster_faces,
        &face_metas,
        cluster_eps_from_similarity(faces_config.cluster_similarity),
        DEFAULT_CLUSTER_MIN_SAMPLES,
        &named_catalog,
    );
    let hints = hints_from_clusters(&clusters);
    cache.write_face_index_async(&clusters).await?;
    cache.write_face_hints_async(&hints).await
}

async fn write_face_indexes(
    cfg: &AppState,
    ai_root: &Path,
    face_revision: &str,
    indexed_photos: &[AiIndexedPhotoRecord],
    face_records: &[AiFaceRecord],
    face_embeddings: &[Option<Vec<f32>>],
) {
    let mut by_cache: HashMap<String, Vec<&AiIndexedPhotoRecord>> = HashMap::new();
    for photo in indexed_photos {
        if photo.cache_id.is_empty() || photo.face_embedding_state == AiItemState::Skipped {
            continue;
        }
        by_cache
            .entry(photo.cache_id.clone())
            .or_default()
            .push(photo);
    }

    let emb_by_face: HashMap<&str, &Vec<f32>> = face_records
        .iter()
        .zip(face_embeddings.iter())
        .filter_map(|(face, emb)| emb.as_ref().map(|v| (face.face_id.as_str(), v)))
        .collect();

    for (cache_id, photos) in by_cache {
        let owner = photos[0].owner.as_str();
        let album_name = photos[0].album.as_str();
        let album_dir = cfg.user_root_path(owner).join("albums").join(album_name);
        let mut faces_config =
            super::super::faces_config::load_faces_config_async(&album_dir).await;

        let stable_to_source: HashMap<&str, String> = photos
            .iter()
            .map(|p| (p.stable_key.as_str(), p.album_relative_path()))
            .collect();
        let mut detections_by_source: HashMap<
            String,
            Vec<super::super::faces_config::ReconcileDetection>,
        > = HashMap::new();
        for face in face_records {
            if !photos.iter().any(|p| p.stable_key == face.stable_key) {
                continue;
            }
            let Some(source) = stable_to_source.get(face.stable_key.as_str()) else {
                continue;
            };
            let Some(vector) = emb_by_face.get(face.face_id.as_str()) else {
                continue;
            };
            let norm = super::super::faces_config::normalize_source_path(source);
            detections_by_source.entry(norm).or_default().push(
                super::super::faces_config::ReconcileDetection {
                    face_id: face.face_id.clone(),
                    box_xywh: face.box_xywh,
                    embedding: (*vector).clone(),
                },
            );
        }
        let reconcile_stats = super::super::faces_config::reconcile_faces_config(
            &mut faces_config,
            &detections_by_source,
        );
        if reconcile_stats.updated > 0 {
            if let Err(error) =
                super::super::faces_config::save_faces_config_async(&album_dir, &faces_config).await
            {
                eprintln!("faces.json reconcile save failed for {owner}/{album_name}: {error}");
            }
        }

        let assigned = super::super::faces_config::AssignedFaceFilter::from_config(&faces_config);
        let named_catalog = super::super::faces_config::collect_named_person_centers(&faces_config);

        let cache = AlbumAiCache::new(ai_root, owner, &cache_id);
        let mut expected = HashSet::new();
        for photo in &photos {
            expected.insert(face_photo_cache_key(
                &photo.stable_key,
                &photo.content_signature,
                face_revision,
            ));
        }

        if let Ok(listed) = cache.list_face_photos_async().await {
            for (old_key, _) in listed {
                if !expected.contains(&old_key) {
                    let _ = tokio::fs::remove_file(cache.face_photo_path(&old_key)).await;
                }
            }
        }

        let (cluster_faces, face_metas) =
            collect_unassigned_faces_for_clustering(&cache, &assigned).await;

        let clusters = build_clusters_cache_with_matches_params(
            &cluster_faces,
            &face_metas,
            cluster_eps_from_similarity(faces_config.cluster_similarity),
            DEFAULT_CLUSTER_MIN_SAMPLES,
            &named_catalog,
        );
        let hints = super::super::album_cache::hints_from_clusters(&clusters);
        if let Err(error) = cache.write_face_index_async(&clusters).await {
            eprintln!("face_index write failed for {cache_id}: {error}");
        }
        if let Err(error) = cache.write_face_hints_async(&hints).await {
            eprintln!("face_hint write failed for {cache_id}: {error}");
        }
    }
}
