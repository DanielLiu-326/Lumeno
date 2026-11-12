//! Album workflows: one TaskManager entry for import/index/thumbnail/AI work.
//!
//! Each phase reuses the same domain function as the standalone task. The
//! group controls status, cancellation, pause, and phase labels; phases never
//! create child tasks.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use uuid::Uuid;

use crate::backend::ai::run_ai_index_for_album;
use crate::backend::common::AppState;
use crate::backend::files::ConflictAction;
use crate::backend::files::UploadFiles;
use crate::backend::i18n::{tr, trf};
use crate::backend::tasks::{StepError, Task, TaskContext, TaskGroup, TaskStep};

use super::helpers::{cleanup_upload_staging_sources, import_album_files, rebuild_album_index};
use super::media::generate_album_thumbnails;

struct ReindexStep {
    data_path: PathBuf,
    root_path: PathBuf,
    owner: String,
    album: String,
}

#[async_trait]
impl TaskStep for ReindexStep {
    fn label(&self, locale: &str) -> String {
        tr(locale, "task.step_index")
    }

    async fn run(&self, ctx: &TaskContext<'_>) -> Result<(), StepError> {
        match rebuild_album_index(
            &self.data_path,
            &self.root_path,
            &self.owner,
            &self.album,
            Some(ctx.base),
            Some(ctx.cancel.as_ref()),
            Some(ctx.paused),
        )
        .await
        {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(StepError::Cancelled),
            Err(error) => {
                if ctx.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    Err(StepError::Cancelled)
                } else {
                    Err(StepError::Failed(format!("{error}")))
                }
            }
        }
    }
}

struct ThumbnailStep {
    owner: String,
    album: String,
}

#[async_trait]
impl TaskStep for ThumbnailStep {
    fn label(&self, locale: &str) -> String {
        tr(locale, "task.step_thumbnails")
    }

    async fn run(&self, ctx: &TaskContext<'_>) -> Result<(), StepError> {
        let errors = generate_album_thumbnails(
            ctx.cfg.as_ref(),
            &self.owner,
            &self.album,
            ctx.base,
            ctx.cancel.as_ref(),
            ctx.paused,
        )
        .await;
        if ctx.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(StepError::Cancelled);
        }
        for error in errors {
            eprintln!("[album-workflow] thumbnail: {error}");
        }
        Ok(())
    }
}

struct AiStep {
    owner: String,
    album: String,
    locale: String,
}

#[async_trait]
impl TaskStep for AiStep {
    fn label(&self, locale: &str) -> String {
        tr(locale, "task.step_ai")
    }

    async fn run(&self, ctx: &TaskContext<'_>) -> Result<(), StepError> {
        match run_ai_index_for_album(
            ctx.cfg.as_ref(),
            &self.owner,
            &self.album,
            &self.locale,
            ctx.base,
            ctx.cancel,
            ctx.paused,
        )
        .await
        {
            Ok(false) => {
                ctx.base
                    .set_cur_task(tr(&self.locale, "task.step_ai_skipped"))
                    .await;
                Ok(())
            }
            Ok(true) => Ok(()),
            Err(error) if error == "cancelled" => Err(StepError::Cancelled),
            Err(error) => Err(StepError::Failed(error)),
        }
    }
}

struct UploadStep {
    upload: Arc<UploadFiles>,
}

#[async_trait]
impl TaskStep for UploadStep {
    fn label(&self, locale: &str) -> String {
        tr(locale, "task.step_upload")
    }

    async fn run(&self, ctx: &TaskContext<'_>) -> Result<(), StepError> {
        Task::execute(self.upload.as_ref()).await;
        let status = *self.upload.base().status.lock().await;
        match status {
            crate::backend::tasks::TaskStatus::Failed => {
                let error = self
                    .upload
                    .base()
                    .error_msg
                    .lock()
                    .await
                    .clone()
                    .unwrap_or_else(|| "upload failed".to_string());
                Err(StepError::Failed(error))
            }
            crate::backend::tasks::TaskStatus::Cancelled => Err(StepError::Cancelled),
            _ if ctx.cancel.load(std::sync::atomic::Ordering::Relaxed) => Err(StepError::Cancelled),
            _ => Ok(()),
        }
    }
}

struct ImportStep {
    root_path: PathBuf,
    username: String,
    album: String,
    sources: Vec<String>,
    move_files: bool,
    conflict: ConflictAction,
    locale: String,
}

#[async_trait]
impl TaskStep for ImportStep {
    fn label(&self, locale: &str) -> String {
        tr(locale, "task.step_import")
    }

    async fn run(&self, ctx: &TaskContext<'_>) -> Result<(), StepError> {
        match import_album_files(
            ctx.cfg,
            &self.root_path,
            &self.username,
            &self.album,
            &self.sources,
            self.move_files,
            self.conflict,
            Some(ctx.base),
            Some(ctx.cancel.as_ref()),
            false,
            &self.locale,
        )
        .await
        {
            Ok(_) => {
                cleanup_upload_staging_sources(&self.root_path, &self.sources).await;
                Ok(())
            }
            Err(error) => {
                if ctx.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    Err(StepError::Cancelled)
                } else {
                    Err(StepError::Failed(format!("{error}")))
                }
            }
        }
    }
}

