use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::Path;
use axum::{Extension, Json};
use function_name::named;
use futures::FutureExt;
use serde_json::{Value, json};
use std::any::Any;
use std::collections::HashMap;
use std::path::Path as FsPath;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::common::{AppError, AppState, Token, create_response};
use super::i18n::{RequestLocale, tr, trf};

const MAX_FINISHED_TASKS: usize = 100;

/// Basename only — never show absolute host paths in task UI.
pub fn task_file_name(path: &FsPath) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("?")
        .to_string()
}

/// Subtask line: `phase · file (current/total)` with 1-based current.
pub fn format_subtask(phase: &str, file_name: &str, current: usize, total: usize) -> String {
    let total = total.max(1);
    let current = current.clamp(1, total);
    if phase.is_empty() && file_name.is_empty() {
        format!("({current}/{total})")
    } else if phase.is_empty() {
        format!("{file_name} ({current}/{total})")
    } else if file_name.is_empty() {
        format!("{phase} ({current}/{total})")
    } else {
        format!("{phase} · {file_name} ({current}/{total})")
    }
}

/// Cooperative pause/cancel wait. Returns `true` when the task should abort
/// because `cancel` is set (including cancel-while-paused).
pub async fn wait_while_paused(cancel: &AtomicBool, paused: &AtomicBool) -> bool {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return true;
        }
        if !paused.load(Ordering::Relaxed) {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TaskStatus {
    Pending,
    Running,
    Paused,
    Finished,
    Cancelled,
    Failed,
}

impl TaskStatus {
    /// Still expected to make progress: holds its deduplication key.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            TaskStatus::Pending | TaskStatus::Running | TaskStatus::Paused
        )
    }

    /// Will not run again: its deduplication key is free.
    pub fn is_finished(self) -> bool {
        matches!(
            self,
            TaskStatus::Finished | TaskStatus::Failed | TaskStatus::Cancelled
        )
    }

    /// The wire value used by `status()`. Unknown numbers are not a task state.
    pub fn from_raw(value: u64) -> Option<Self> {
        Some(match value {
            0 => TaskStatus::Pending,
            1 => TaskStatus::Running,
            2 => TaskStatus::Paused,
            3 => TaskStatus::Finished,
            4 => TaskStatus::Cancelled,
            5 => TaskStatus::Failed,
            _ => return None,
        })
    }
}

pub struct TaskBase {
    pub uuid: Uuid,
    pub name: String,
    pub owner: String,
    pub status: Mutex<TaskStatus>,
    pub current_task: Mutex<String>,
    pub total_count: AtomicUsize,
    pub processed_count: AtomicUsize,
    pub total_cost: AtomicU64,
    pub processed_cost: AtomicU64,
    pub phase_index: AtomicUsize,
    pub phase_count: AtomicUsize,
    pub phase_name: Mutex<String>,
    pub error_msg: Mutex<Option<String>>,
}

impl TaskBase {
    pub fn new(name: String, owner: String) -> Self {
        Self {
            uuid: Uuid::new_v4(),
            name,
            owner,
            status: Mutex::new(TaskStatus::Pending),
            current_task: Default::default(),
            total_count: 0.into(),
            processed_count: 0.into(),
            total_cost: 0.into(),
            processed_cost: 0.into(),
            phase_index: 0.into(),
            phase_count: 0.into(),
            phase_name: Mutex::new(String::new()),
            error_msg: Mutex::new(None),
        }
    }

    pub async fn set_status(&self, status: TaskStatus) {
        *self.status.lock().await = status;
    }

    pub async fn set_cur_task(&self, msg: String) {
        *self.current_task.lock().await = msg;
    }

    pub fn set_total_count(&self, count: usize) {
        self.total_count.store(count, Ordering::Relaxed);
    }

    pub fn set_processed_count(&self, count: usize) {
        self.processed_count.store(count, Ordering::Relaxed);
    }

    pub async fn set_phase(&self, index: usize, count: usize, name: String) {
        self.phase_index.store(index, Ordering::Relaxed);
        self.phase_count.store(count, Ordering::Relaxed);
        *self.phase_name.lock().await = name;
    }

