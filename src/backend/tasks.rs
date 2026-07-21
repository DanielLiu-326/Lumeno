use async_trait::async_trait;
use axum::extract::Path;
use axum::{Extension, Json};
use function_name::named;
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
            "error_msg": base.error_msg.lock().await.as_deref(),
            "cancellable": self.cancellable(),
            "pausable": self.pausable(),
        })
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
            if matches!(
                status,
                TaskStatus::Finished | TaskStatus::Failed | TaskStatus::Cancelled
            ) {
                finished.push(*k);
            }
        }
        if finished.len() >= MAX_FINISHED_TASKS {
            for k in &finished {
                tasks.remove(k);
            }
        }
    }

    pub async fn add_task(&self, task: Arc<dyn Task>) {
        let mut tasks = self.task_set.lock().await;
        Self::evict_finished_tasks(&mut tasks).await;
        tasks.insert(task.uuid(), task.clone());
        drop(tasks);
        tokio::spawn(async move {
            task.execute().await;
        });
    }

    /// Adds a task unless another pending or running task has the same key.
    /// Returns the active task ID and whether a new task was started.
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
            if matches!(
                status,
                TaskStatus::Pending | TaskStatus::Running | TaskStatus::Paused
            ) {
                return (existing.uuid(), false);
            }
        }

        let id = task.uuid();
        tasks.insert(id, task.clone());
        drop(tasks);
        tokio::spawn(async move {
            task.execute().await;
        });
        (id, true)
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