fn tail_steps(
    cfg: &AppState,
    root_path: &std::path::Path,
    owner: &str,
    album: &str,
    locale: &str,
) -> Vec<Arc<dyn TaskStep>> {
    vec![
        Arc::new(ReindexStep {
            data_path: cfg.data_path.clone(),
            root_path: root_path.to_path_buf(),
            owner: owner.to_string(),
            album: album.to_string(),
        }),
        Arc::new(ThumbnailStep {
            owner: owner.to_string(),
            album: album.to_string(),
        }),
        Arc::new(AiStep {
            owner: owner.to_string(),
            album: album.to_string(),
            locale: locale.to_string(),
        }),
    ]
}

fn workflow_name(locale: &str, album: &str, key: &str) -> String {
    trf(locale, key, &[("album", album.to_string())])
}

pub(crate) async fn queue_reindex_only(
    cfg: Arc<AppState>,
    task_owner: String,
    root_path: PathBuf,
    owner: String,
    album: String,
    locale: String,
) -> (Uuid, bool) {
    let name = workflow_name(&locale, &album, "task.rebuild_index");
    let steps: Vec<Arc<dyn TaskStep>> = vec![Arc::new(ReindexStep {
        data_path: cfg.data_path.clone(),
        root_path,
        owner: owner.clone(),
        album: album.clone(),
    })];
    let group = TaskGroup::new(
        name,
        task_owner,
        cfg.clone(),
        locale,
        Some(format!("album-workflow:{owner}:{album}")),
    )
    .with_steps(steps);
    cfg.task_mgr.add_task_if_absent(Arc::new(group)).await
}

pub(crate) async fn queue_rebuild_workflow(
    cfg: Arc<AppState>,
    task_owner: String,
    root_path: PathBuf,
    owner: String,
    album: String,
    locale: String,
) -> (Uuid, bool) {
    let name = workflow_name(&locale, &album, "task.workflow_rebuild");
    let steps = tail_steps(cfg.as_ref(), root_path.as_path(), &owner, &album, &locale);
    let group = TaskGroup::new(
        name,
        task_owner,
        cfg.clone(),
        locale,
        Some(format!("album-workflow:{owner}:{album}")),
    )
    .with_steps(steps);
    cfg.task_mgr.add_task_if_absent(Arc::new(group)).await
}

pub(crate) async fn queue_import_workflow(
    cfg: Arc<AppState>,
    task_owner: String,
    root_path: PathBuf,
    owner: String,
    album: String,
    sources: Vec<String>,
    move_files: bool,
    conflict: ConflictAction,
    locale: String,
) -> (Uuid, bool) {
    let name = workflow_name(&locale, &album, "task.workflow_import");
    let mut steps: Vec<Arc<dyn TaskStep>> = vec![Arc::new(ImportStep {
        root_path: root_path.clone(),
        username: task_owner.clone(),
        album: album.clone(),
        sources,
        move_files,
        conflict,
        locale: locale.clone(),
    })];
    steps.extend(tail_steps(
        cfg.as_ref(),
        root_path.as_path(),
        &owner,
        &album,
        &locale,
    ));
    let group = TaskGroup::new(
        name,
        task_owner,
        cfg.clone(),
        locale,
        Some(format!("album-workflow:{owner}:{album}")),
    )
    .with_steps(steps);
    cfg.task_mgr.add_task_if_absent(Arc::new(group)).await
}

pub(crate) async fn queue_upload_workflow(
    cfg: Arc<AppState>,
    task_owner: String,
    root_path: PathBuf,
    owner: String,
    album: String,
    upload: Arc<UploadFiles>,
    staging_rel: String,
    conflict: ConflictAction,
    locale: String,
) -> (Uuid, bool) {
    let name = workflow_name(&locale, &album, "task.workflow_import");
    let mut steps: Vec<Arc<dyn TaskStep>> = vec![
        Arc::new(UploadStep {
            upload: upload.clone(),
        }),
        Arc::new(ImportStep {
            root_path: root_path.clone(),
            username: task_owner.clone(),
            album: album.clone(),
            sources: vec![staging_rel],
            move_files: true,
            conflict,
            locale: locale.clone(),
        }),
    ];
    steps.extend(tail_steps(
        cfg.as_ref(),
        root_path.as_path(),
        &owner,
        &album,
        &locale,
    ));
    let group = TaskGroup::new(
        name,
        task_owner,
        cfg.clone(),
        locale,
        Some(format!("album-workflow:{owner}:{album}")),
    )
    .with_steps(steps)
    .with_upload_feed(upload);
    cfg.task_mgr.add_task_if_absent(Arc::new(group)).await
}