    pub fn add_processed_count(&self, count: usize) {
        self.processed_count.fetch_add(count, Ordering::Relaxed);
    }

    pub fn set_total_cost(&self, cost: u64) {
        self.total_cost.store(cost, Ordering::Relaxed);
    }

    pub fn set_processed_cost(&self, cost: u64) {
        self.processed_cost.store(cost, Ordering::Relaxed);
    }

    pub fn add_processed_cost(&self, cost: u64) {
        self.processed_cost.fetch_add(cost, Ordering::Relaxed);
    }

    pub async fn set_error_msg(&self, msg: String) {
        *self.error_msg.lock().await = Some(msg);
    }
}

#[async_trait]
pub trait Task: Send + Sync + Any {
    fn base(&self) -> &TaskBase;

    fn uuid(&self) -> Uuid {
        self.base().uuid
    }

    fn owner(&self) -> &str {
        self.base().owner.as_str()
    }

    async fn execute(&self);
    async fn cancel(&self);
    async fn pause(&self) {}
    async fn resume(&self, _extra_info: Value) {}

    fn deduplication_key(&self) -> Option<String> {
        None
    }

    fn cancellable(&self) -> bool {
        false
    }

    fn pausable(&self) -> bool {
        false
    }

    /// Tasks that accept chunked uploads expose a feed; composite workflows
    /// forward the feed of their first upload step.
    fn upload_feed(&self) -> Option<&dyn UploadFeed> {
        None
    }

    async fn status(&self) -> Value {
        let base = self.base();
        json!({
            "name": base.name,
            "status": (*base.status.lock().await) as u8,
            "current_task": &*base.current_task.lock().await,
            "total_count": base.total_count.load(Ordering::Relaxed),
            "processed_count": base.processed_count.load(Ordering::Relaxed),
            "total_cost": base.total_cost.load(Ordering::Relaxed),
            "processed_cost": base.processed_cost.load(Ordering::Relaxed),
            "phase_index": base.phase_index.load(Ordering::Relaxed),
            "phase_count": base.phase_count.load(Ordering::Relaxed),
            "phase_name": &*base.phase_name.lock().await,
            "error_msg": base.error_msg.lock().await.as_deref(),
            "cancellable": self.cancellable(),
            "pausable": self.pausable(),
        })
    }
}

#[derive(Debug)]
pub enum StepError {
    Cancelled,
    Failed(String),
}

pub struct TaskContext<'a> {
    pub cfg: &'a Arc<AppState>,
    pub base: &'a TaskBase,
    pub cancel: &'a Arc<AtomicBool>,
    pub paused: &'a AtomicBool,
}

#[async_trait]
pub trait TaskStep: Send + Sync {
    fn label(&self, locale: &str) -> String;
    async fn run(&self, ctx: &TaskContext<'_>) -> Result<(), StepError>;
}

#[async_trait]
pub trait UploadFeed: Send + Sync {
    async fn feed_bytes(&self, request_path: String, bytes: Bytes) -> Result<(), AppError>;
    async fn cancel(&self);
}

/// A single TaskManager entry that executes steps in order.
pub struct TaskGroup {
    base: TaskBase,
    cfg: Arc<AppState>,
    locale: String,
    cancel: Arc<AtomicBool>,
    paused: AtomicBool,
    steps: Vec<Arc<dyn TaskStep>>,
    upload_feed: Option<Arc<dyn UploadFeed>>,
    dedup_key: Option<String>,
}

impl TaskGroup {
    pub fn new(
        name: String,
        owner: String,
        cfg: Arc<AppState>,
        locale: String,
        dedup_key: Option<String>,
    ) -> Self {
        Self {
            base: TaskBase::new(name, owner),
            cfg,
            locale,
            cancel: Arc::new(AtomicBool::new(false)),
            paused: AtomicBool::new(false),
            steps: Vec::new(),
            upload_feed: None,
            dedup_key,
        }
    }

    pub fn with_steps(mut self, steps: Vec<Arc<dyn TaskStep>>) -> Self {
        self.steps = steps;
        self
    }

    pub fn with_upload_feed(mut self, feed: Arc<dyn UploadFeed>) -> Self {
        self.upload_feed = Some(feed);
        self
    }

