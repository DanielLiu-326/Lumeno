use async_trait::async_trait;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

use super::super::common::AppState;
use super::super::i18n::trf;
use super::super::tasks::{Task, TaskBase, TaskStatus};
use super::models::NormalizedAiIndexRequest;
use super::pipelines::{AiIndexPipeline, IndexPipelineError};
use serde_json::Value;

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

    async fn fail(&self, error: impl Into<String>) {
        self.base.set_error_msg(error.into()).await;
        self.base.set_status(TaskStatus::Failed).await;
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

        match AiIndexPipeline::new(
            self.cfg.as_ref(),
            self.locale.as_str(),
            &self.request,
            &self.base,
            &self.cancel,
            &self.paused,
        )
        .run()
        .await
        {
            Ok(()) => {
                self.base.set_status(TaskStatus::Finished).await;
            }
            Err(IndexPipelineError::Cancelled) => {
                let _ = self.abort_if_cancelled().await;
            }
            Err(IndexPipelineError::Failed(error)) => {
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
        Some(self.request.deduplication_key(self.base.owner.as_str()))
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
        .owner_has_current_index(owner.as_str())
        .await;
    if !ai_enabled && !has_index {
        return None;
    }

    let request = NormalizedAiIndexRequest {
        album: album.clone(),
        mode: "incremental".to_string(),
        features: NormalizedAiIndexRequest::default_incremental_features(),
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
