//! Embedding domain pipeline: cache miss → decode → batch infer → write each embedding.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::backend::album::decoder::{DecoderCreateOptions, ImageDecoder, create_decoder};
use crate::backend::constants::AI_EMBED_COST;
use crate::backend::tasks::{TaskBase, format_subtask, task_file_name};

use super::super::album_cache::{AlbumAiCache, EmbeddingCacheFile, embedding_cache_key};
use super::super::models::AiIndexedPhotoRecord;
use super::super::pipeline::{ImageJob, PipelineConfig, progress_channel, run_image_pipeline};
use super::super::provider::{AiInferenceProvider, AiItemError};

struct EmbedMeta {
    embedding_key: String,
    photo_path: PathBuf,
    stable_key: String,
    content_signature: String,
    album_relative: String,
    owner: String,
    cache_id: String,
    provider_revision: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
}

pub(crate) async fn run_embedding_pipeline(
    ai_root: &Path,
    provider: Arc<dyn AiInferenceProvider>,
    pending: &[(String, PathBuf, PathBuf)],
    indexed_photos: &mut [AiIndexedPhotoRecord],
    indexed_embeddings: &mut HashMap<String, Vec<f32>>,
    progress: &TaskBase,
    cancel: Arc<AtomicBool>,
    decode_parallelism: usize,
) -> Result<(), ()> {
    if pending.is_empty() {
        return Ok(());
    }

    let photo_lookup: HashMap<String, usize> = indexed_photos
        .iter()
        .enumerate()
        .map(|(i, photo)| (photo.embedding_key(), i))
        .collect();

    let jobs = pending
        .iter()
        .enumerate()
        .filter_map(|(index, (embedding_key, photo_path, decode_path))| {
            let photo_index = *photo_lookup.get(embedding_key)?;
            let photo = &indexed_photos[photo_index];
            Some(ImageJob {
                index,
                path: decode_path.clone(),
                meta: EmbedMeta {
                    embedding_key: embedding_key.clone(),
                    photo_path: photo_path.clone(),
                    stable_key: photo.stable_key.clone(),
                    content_signature: photo.content_signature.clone(),
                    album_relative: photo.album_relative_path(),
                    owner: photo.owner.clone(),
                    cache_id: photo.cache_id.clone(),
                    provider_revision: photo.provider_revision.clone(),
                    width: photo.width,
                    height: photo.height,
                },
            })
        })
        .collect::<Vec<_>>();

    if jobs.is_empty() {
        return Ok(());
    }

    let embed_total = jobs.len().max(1);
    let gpu_batch = provider.capabilities().max_batch_size.unwrap_or(8).max(1);
    let config = PipelineConfig::new("embedding", decode_parallelism.max(1), gpu_batch);
    let (progress_tx, mut progress_rx) = progress_channel(config.progress_queue_capacity());
    let display_paths: HashMap<usize, PathBuf> = jobs
        .iter()
        .map(|job| (job.index, job.meta.photo_path.clone()))
        .collect();

    let progress_task = {
        let base = progress;
        async move {
            let mut done = 0usize;
            while let Some((index, path)) = progress_rx.recv().await {
                done += 1;
                let display = display_paths
                    .get(&index)
                    .map(PathBuf::as_path)
                    .unwrap_or(path.as_path());
                base.set_cur_task(format_subtask(
                    "embedding images",
                    &task_file_name(display),
                    done,
                    embed_total,
                ))
                .await;
                base.add_processed_cost(AI_EMBED_COST);
            }
        }
    };

    let decoder: Arc<dyn ImageDecoder> =
        Arc::from(create_decoder(DecoderCreateOptions::auto()).expect("ai embedding decoder"));
    let ai_root = ai_root.to_path_buf();
    let ai_root_save = ai_root.clone();
    let results = Arc::new(Mutex::new(
        Vec::<(String, Result<Vec<f32>, AiItemError>)>::new(),
    ));
    let results_save = Arc::clone(&results);
    let provider_infer = Arc::clone(&provider);

    let pipeline = run_image_pipeline(
        jobs,
        config,
        decoder,
        Some(cancel),
        Some(progress_tx),
        move |images| {
            let provider = Arc::clone(&provider_infer);
            async move {
                let results = provider.embed_images(&images).await?;
                Ok((images, results))
            }
        },
        move |meta, result, _image| {
            let ai_root = ai_root_save.clone();
            let results_save = Arc::clone(&results_save);
            async move {
                if let Ok(ref vector) = result {
                    if !meta.cache_id.is_empty() {
                        let revision = meta.provider_revision.as_deref().unwrap_or("unknown");
                        let key = embedding_cache_key(
                            &meta.stable_key,
                            &meta.content_signature,
                            revision,
                        );
                        let mut file = EmbeddingCacheFile::from_vector(
                            &meta.stable_key,
                            &meta.album_relative,
                            vector,
                            meta.provider_revision.as_deref(),
                            Some(meta.content_signature.as_str()),
                        );
                        file.width = meta.width;
                        file.height = meta.height;
                        let cache = AlbumAiCache::new(&ai_root, &meta.owner, &meta.cache_id);
                        if let Err(error) = cache.write_embedding_async(&key, &file).await {
                            eprintln!(
                                "embedding cache write failed for {}: {error}",
                                meta.stable_key
                            );
                        }
                    }
                }
                results_save
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push((meta.embedding_key, result));
            }
        },
    );

    let (pipeline_result, _) = tokio::join!(pipeline, progress_task);
    if pipeline_result.is_err() {
        return Err(());
    }

    let saved = results
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
        .collect::<Vec<_>>();
    for (embedding_key, result) in saved {
        apply_embed_result(indexed_photos, indexed_embeddings, &embedding_key, result);
    }

    // Ensure reused (non-pending) ready embeddings also exist as file cache.
    for photo in indexed_photos.iter() {
        if photo.image_embedding_state != "ready" || photo.cache_id.is_empty() {
            continue;
        }
        let Some(vector) = indexed_embeddings
            .get(&photo.embedding_key())
            .or_else(|| indexed_embeddings.get(&photo.stable_key))
        else {
            continue;
        };
        let revision = photo.provider_revision.as_deref().unwrap_or("unknown");
        let key = embedding_cache_key(&photo.stable_key, &photo.content_signature, revision);
        let cache = AlbumAiCache::new(&ai_root, photo.owner.as_str(), photo.cache_id.as_str());
        if cache.read_embedding_async(&key).await.is_some() {
            continue;
        }
        let mut file = EmbeddingCacheFile::from_vector(
            &photo.stable_key,
            &photo.album_relative_path(),
            vector,
            photo.provider_revision.as_deref(),
            Some(photo.content_signature.as_str()),
        );
        file.width = photo.width;
        file.height = photo.height;
        if let Err(error) = cache.write_embedding_async(&key, &file).await {
            eprintln!(
                "embedding cache backfill failed for {}: {error}",
                photo.stable_key
            );
        }
    }
    Ok(())
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
