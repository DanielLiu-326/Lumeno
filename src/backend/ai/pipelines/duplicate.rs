//! Duplicate domain pipeline: ensure pHash files + publish `duplicates_index.json`.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::backend::tasks::{TaskBase, format_subtask};

use super::super::album_cache::DEFAULT_NEAR_DUPLICATE_DISTANCE;
use super::super::duplicate::publish_duplicates_index_async;
use super::super::models::{AiIndexedAlbumSummary, AiIndexedPhotoRecord};

pub(crate) async fn run_duplicate_pipeline(
    ai_root: &Path,
    owner: &str,
    indexed_albums: &[AiIndexedAlbumSummary],
    indexed_photos: &[AiIndexedPhotoRecord],
    progress: &TaskBase,
    cancel: &Arc<AtomicBool>,
    paused: &AtomicBool,
) -> Result<(), ()> {
    let mut cache_ids = indexed_photos
        .iter()
        .map(|photo| photo.cache_id.clone())
        .filter(|id| !id.is_empty())
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

    let total = cache_ids.len().max(1);
    for (index, cache_id) in cache_ids.into_iter().enumerate() {
        if crate::backend::tasks::wait_while_paused(cancel.as_ref(), paused).await {
            return Err(());
        }
        if cancel.load(Ordering::Relaxed) {
            return Err(());
        }
        let label = indexed_albums
            .iter()
            .find(|album| album.cache_id == cache_id)
            .map(|album| album.album.as_str())
            .unwrap_or(cache_id.as_str());
        progress
            .set_cur_task(format_subtask(
                "indexing duplicates",
                label,
                index + 1,
                total,
            ))
            .await;
        if let Err(error) = publish_duplicates_index_async(
            ai_root,
            owner,
            cache_id.as_str(),
            indexed_photos,
            DEFAULT_NEAR_DUPLICATE_DISTANCE,
        )
        .await
        {
            eprintln!("duplicates index publish failed for {cache_id}: {error}");
            return Err(());
        }
    }
    Ok(())
}
