//! Finalize domain pipeline: commit per-album AI generations (no per-photo cache rewrite).

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::backend::common::AppState;
use crate::backend::constants::AI_WRITE_COST;
use crate::backend::tasks::{TaskBase, format_subtask};

use super::super::album_cache::AlbumAiCache;
use super::super::models::{
    AiFaceRecord, AiIndexedAlbumSummary, AiIndexedPhotoRecord, NormalizedAiIndexRequest,
};
use super::super::provider::AiProviderHealth;

pub(crate) async fn run(
    cfg: &AppState,
    owner: &str,
    request: &NormalizedAiIndexRequest,
    provider_health: &AiProviderHealth,
    indexed_albums: &[AiIndexedAlbumSummary],
    indexed_photos: &[AiIndexedPhotoRecord],
    face_records: &[AiFaceRecord],
    progress: &TaskBase,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let cache_id_list: Vec<String> = {
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
        cache_ids.into_iter().collect()
    };

    let write_total = cache_id_list.len().max(1);
    let mut committed = Vec::new();
    let mut commit_errors = Vec::new();
    if let Err(error) = cfg.ai_service.store().check_writable() {
        return Err(format!("AI cache not writable: {error}"));
    }
    for (write_index, cache_id) in cache_id_list.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".to_string());
        }
        let album_label = indexed_albums
            .iter()
            .find(|album| album.cache_id == *cache_id)
            .map(|album| album.album.as_str())
            .unwrap_or(cache_id.as_str());
        progress
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
            if photo.cache_id != *cache_id {
                continue;
            }
            let new_id = album_photos.len();
            old_photo_id_to_new.insert(photo.photo_id, new_id);
            let mut photo = photo;
            photo.photo_id = new_id;
            album_photos.push(photo);
        }
        let mut album_faces = Vec::new();
        for face in face_records {
            let Some(&new_photo_id) = old_photo_id_to_new.get(&face.photo_id) else {
                continue;
            };
            let mut face = face.clone();
            face.photo_id = new_photo_id;
            album_faces.push(face);
        }
        let album_summaries = indexed_albums
            .iter()
            .filter(|album| album.cache_id == *cache_id)
            .cloned()
            .collect::<Vec<_>>();
        let album_photo_count = album_photos.len();
        let write_cost = (album_photo_count as u64).max(1) * AI_WRITE_COST;
        match cfg
            .ai_service
            .bootstrap_owner_index(
                owner,
                cache_id.as_str(),
                request,
                provider_health,
                &album_summaries,
                &album_photos,
                &album_faces,
                album_photo_count,
            )
            .await
        {
            Ok(generation_id) => {
                committed.push((cache_id.clone(), generation_id.clone(), album_photo_count));
                let cache =
                    AlbumAiCache::new(cfg.ai_service.store().root(), owner, cache_id.as_str());
                if let Err(error) = cache
                    .build_embedding_snapshot_async(generation_id.as_str())
                    .await
                {
                    eprintln!("embedding snapshot build failed for {cache_id}: {error}");
                }
            }
            Err(error) => {
                let message = format!("{cache_id}: {error}");
                eprintln!("index commit failed for {message}");
                commit_errors.push(message);
            }
        }
        // Count write work even when commit fails so the bar can reach 100%.
        progress.add_processed_cost(write_cost);
    }

    if cancel.load(Ordering::Relaxed) {
        return Err("cancelled".to_string());
    }

    progress.set_processed_cost(progress.total_cost.load(Ordering::Relaxed));

    if !cache_id_list.is_empty() && committed.is_empty() {
        let error = format!(
            "failed to commit any album index ({}): {}",
            cache_id_list.len(),
            commit_errors.join("; ")
        );
        progress.set_cur_task(error.clone()).await;
        return Err(error);
    }

    if cache_id_list.is_empty() && !indexed_photos.is_empty() {
        return Err(
            "no album cache_id available to commit index (photos missing cache_id)".to_string(),
        );
    }

    let summary = committed
        .iter()
        .map(|(cache_id, generation_id, count)| format!("{cache_id}:{generation_id}({count})"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut message = format!("committed {} album index(es): {summary}", committed.len());
    if !commit_errors.is_empty() {
        message.push_str(&format!(" ({} failed)", commit_errors.len()));
        for error in &commit_errors {
            eprintln!("index commit failed for {error}");
        }
    }
    progress.set_cur_task(message).await;
    Ok(())
}