    async fn finish_cancelled(&self) {
        self.base.set_cur_task("Cancelled".to_string()).await;
        self.base.set_status(TaskStatus::Cancelled).await;
    }

    async fn finish_failed(&self, error: String) {
        self.base.set_error_msg(error).await;
        self.base.set_status(TaskStatus::Failed).await;
    }
}

#[async_trait]
impl Task for TaskGroup {
    fn base(&self) -> &TaskBase {
        &self.base
    }

    fn deduplication_key(&self) -> Option<String> {
        self.dedup_key.clone()
    }

    fn cancellable(&self) -> bool {
        true
    }

    fn pausable(&self) -> bool {
        true
    }

    fn upload_feed(&self) -> Option<&dyn UploadFeed> {
        self.upload_feed.as_deref()
    }

    async fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.paused.store(false, Ordering::Relaxed);
        if let Some(feed) = &self.upload_feed {
            feed.cancel().await;
        }
        self.finish_cancelled().await;
    }

    async fn pause(&self) {
        self.paused.store(true, Ordering::Relaxed);
        self.base.set_status(TaskStatus::Paused).await;
    }

    async fn resume(&self, _extra_info: Value) {
        self.paused.store(false, Ordering::Relaxed);
        self.base.set_status(TaskStatus::Running).await;
    }

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Running).await;
        self.base
            .set_phase(0, self.steps.len(), String::new())
            .await;

        for (index, step) in self.steps.iter().enumerate() {
            if wait_while_paused(self.cancel.as_ref(), &self.paused).await
                || self.cancel.load(Ordering::Relaxed)
            {
                self.finish_cancelled().await;
                return;
            }
            self.base
                .set_phase(index + 1, self.steps.len(), step.label(&self.locale))
                .await;
            let ctx = TaskContext {
                cfg: &self.cfg,
                base: &self.base,
                cancel: &self.cancel,
                paused: &self.paused,
            };
            match step.run(&ctx).await {
                Ok(()) => {}
                Err(StepError::Cancelled) => {
                    self.finish_cancelled().await;
                    return;
                }
                Err(StepError::Failed(error)) => {
                    self.finish_failed(error).await;
                    return;
                }
            }
        }

        if self.cancel.load(Ordering::Relaxed) {
            self.finish_cancelled().await;
        } else {
            self.base.set_status(TaskStatus::Finished).await;
        }
    }
}

#[derive(Default)]
pub struct TaskManager {
    task_set: Mutex<HashMap<Uuid, Arc<dyn Task>>>,
}

impl TaskManager {
    async fn evict_finished_tasks(tasks: &mut HashMap<Uuid, Arc<dyn Task>>) {
        let mut finished = Vec::new();
        for (k, v) in tasks.iter() {
            let status = *v.base().status.lock().await;
            if status.is_finished() {
                finished.push(*k);
            }
        }
        if finished.len() >= MAX_FINISHED_TASKS {
            for k in &finished {
                tasks.remove(k);
            }
        }
    }

