//! Top-level AI index pipeline.
//!
//! This is the entry point for understanding an index run. Domain details live
//! in the sibling modules; the order and shared state live here.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::backend::ai::models::{
    AiFaceRecord, AiIndexedAlbumSummary, AiIndexedPhotoRecord, LoadedOwnerIndex,
    NormalizedAiIndexRequest,
};
use crate::backend::ai::provider::{AiInferenceProvider, AiProviderHealth};
use crate::backend::common::AppState;
use crate::backend::tasks::{TaskBase, wait_while_paused};

use super::{duplicate, embedding, face, finalize, scan};

#[derive(Debug)]
pub(crate) enum IndexPipelineError {
    Cancelled,
    Failed(String),
}

struct IndexPipelineContext<'a> {
    cfg: &'a AppState,
    owner: &'a str,
    request: &'a NormalizedAiIndexRequest,
    provider: Arc<dyn AiInferenceProvider>,
    provider_health: AiProviderHealth,
    progress: &'a TaskBase,
    cancel: &'a Arc<AtomicBool>,
    paused: &'a AtomicBool,
    decode_parallelism: usize,
}

struct IndexPipelineState {
    indexed_albums: Vec<AiIndexedAlbumSummary>,
    indexed_photos: Vec<AiIndexedPhotoRecord>,
    indexed_embeddings: HashMap<String, Vec<f32>>,
    pending_image_embeds: Vec<(String, PathBuf, PathBuf)>,
    photo_disk_paths: HashMap<String, PathBuf>,
    previous_index: Option<LoadedOwnerIndex>,
    face_records: Vec<AiFaceRecord>,
}

/// Orchestrates one complete AI index run.
pub(crate) struct AiIndexPipeline<'a> {
    cfg: &'a AppState,
    locale: &'a str,
    request: &'a NormalizedAiIndexRequest,
    progress: &'a TaskBase,
    cancel: &'a Arc<AtomicBool>,
    paused: &'a AtomicBool,
}

impl<'a> AiIndexPipeline<'a> {
    pub(crate) fn new(
        cfg: &'a AppState,
        locale: &'a str,
        request: &'a NormalizedAiIndexRequest,
        progress: &'a TaskBase,
        cancel: &'a Arc<AtomicBool>,
        paused: &'a AtomicBool,
    ) -> Self {
        Self {
            cfg,
            locale,
            request,
            progress,
            cancel,
            paused,
        }
    }

    /// Complete pipeline, in execution order.
    pub(crate) async fn run(self) -> Result<(), IndexPipelineError> {
        let _permit = self.cfg.ai_limiter.acquire().await;
        let provider = self.cfg.ai_service.provider().await;
        let provider_health = provider.health().await;
        let context = IndexPipelineContext {
            cfg: self.cfg,
            owner: self.progress.owner.as_str(),
            request: self.request,
            provider,
            provider_health,
            progress: self.progress,
            cancel: self.cancel,
            paused: self.paused,
            decode_parallelism: self.cfg.settings_manager.ai_cpu_threads().await.max(1),
        };

        let mut state = Self::scan(&context, self.locale).await?;
        Self::index_duplicates(&context, &state).await?;
        Self::index_embeddings(&context, &mut state).await?;
        Self::index_faces(&context, &mut state).await?;
        Self::finalize(&context, &state).await
    }

    async fn scan(
        context: &IndexPipelineContext<'_>,
        locale: &str,
    ) -> Result<IndexPipelineState, IndexPipelineError> {
        let mut previous_index = context
            .cfg
            .ai_service
            .store()
            .load_current_index_async(context.owner)
            .await;
        let provider_revision = context.provider_health.revision.clone();
        let output = scan::run(
            context.cfg,
            locale,
            context.request,
            context.progress,
            context.cancel,
            context.paused,
            &mut previous_index,
            &provider_revision,
            context.request.has_feature("duplicates"),
            context.request.has_feature("image"),
            context.request.has_feature("faces"),
        )
        .await?;

        Ok(IndexPipelineState {
            indexed_albums: output.indexed_albums,
            indexed_photos: output.indexed_photos,
            indexed_embeddings: output.indexed_embeddings,
            pending_image_embeds: output.pending_image_embeds,
            photo_disk_paths: output.photo_disk_paths,
            previous_index,
            face_records: Vec::new(),
        })
    }

    async fn index_duplicates(
        context: &IndexPipelineContext<'_>,
        state: &IndexPipelineState,
    ) -> Result<(), IndexPipelineError> {
        let enabled = context.request.has_feature("duplicates")
            || state
                .indexed_photos
                .iter()
                .any(|photo| photo.duplicate_phash_valid);
        if !enabled {
            return Ok(());
        }
        duplicate::run(
            context.cfg.ai_service.store().root(),
            context.owner,
            &state.indexed_albums,
            &state.indexed_photos,
            context.progress,
            context.cancel,
            context.paused,
        )
        .await
        .map_err(|_| context.stage_error("failed to publish duplicates index"))
    }

    async fn index_embeddings(
        context: &IndexPipelineContext<'_>,
        state: &mut IndexPipelineState,
    ) -> Result<(), IndexPipelineError> {
        if !context.request.has_feature("image") {
            return Ok(());
        }
        embedding::run(
            context.cfg.ai_service.store().root(),
            Arc::clone(&context.provider),
            &state.pending_image_embeds,
            &mut state.indexed_photos,
            &mut state.indexed_embeddings,
            context.progress,
            Arc::clone(context.cancel),
            context.decode_parallelism,
        )
        .await
        .map_err(|_| context.stage_error("failed to index image embeddings"))
    }

    async fn index_faces(
        context: &IndexPipelineContext<'_>,
        state: &mut IndexPipelineState,
    ) -> Result<(), IndexPipelineError> {
        context.checkpoint().await?;
        state.face_records = face::run(
            context.cfg,
            Arc::clone(&context.provider),
            context.request.has_feature("faces"),
            &mut state.indexed_photos,
            &mut state.previous_index,
            &state.photo_disk_paths,
            context.progress,
            Arc::clone(context.cancel),
            context.paused,
            context.decode_parallelism,
        )
        .await
        .map_err(|_| context.stage_error("failed to index faces"))?
        .face_records;
        Ok(())
    }

    async fn finalize(
        context: &IndexPipelineContext<'_>,
        state: &IndexPipelineState,
    ) -> Result<(), IndexPipelineError> {
        context.checkpoint().await?;
        finalize::run(
            context.cfg,
            context.owner,
            context.request,
            &context.provider_health,
            &state.indexed_albums,
            &state.indexed_photos,
            &state.face_records,
            context.progress,
            context.cancel,
        )
        .await
        .map_err(|error| {
            if error == "cancelled" || context.is_cancelled() {
                IndexPipelineError::Cancelled
            } else {
                IndexPipelineError::Failed(error)
            }
        })
    }
}

impl IndexPipelineContext<'_> {
    fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn stage_error(&self, message: &str) -> IndexPipelineError {
        if self.is_cancelled() {
            IndexPipelineError::Cancelled
        } else {
            IndexPipelineError::Failed(message.to_string())
        }
    }

    async fn checkpoint(&self) -> Result<(), IndexPipelineError> {
        if wait_while_paused(self.cancel.as_ref(), self.paused).await || self.is_cancelled() {
            Err(IndexPipelineError::Cancelled)
        } else {
            Ok(())
        }
    }
}