    /// Run the task on its own Tokio task, turning a panic into a Failed status.
    ///
    /// Without this a panicking task keeps its `Running` status forever, so the
    /// dedup key never frees and the UI shows the job as still in progress.
    fn spawn_task(task: Arc<dyn Task>) {
        tokio::spawn(async move {
            let base = task.base();
            let outcome = std::panic::AssertUnwindSafe(task.execute())
                .catch_unwind()
                .await;
            if let Err(payload) = outcome {
                let detail = payload
                    .downcast_ref::<&str>()
                    .map(|text| text.to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".to_string());
                eprintln!("task {} panicked: {}", base.name, detail);
                let status = *base.status.lock().await;
                if status.is_active() {
                    base.set_error_msg(detail).await;
                    base.set_status(TaskStatus::Failed).await;
                }
            }
        });
    }

    pub async fn add_task(&self, task: Arc<dyn Task>) {
        let mut tasks = self.task_set.lock().await;
        Self::evict_finished_tasks(&mut tasks).await;
        tasks.insert(task.uuid(), task.clone());
        drop(tasks);
        Self::spawn_task(task);
    }

    pub async fn add_task_if_absent(&self, task: Arc<dyn Task>) -> (Uuid, bool) {
        let Some(key) = task.deduplication_key() else {
            let id = task.uuid();
            self.add_task(task).await;
            return (id, true);
        };

        let mut tasks = self.task_set.lock().await;
        Self::evict_finished_tasks(&mut tasks).await;
        for existing in tasks.values() {
            if existing.deduplication_key().as_deref() != Some(key.as_str()) {
                continue;
            }
            let status = *existing.base().status.lock().await;
            if status.is_active() {
                return (existing.uuid(), false);
            }
        }

        let id = task.uuid();
        tasks.insert(id, task.clone());
        drop(tasks);
        Self::spawn_task(task);
        (id, true)
    }

    /// Any active task whose deduplication key starts with `prefix`.
    pub async fn has_active_key_prefix(&self, prefix: &str) -> bool {
        let tasks = self.task_set.lock().await;
        for existing in tasks.values() {
            let Some(key) = existing.deduplication_key() else {
                continue;
            };
            if !key.starts_with(prefix) {
                continue;
            }
            let status = *existing.base().status.lock().await;
            if status.is_active() {
                return true;
            }
        }
        false
    }

    pub async fn has_active_key(&self, key: &str) -> bool {
        let tasks = self.task_set.lock().await;
        for existing in tasks.values() {
            if existing.deduplication_key().as_deref() != Some(key) {
                continue;
            }
            let status = *existing.base().status.lock().await;
            if status.is_active() {
                return true;
            }
        }
        false
    }

    pub async fn get_task_for_user(&self, id: &Uuid, username: &str) -> Option<Arc<dyn Task>> {
        self.task_set
            .lock()
            .await
            .get(id)
            .filter(|task| task.owner() == username)
            .cloned()
    }

    pub async fn get_task_status_map_for_user(
        &self,
        username: &str,
    ) -> serde_json::Map<String, Value> {
        let tasks = self
            .task_set
            .lock()
            .await
            .iter()
            .filter(|(_, task)| task.owner() == username)
            .map(|(uuid, task)| (*uuid, task.clone()))
            .collect::<Vec<_>>();
        let mut status_map = serde_json::Map::new();
        for (uuid, value) in tasks {
            let status = value.status().await;
            status_map.insert(uuid.to_string(), status);
        }
        status_map
    }

    pub async fn clear_finished_for_user(&self, username: &str) -> usize {
        let mut tasks = self.task_set.lock().await;
        let mut to_remove = Vec::new();
        for (k, v) in &*tasks {
            if v.owner() != username {
                continue;
            }
            let status = *v.base().status.lock().await;
            if status == TaskStatus::Finished
                || status == TaskStatus::Failed
                || status == TaskStatus::Cancelled
            {
                to_remove.push(*k);
            }
        }
        for k in &to_remove {
            tasks.remove(k);
        }
        to_remove.len()
    }
}

// ── Task API handlers ────────────────────────────────────────────────────────

#[named]
pub async fn api_get_task_status(
    locale: RequestLocale,
    token: Token,
    Path(uuid): Path<Uuid>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let task = cfg.task_mgr.get_task_for_user(&uuid, &username).await;
    let Some(task) = task else {
        return Err(AppError::InvalidTask(trf(
            locale.as_str(),
            "error.invalid_task",
            &[("value", uuid.to_string())],
        )));
    };
    let status = task.status().await;
    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(true, &message, 0, Some(status)))
}

#[named]
pub async fn api_get_task_list(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let status_map = cfg.task_mgr.get_task_status_map_for_user(&username).await;
    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(true, &message, 0, Some(status_map.into())))
}

#[named]
pub async fn api_clear_finished_tasks(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    eprintln!("{} called:", function_name!());
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let count = cfg.task_mgr.clear_finished_for_user(&username).await;
    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "cleared": count,
        })),
    ))
}

#[named]
pub async fn api_cancel_task(
    locale: RequestLocale,
    token: Token,
    Path(uuid): Path<Uuid>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let task = cfg.task_mgr.get_task_for_user(&uuid, &username).await;
    let Some(task) = task else {
        return Err(AppError::InvalidTask(trf(
            locale.as_str(),
            "error.invalid_task",
            &[("value", uuid.to_string())],
        )));
    };
    if task.cancellable() {
        task.cancel().await;
        let message = tr(locale.as_str(), "response.success");
        Ok(create_response(true, &message, 0, None))
    } else {
        Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.task_not_cancellable",
        )))
    }
}

#[named]
pub async fn api_pause_task(
    locale: RequestLocale,
    token: Token,
    Path(uuid): Path<Uuid>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let task = cfg.task_mgr.get_task_for_user(&uuid, &username).await;
    let Some(task) = task else {
        return Err(AppError::InvalidTask(trf(
            locale.as_str(),
            "error.invalid_task",
            &[("value", uuid.to_string())],
        )));
    };
    if !task.pausable() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.task_not_pausable",
        )));
    }
    let status = *task.base().status.lock().await;
    if status != TaskStatus::Running {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.task_not_running",
        )));
    }
    task.pause().await;
    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(true, &message, 0, None))
}

#[named]
pub async fn api_resume_task(
    locale: RequestLocale,
    token: Token,
    Path(uuid): Path<Uuid>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let task = cfg.task_mgr.get_task_for_user(&uuid, &username).await;
    let Some(task) = task else {
        return Err(AppError::InvalidTask(trf(
            locale.as_str(),
            "error.invalid_task",
            &[("value", uuid.to_string())],
        )));
    };
    if !task.pausable() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.task_not_pausable",
        )));
    }
    let status = *task.base().status.lock().await;
    if status != TaskStatus::Paused {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.task_not_paused",
        )));
    }
    task.resume(json!({})).await;
    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(true, &message, 0, None))
}

#[cfg(test)]
mod panic_tests {
    use super::*;
    use std::time::Duration;

    struct PanicTask {
        base: TaskBase,
    }

    #[async_trait]
    impl Task for PanicTask {
        fn base(&self) -> &TaskBase {
            &self.base
        }

        fn deduplication_key(&self) -> Option<String> {
            Some(format!("panic-test:{}", self.base.owner))
        }

        async fn execute(&self) {
            panic!("boom");
        }

        async fn cancel(&self) {}
    }

    #[tokio::test]
    async fn a_panicking_task_ends_failed_and_frees_its_key() {
        let manager = TaskManager::default();
        let task = Arc::new(PanicTask {
            base: TaskBase::new("panic".to_string(), "alice".to_string()),
        });
        manager.add_task(task.clone()).await;

        let mut status = TaskStatus::Running;
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            status = *task.base.status.lock().await;
            if status == TaskStatus::Failed {
                break;
            }
        }
        assert_eq!(status, TaskStatus::Failed);
        assert_eq!(
            task.base.error_msg.lock().await.as_deref(),
            Some("boom"),
            "the panic message is kept for the task list"
        );
        assert!(
            !manager.has_active_key("panic-test:alice").await,
            "a failed task must not hold its dedup key"
        );
    }
}

#[cfg(test)]
mod status_tests {
    use super::TaskStatus;

    #[test]
    fn wire_values_round_trip_and_unknown_values_are_rejected() {
        for status in [
            TaskStatus::Pending,
            TaskStatus::Running,
            TaskStatus::Paused,
            TaskStatus::Finished,
            TaskStatus::Cancelled,
            TaskStatus::Failed,
        ] {
            assert_eq!(TaskStatus::from_raw(status as u64), Some(status));
        }
        assert_eq!(TaskStatus::from_raw(6), None);
        assert_eq!(TaskStatus::from_raw(u64::MAX), None);
    }

    #[test]
    fn active_and_finished_partition_every_state() {
        for status in [
            TaskStatus::Pending,
            TaskStatus::Running,
            TaskStatus::Paused,
            TaskStatus::Finished,
            TaskStatus::Cancelled,
            TaskStatus::Failed,
        ] {
            assert_ne!(
                status.is_active(),
                status.is_finished(),
                "{status:?} must be exactly one of active/finished"
            );
        }
        assert!(TaskStatus::Paused.is_active());
        assert!(TaskStatus::Failed.is_finished());
    }
}
