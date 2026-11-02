use crate::if_or;
use async_recursion::async_recursion;
use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{Multipart, Path, Query};
use axum::http::HeaderMap;
use axum::http::HeaderValue;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;
use axum::{Extension, Json};
use function_name::named;
use futures::FutureExt;
use futures::future::BoxFuture;
use headers::{ContentType, HeaderMapExt};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::any::Any;
use std::collections::HashSet;
use std::path::{Path as FsPath, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncSeekExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use axum::body::Bytes;

use super::common::{
    AppError, AppState, Token, create_response, into_host_path_safely, move_file_or_dir,
};
use super::i18n::{RequestLocale, tr, trf};
use super::tasks::{Task, TaskBase, TaskStatus, format_subtask, task_file_name};

const UPLOAD_FEED_IDLE_TIMEOUT: Duration = Duration::from_secs(120);

// ── Copy / Move helpers ──────────────────────────────────────────────────────

#[derive(Debug, Default, Clone, PartialEq)]
pub struct CopyProgress {
    pub current_file: usize,
    pub total_files: usize,
    pub total_bytes: u64,
    pub processed_bytes: u64,
}

#[async_recursion]
pub async fn collect_files_to_copy(
    source: &std::path::Path,
    dst: &std::path::Path,
    collected: &mut Vec<(PathBuf, PathBuf)>,
) -> (usize, u64) {
    collected.push((source.to_owned(), dst.to_owned()));
    if source.is_file() {
        let Ok(metadata) = tokio::fs::metadata(source).await else {
            return (0, 0);
        };
        return (1, metadata.len().into());
    }
    if !source.is_dir() {
        return (0, 0);
    }

    let (mut count, mut bytes) = (0usize, 0u64);
    let Ok(mut entries) = tokio::fs::read_dir(source).await else {
        return (0, 0);
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let source_path = entry.path();
        let source_path = source_path.as_path();
        let dest_path = dst.join(entry.file_name());
        let dest_path = dest_path.as_path();
        let (c, b) = collect_files_to_copy(source_path, dest_path, collected).await;
        count += c;
        bytes += b;
    }
    (count, bytes)
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum ExecControl {
    Continue,
    Cancel,
}

/// Copy through a sibling temp file and rename on success.
///
/// The destination is only replaced once every byte landed: a short write, a read
/// error or a cancel never leaves a truncated copy that still reports success.
async fn copy_file_with_progress<'a, T>(
    source: impl AsRef<std::path::Path>,
    destination: impl AsRef<std::path::Path>,
    mut progress_callback: T,
) -> tokio::io::Result<()>
where
    T: FnMut(u64, u64) -> BoxFuture<'a, ExecControl>,
{
    let source = source.as_ref();
    let destination = destination.as_ref();
    let parent = destination.parent().unwrap_or_else(|| FsPath::new("."));
    let temp = parent.join(format!(".lumeno-copy-{}.part", uuid::Uuid::new_v4()));

    let result = copy_into(source, &temp, &mut progress_callback).await;
    match result {
        Ok(()) => match tokio::fs::rename(&temp, destination).await {
            Ok(()) => Ok(()),
            Err(error) => {
                let _ = tokio::fs::remove_file(&temp).await;
                Err(error)
            }
        },
        Err(error) => {
            let _ = tokio::fs::remove_file(&temp).await;
            Err(error)
        }
    }
}

async fn copy_into<'a, T>(
    source: &FsPath,
    temp: &FsPath,
    progress_callback: &mut T,
) -> tokio::io::Result<()>
where
    T: FnMut(u64, u64) -> BoxFuture<'a, ExecControl>,
{
    let metadata = tokio::fs::metadata(source).await?;
    let total_bytes = metadata.len();

    let mut source_file = tokio::fs::File::open(source).await?;
    let mut temp_file = tokio::fs::File::create(temp).await?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut copied = 0u64;
    loop {
        let bytes_read = source_file.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        // `write_all`, not `write`: a short write would silently drop bytes while
        // progress still reached 100%.
        temp_file.write_all(&buffer[..bytes_read]).await?;
        copied += bytes_read as u64;
        if progress_callback(copied, total_bytes).await == ExecControl::Cancel {
            return Err(tokio::io::ErrorKind::BrokenPipe.into());
        }
    }
    temp_file.sync_all().await?;
    Ok(())
}

// ── CopyFiles task ───────────────────────────────────────────────────────────

struct CopyFiles {
    the_base: TaskBase,
    root_path: PathBuf,
    src: Vec<PathBuf>,
    dst: Vec<PathBuf>,
    conflict: ConflictAction,
    collected: Mutex<Vec<(PathBuf, PathBuf)>>,
    failures: Mutex<Vec<PathBuf>>,
    cancel: AtomicBool,
}
impl CopyFiles {
    fn new(
        name: String,
        owner: String,
        root_path: PathBuf,
        src: Vec<PathBuf>,
        dst: Vec<PathBuf>,
        conflict: ConflictAction,
    ) -> Self {
        Self {
            the_base: TaskBase::new(name, owner),
            root_path,
            src,
            dst,
            conflict,
            collected: Default::default(),
            failures: Default::default(),
            cancel: Default::default(),
        }
    }
}

#[async_trait]
impl Task for CopyFiles {
    fn base(&self) -> &TaskBase {
        &self.the_base
    }
    fn cancellable(&self) -> bool {
        true
    }
    async fn execute(&self) {
        let base = self.base();
        *base.status.lock().await = TaskStatus::Pending;
        let mut collected = Vec::new();
        for (src, dst) in self.src.iter().zip(self.dst.iter()) {
            let (c, b) = collect_files_to_copy(src.as_path(), dst.as_path(), &mut collected).await;
            base.set_total_count(c);
            base.set_total_cost(b);
        }
        *self.collected.lock().await = collected.clone();

        *base.status.lock().await = TaskStatus::Running;
        for (_idx, (src, dst)) in collected.iter().enumerate() {
            if src.is_file() {
                let done = base.processed_count.load(Ordering::Relaxed);
                let total = base.total_count.load(Ordering::Relaxed).max(1);
                *base.current_task.lock().await =
                    format_subtask("copying", &task_file_name(src.as_path()), done + 1, total);

                let effective_dst = if dst.exists() {
                    match self.conflict {
                        ConflictAction::Overwrite => dst.clone(),
                        ConflictAction::Skip => {
                            base.processed_count.fetch_add(1, Ordering::Relaxed);
                            if self.cancel.load(Ordering::Relaxed) {
                                break;
                            }
                            continue;
                        }
                        ConflictAction::Rename => find_free_path(dst.as_path()),
                    }
                } else {
                    dst.clone()
                };

                let other_cost = base.processed_cost.load(Ordering::Relaxed);
                #[rustfmt::skip]
                let cb = |copied, _| async move {
                    base.processed_cost.store(other_cost + copied, Ordering::Release);
                    if_or!(self.cancel.load(Ordering::Relaxed),ExecControl::Cancel, ExecControl::Continue)
                }.boxed();
                base.processed_count.fetch_add(1, Ordering::Relaxed);
                if let Err(_) =
                    copy_file_with_progress(src.as_path(), effective_dst.as_path(), cb).await
                {
                    self.failures.lock().await.push(src.clone());
                }
            } else if src.is_dir() {
                if let Err(_e) = tokio::fs::create_dir_all(dst).await {
                    self.failures.lock().await.push(src.clone());
                }
            }

            if self.cancel.load(Ordering::Relaxed) {
                break;
            }
        }
        *base.status.lock().await = if_or! {self.cancel.load(Ordering::Relaxed),TaskStatus::Cancelled,TaskStatus::Finished,};
    }

    async fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

// ── MoveFiles task ───────────────────────────────────────────────────────────

struct MoveFiles {
    base: TaskBase,
    root_path: PathBuf,
    src: Vec<PathBuf>,
    dst: Vec<PathBuf>,
    conflict: ConflictAction,
    failures: Mutex<Vec<PathBuf>>,
    cancel: AtomicBool,
}

impl MoveFiles {
    fn new(
        name: String,
        owner: String,
        root_path: PathBuf,
        src: Vec<PathBuf>,
        dst: Vec<PathBuf>,
        conflict: ConflictAction,
    ) -> Self {
        Self {
            base: TaskBase::new(name, owner),
            root_path,
            src,
            dst,
            conflict,
            failures: Default::default(),
            cancel: Default::default(),
        }
    }
}

#[async_trait]
impl Task for MoveFiles {
    fn base(&self) -> &TaskBase {
        &self.base
    }
    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Pending).await;
        self.base.set_total_count(self.src.len());
        self.base.set_total_cost(0);
        self.base.set_status(TaskStatus::Running).await;

        for (src, dst) in self.src.iter().zip(self.dst.iter()) {
            if self.cancel.load(Ordering::Relaxed) {
                break;
            }
            let done = self.base.processed_count.load(Ordering::Relaxed);
            let total = self.base.total_count.load(Ordering::Relaxed).max(1);
            self.base
                .set_cur_task(format_subtask(
                    "moving",
                    &task_file_name(src.as_path()),
                    done + 1,
                    total,
                ))
                .await;

            if src == dst {
                self.base.add_processed_count(1);
                continue;
            }

            let effective_dst = if dst.exists() {
                match self.conflict {
                    ConflictAction::Overwrite => {
                        let _ = tokio::fs::remove_file(dst).await;
                        dst.clone()
                    }
                    ConflictAction::Skip => {
                        self.base.add_processed_count(1);
                        continue;
                    }
                    ConflictAction::Rename => find_free_path(dst.as_path()),
                }
            } else {
                dst.clone()
            };

            if move_file_or_dir(src.as_path(), effective_dst.as_path())
                .await
                .is_err()
            {
                self.failures.lock().await.push(src.clone());
                continue;
            }

            self.base.add_processed_count(1);
        }

        if self.cancel.load(Ordering::Relaxed) {
            self.base.set_status(TaskStatus::Cancelled).await;
            return;
        }

        let failure_count = self.failures.lock().await.len();
        if failure_count > 0 {
            self.base
                .set_error_msg(trf(
                    "en",
                    "task.move_failed",
                    &[("count", failure_count.to_string())],
                ))
                .await;
            self.base.set_status(TaskStatus::Failed).await;
            return;
        }

        self.base.set_status(TaskStatus::Finished).await;
    }

    async fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

// ── Upload task ──────────────────────────────────────────────────────────────

/// Bytes accepted for one upload chunk, and the piece size used to hand them to
/// the upload task. Both bound peak memory per request.
const UPLOAD_FEED_MAX_BYTES: u64 = 32 * 1024 * 1024;
const UPLOAD_FEED_PIECE: usize = 1024 * 1024;

/// Removes its file on drop, so every early return cleans up the spool.
struct TempPath(Option<PathBuf>);

impl TempPath {
    fn new(path: PathBuf) -> Self {
        Self(Some(path))
    }

    fn as_path(&self) -> &FsPath {
        self.0.as_deref().unwrap_or_else(|| FsPath::new(""))
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub(crate) struct UploadFileMeta {
    pub(crate) bytes: u64,
    pub(crate) is_dir: bool,
    /// Confined host path the bytes are written to.
    pub(crate) path: String,
    /// Path as the client named it (relative to the upload destination). Chunk
    /// requests are matched on this, never on a re-derived host path.
    #[serde(default)]
    pub(crate) request_path: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct UploadMetas {
    files: Vec<UploadFileMeta>,
    destination: String,
    #[serde(default)]
    conflict: ConflictAction,
}

pub(crate) struct UploadChunk {
    request_path: String,
    bytes: Bytes,
}

pub(crate) struct UploadFiles {
    base: TaskBase,
    root_path: PathBuf,
    files: Vec<UploadFileMeta>,
    conflict: ConflictAction,
    ch_receiver: Mutex<tokio::sync::mpsc::Receiver<UploadChunk>>,
    ch_sender: Mutex<Option<tokio::sync::mpsc::Sender<UploadChunk>>>,
    cancel: AtomicBool,
    on_finished: Option<Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>>,
    dedup_key: Option<String>,
}

impl UploadFiles {
    pub(crate) fn new(
        name: String,
        owner: String,
        root_path: PathBuf,
        metas: Vec<UploadFileMeta>,
        conflict: ConflictAction,
    ) -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(10);
        Self {
            base: TaskBase::new(name, owner),
            root_path,
            files: metas,
            conflict,
            ch_receiver: Mutex::new(receiver),
            ch_sender: Mutex::new(Some(sender)),
            cancel: Default::default(),
            on_finished: None,
            dedup_key: None,
        }
    }

    pub(crate) fn with_on_finished(
        mut self,
        cb: impl Fn() -> BoxFuture<'static, ()> + Send + Sync + 'static,
    ) -> Self {
        self.on_finished = Some(Arc::new(cb));
        self
    }

    pub(crate) fn with_dedup_key(mut self, key: String) -> Self {
        self.dedup_key = Some(key);
        self
    }

    async fn feed_bytes(&self, request_path: String, bytes: Bytes) -> Result<(), AppError> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(AppError::BadRequest(tr("en", "error.task_cancelled")));
        }
        let guard = self.ch_sender.lock().await;
        let Some(ref sender) = *guard else {
            return Err(AppError::BadRequest(tr("en", "error.task_cancelled")));
        };
        sender
            .send(UploadChunk {
                request_path,
                bytes,
            })
            .await
            .map_err(|_| AppError::BadRequest(tr("en", "error.task_finished")))
    }
}

#[async_trait]
impl Task for UploadFiles {
    fn base(&self) -> &TaskBase {
        &self.base
    }
    fn cancellable(&self) -> bool {
        true
    }

    fn deduplication_key(&self) -> Option<String> {
        self.dedup_key.clone()
    }

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Pending).await;
        self.base.set_total_count(self.files.len());
        self.base
            .set_total_cost(self.files.iter().fold(0, |acc, f| acc + f.bytes));
        self.base.set_status(TaskStatus::Running).await;

        let set_error_and_stop = async |e| {
            self.base.set_status(TaskStatus::Failed).await;
            self.base.set_error_msg(e).await;
        };
        for file_meta in &self.files {
            if self.cancel.load(Ordering::Relaxed) {
                break;
            }
            let done = self.base.processed_count.load(Ordering::Relaxed);
            let total = self.base.total_count.load(Ordering::Relaxed).max(1);
            self.base
                .set_cur_task(format_subtask(
                    "uploading",
                    &task_file_name(FsPath::new(&file_meta.path)),
                    done + 1,
                    total,
                ))
                .await;
            let path = std::path::Path::new(&file_meta.path);
            if file_meta.is_dir {
                if let Err(e) = tokio::fs::create_dir_all(path).await {
                    set_error_and_stop(e.to_string()).await;
                    return;
                }
                self.base.add_processed_count(1);
                continue;
            }
            if let Some(parent) = path.parent()
                && let Err(e) = tokio::fs::create_dir_all(parent).await
            {
                set_error_and_stop(e.to_string()).await;
                return;
            }

            // Bytes go to a sibling temp file and are renamed into place only when
            // the whole file arrived. A cancelled or failed upload therefore never
            // truncates the destination, and two uploads of the same name cannot
            // overwrite each other's bytes mid-flight.
            let (final_path, skip_write) = if path.exists() {
                match self.conflict {
                    ConflictAction::Overwrite => (path.to_owned(), false),
                    ConflictAction::Skip => (path.to_owned(), true),
                    ConflictAction::Rename => (find_free_path(path), false),
                }
            } else {
                (path.to_owned(), false)
            };
            let temp_path = final_path
                .parent()
                .map(|parent| parent.join(format!(".lumeno-upload-{}.part", uuid::Uuid::new_v4())));
            // Also covers a panic or an early return: the partial file never stays
            // behind (after a successful rename the path no longer exists).
            let _temp_guard = temp_path.as_ref().map(|path| TempPath::new(path.clone()));

            let mut file: Option<tokio::fs::File> = if skip_write {
                None
            } else {
                let Some(temp) = temp_path.as_ref() else {
                    set_error_and_stop("error file path".to_string()).await;
                    return;
                };
                match tokio::fs::File::create(temp).await {
                    Ok(f) => Some(f),
                    Err(e) => {
                        set_error_and_stop(e.to_string()).await;
                        return;
                    }
                }
            };

            let mut bytes_written = 0u64;
            loop {
                if bytes_written == file_meta.bytes {
                    break;
                }
                if self.cancel.load(Ordering::Relaxed) {
                    break;
                }
                let chunk = {
                    let mut receiver = self.ch_receiver.lock().await;
                    match tokio::time::timeout(UPLOAD_FEED_IDLE_TIMEOUT, receiver.recv()).await {
                        Ok(Some(chunk)) => chunk,
                        Ok(None) => {
                            if self.cancel.load(Ordering::Relaxed) {
                                break;
                            }
                            set_error_and_stop(format!(
                                "upload stream closed before file completed: {}",
                                file_meta.path
                            ))
                            .await;
                            return;
                        }
                        Err(_) => {
                            if self.cancel.load(Ordering::Relaxed) {
                                break;
                            }
                            set_error_and_stop(format!(
                                "upload feed idle timeout after {}s: {}",
                                UPLOAD_FEED_IDLE_TIMEOUT.as_secs(),
                                file_meta.path
                            ))
                            .await;
                            return;
                        }
                    }
                };
                if chunk.request_path != file_meta.request_path {
                    drop(file);
                    if let Some(temp) = &temp_path {
                        let _ = tokio::fs::remove_file(temp).await;
                    }
                    set_error_and_stop(format!(
                        "upload chunk for {:?} arrived while writing {:?}",
                        chunk.request_path, file_meta.request_path
                    ))
                    .await;
                    return;
                }
                if bytes_written + chunk.bytes.len() as u64 > file_meta.bytes {
                    set_error_and_stop("error file length".to_string()).await;
                    return;
                }
                if let Some(ref mut f) = file {
                    if let Err(e) = f.write_all(&chunk.bytes).await {
                        set_error_and_stop(e.to_string()).await;
                        return;
                    }
                }
                bytes_written += chunk.bytes.len() as u64;
                if !skip_write {
                    self.base.add_processed_cost(chunk.bytes.len() as u64);
                }
            }
            if self.cancel.load(Ordering::Relaxed) {
                drop(file);
                if let Some(temp) = &temp_path {
                    let _ = tokio::fs::remove_file(temp).await;
                }
                break;
            }

            // Commit only after every byte arrived.
            if let Some(handle) = file.take() {
                if let Err(e) = handle.sync_all().await {
                    let _ = tokio::fs::remove_file(temp_path.as_ref().unwrap()).await;
                    set_error_and_stop(e.to_string()).await;
                    return;
                }
                drop(handle);
                let commit_path = if self.conflict == ConflictAction::Rename && final_path.exists()
                {
                    find_free_path(&final_path)
                } else {
                    final_path.clone()
                };
                if self.conflict == ConflictAction::Skip && final_path.exists() {
                    // It appeared while we were uploading; keep the existing file.
                    if let Some(temp) = &temp_path {
                        let _ = tokio::fs::remove_file(temp).await;
                    }
                } else if let Some(temp) = &temp_path
                    && let Err(e) = tokio::fs::rename(temp, &commit_path).await
                {
                    let _ = tokio::fs::remove_file(temp).await;
                    set_error_and_stop(e.to_string()).await;
                    return;
                }
            }
            self.base.add_processed_count(1);
        }
        if self.cancel.load(Ordering::Relaxed) {
            self.base.set_status(TaskStatus::Cancelled).await;
        } else {
            // Queue the follow-up (album import) while this task is still
            // Running so the import dedup key never has a gap.
            if let Some(ref cb) = self.on_finished {
                cb().await;
            }
            self.base.set_status(TaskStatus::Finished).await;
        }
    }

    async fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        // Drop the sender to close the channel; recv() returns None immediately
        self.ch_sender.lock().await.take();
    }
}

// ── DeleteFiles task ─────────────────────────────────────────────────────────

#[async_recursion]
async fn collect_files_to_delete(
    path: &std::path::Path,
    collected: &mut Vec<PathBuf>,
) -> (usize, u64) {
    if path.is_file() {
        let Ok(metadata) = tokio::fs::metadata(path).await else {
            return (0, 0);
        };
        collected.push(path.to_owned());
        return (1, metadata.len());
    }
    if !path.is_dir() {
        return (0, 0);
    }

    let (mut count, mut bytes) = (0usize, 0u64);
    let Ok(mut entries) = tokio::fs::read_dir(path).await else {
        return (0, 0);
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let child = entry.path();
        let (c, b) = collect_files_to_delete(&child, collected).await;
        count += c;
        bytes += b;
    }
    collected.push(path.to_owned());
    (count, bytes)
}

struct DeleteFiles {
    base: TaskBase,
    root_path: PathBuf,
    paths: Vec<PathBuf>,
    cancel: AtomicBool,
}

impl DeleteFiles {
    fn new(name: String, owner: String, root_path: PathBuf, paths: Vec<PathBuf>) -> Self {
        Self {
            base: TaskBase::new(name, owner),
            root_path,
            paths,
            cancel: Default::default(),
        }
    }
}

#[async_trait]
impl Task for DeleteFiles {
    fn base(&self) -> &TaskBase {
        &self.base
    }
    fn cancellable(&self) -> bool {
        true
    }

    async fn execute(&self) {
        let base = self.base();
        base.set_status(TaskStatus::Pending).await;

        let mut collected = Vec::new();
        let (mut total_count, mut total_bytes) = (0usize, 0u64);
        for p in &self.paths {
            let (c, b) = collect_files_to_delete(p.as_path(), &mut collected).await;
            total_count += c;
            total_bytes += b;
        }
        base.set_total_count(total_count);
        base.set_total_cost(total_bytes);
        base.set_status(TaskStatus::Running).await;

        for entry in &collected {
            if self.cancel.load(Ordering::Relaxed) {
                break;
            }
            let done = base.processed_count.load(Ordering::Relaxed);
            let total = base.total_count.load(Ordering::Relaxed).max(1);
            base.set_cur_task(format_subtask(
                "deleting",
                &task_file_name(entry.as_path()),
                done + 1,
                total,
            ))
            .await;
            if entry.is_file() {
                if let Ok(meta) = tokio::fs::metadata(&entry).await {
                    let size = meta.len();
                    let _ = tokio::fs::remove_file(&entry).await;
                    base.add_processed_count(1);
                    base.add_processed_cost(size);
                }
            } else if entry.is_dir() {
                let _ = tokio::fs::remove_dir(&entry).await;
            }
        }

        base.set_status(if self.cancel.load(Ordering::Relaxed) {
            TaskStatus::Cancelled
        } else {
            TaskStatus::Finished
        })
        .await;
    }

    async fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

// ── File API handlers ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub(crate) struct FileListQuery {
    offset: Option<u32>,
    limit: Option<u32>,
}

#[named]
pub async fn api_get_file_list(
    locale: RequestLocale,
    token: Token,
    path: Option<Path<String>>,
    query: Query<FileListQuery>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    eprintln!("{} called: {:?}", function_name!(), path);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let path = path.unwrap_or(Path(String::new()));
    let full_path = into_host_path_safely(root_path.as_path(), &path)?;
    if !full_path.exists() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", path.0.clone())],
        )));
    }
    if !full_path.is_dir() {
        let message = tr(locale.as_str(), "error.not_directory");
        return Ok(create_response(true, &message, 0, None));
    }
    let mut entries = tokio::fs::read_dir(&full_path).await.map_err(|_| {
        AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", path.0.clone())],
        ))
    })?;

    let mut files = Vec::new();
    let mut directories = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|_| {
        AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", path.0.clone())],
        ))
    })? {
        let metadata = entry.metadata().await.map_err(|_| {
            AppError::NotFound(trf(
                locale.as_str(),
                "error.resource_not_found",
                &[("value", path.0.clone())],
            ))
        })?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }

        let item = json!({
            "name": name,
            "size": metadata.len(),
            "modified": metadata.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs()),
            "is_dir": metadata.is_dir(),
            "is_file": metadata.is_file(),
        });

        if metadata.is_dir() {
            directories.push(item);
        } else {
            files.push(item);
        }
    }

    directories.sort_by(|a, b| a["name"].as_str().unwrap().cmp(b["name"].as_str().unwrap()));
    files.sort_by(|a, b| a["name"].as_str().unwrap().cmp(b["name"].as_str().unwrap()));

    directories.append(&mut files);

    let offset = query.offset.unwrap_or(0) as usize;
    let requested_limit = match query.limit {
        Some(n) => n,
        None => cfg.settings_manager.photos_per_page().await,
    };
    let limit = if requested_limit == 0 {
        usize::MAX
    } else {
        requested_limit as usize
    };

    let total_count = directories.len();
    let paginated_items = directories
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();

    let response_data = json!({
        "items": paginated_items,
        "total_count": total_count,
        "offset": offset,
        "limit": requested_limit,
    });
    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(true, &message, 0, Some(response_data)))
}

pub async fn send_file(full_path: PathBuf, disposition: bool) -> Result<Response<Body>, AppError> {
    if !full_path.exists() {
        return Err(AppError::NotFound(trf(
            "en",
            "error.resource_not_found",
            &[("value", full_path.display().to_string())],
        )));
    }
    if full_path.is_dir() {
        return Err(AppError::NotFound(trf(
            "en",
            "error.resource_not_found",
            &[("value", full_path.display().to_string())],
        )));
    }

    let file = match tokio::fs::File::open(&full_path).await {
        Ok(file) => file,
        Err(e) => {
            eprintln!("Failed to open file {:?}: {}", full_path, e);
            return Err(AppError::NotFound(trf(
                "en",
                "error.resource_not_found",
                &[("value", full_path.display().to_string())],
            )));
        }
    };
    let metadata = tokio::fs::metadata(&full_path).await.map_err(|_| {
        AppError::NotFound(trf(
            "en",
            "error.resource_not_found",
            &[("value", full_path.display().to_string())],
        ))
    })?;

    let file_name = full_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    let mime_type = mime_for_path(&full_path);
    let mut headers = HeaderMap::new();
    headers.typed_insert(ContentType::from(mime_type));

    if disposition {
        // ASCII fallback plus filename* so non-ASCII (e.g. Chinese) names survive
        // instead of panicking on HeaderValue::from_str.
        let fallback: String = file_name
            .chars()
            .map(|c| {
                if c.is_ascii() && !c.is_control() && c != '"' && c != '\\' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let fallback = if fallback.trim_matches('_').is_empty() {
            "file"
        } else {
            fallback.as_str()
        };
        let encoded_name = utf8_percent_encode(file_name, NON_ALPHANUMERIC).to_string();
        let content_disposition =
            format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded_name}");
        if let Ok(value) = HeaderValue::from_str(&content_disposition) {
            headers.insert("Content-Disposition", value);
        }
    }
    headers.insert(
        "Content-Length",
        metadata.len().to_string().parse().unwrap(),
    );

    let stream = ReaderStream::new(file);
    let body = Body::from_stream(stream);

    let mut ret = Response::builder()
        .status(StatusCode::OK)
        .body(body)
        .unwrap();
    *(ret.headers_mut()) = headers;
    Ok(ret)
}

struct ByteRange {
    start: u64,
    end_inclusive: u64,
}

fn parse_byte_range(header: &str, len: u64) -> Result<ByteRange, ()> {
    if len == 0 {
        return Err(());
    }
    let Some(rest) = header.trim().strip_prefix("bytes=") else {
        return Err(());
    };
    let spec = rest.split(',').next().unwrap_or("").trim();
    if spec.is_empty() {
        return Err(());
    }
    if let Some(suffix) = spec.strip_prefix('-') {
        if suffix.is_empty() {
            return Err(());
        }
        let n: u64 = suffix.parse().map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        let n = n.min(len);
        return Ok(ByteRange {
            start: len - n,
            end_inclusive: len - 1,
        });
    }
    let Some((start_s, end_s)) = spec.split_once('-') else {
        return Err(());
    };
    if start_s.is_empty() {
        return Err(());
    }
    let start: u64 = start_s.parse().map_err(|_| ())?;
    if start >= len {
        return Err(());
    }
    let end_inclusive = if end_s.is_empty() {
        len - 1
    } else {
        let end: u64 = end_s.parse().map_err(|_| ())?;
        if end < start {
            return Err(());
        }
        end.min(len - 1)
    };
    Ok(ByteRange {
        start,
        end_inclusive,
    })
}

fn range_not_satisfiable(len: u64) -> Result<Response<Body>, AppError> {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_RANGE,
        format!("bytes */{len}")
            .parse()
            .unwrap_or_else(|_| HeaderValue::from_static("bytes */0")),
    );
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    let mut ret = Response::builder()
        .status(StatusCode::RANGE_NOT_SATISFIABLE)
        .body(Body::empty())
        .unwrap();
    *(ret.headers_mut()) = headers;
    Ok(ret)
}

fn media_response_headers(path: &FsPath, content_length: u64) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.typed_insert(ContentType::from(mime_for_path(path)));
    headers.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        header::CONTENT_LENGTH,
        content_length.to_string().parse().unwrap(),
    );
    headers
}

/// Inline playback with `Accept-Ranges`. Photos keep `send_file(..., true)`.
pub async fn send_media_range(
    full_path: PathBuf,
    request_headers: &HeaderMap,
) -> Result<Response<Body>, AppError> {
    if !full_path.exists() || full_path.is_dir() {
        return Err(AppError::NotFound(trf(
            "en",
            "error.resource_not_found",
            &[("value", full_path.display().to_string())],
        )));
    }
    let metadata = tokio::fs::metadata(&full_path).await.map_err(|_| {
        AppError::NotFound(trf(
            "en",
            "error.resource_not_found",
            &[("value", full_path.display().to_string())],
        ))
    })?;
    let len = metadata.len();
    let range_header = request_headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    let file = tokio::fs::File::open(&full_path).await.map_err(|_| {
        AppError::NotFound(trf(
            "en",
            "error.resource_not_found",
            &[("value", full_path.display().to_string())],
        ))
    })?;

    if let Some(spec) = range_header {
        let range = match parse_byte_range(&spec, len) {
            Ok(range) => range,
            Err(()) => return range_not_satisfiable(len),
        };
        let mut file = file;
        file.seek(std::io::SeekFrom::Start(range.start))
            .await
            .map_err(|e| AppError::Unknown(format!("seek video failed: {e}")))?;
        let take_len = range.end_inclusive - range.start + 1;
        let mut headers = media_response_headers(&full_path, take_len);
        headers.insert(
            header::CONTENT_RANGE,
            format!("bytes {}-{}/{len}", range.start, range.end_inclusive)
                .parse()
                .map_err(|e| AppError::Unknown(format!("invalid content range header: {e}")))?,
        );
        let stream = ReaderStream::new(file.take(take_len));
        let mut ret = Response::builder()
            .status(StatusCode::PARTIAL_CONTENT)
            .body(Body::from_stream(stream))
            .unwrap();
        *(ret.headers_mut()) = headers;
        return Ok(ret);
    }

    let headers = media_response_headers(&full_path, len);
    let mut ret = Response::builder()
        .status(StatusCode::OK)
        .body(Body::from_stream(ReaderStream::new(file)))
        .unwrap();
    *(ret.headers_mut()) = headers;
    Ok(ret)
}

fn mime_for_path(path: &FsPath) -> mime_guess::mime::Mime {
    let ext = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("");
    if ext.eq_ignore_ascii_case("webmanifest") {
        return "application/manifest+json"
            .parse()
            .expect("valid mime type");
    }
    mime_guess::from_path(path).first_or_octet_stream()
}

pub async fn web_file(
    path: Option<Path<String>>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Response<Body>, AppError> {
    let requested = path.map(|p| p.0).unwrap_or_default();
    let full_path = into_host_path_safely(cfg.resources_path.as_path(), &requested)?;
    match send_file(full_path.clone(), false).await {
        Ok(mut response) => {
            if full_path.file_name().and_then(|name| name.to_str()) == Some("sw.js") {
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
            }
            Ok(response)
        }
        Err(err) => {
            if looks_like_static_asset(&requested) {
                return Err(err);
            }
            send_file(cfg.resources_path.join("index.html"), false).await
        }
    }
}

fn looks_like_static_asset(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    let Some(dot) = name.rfind('.') else {
        return false;
    };
    matches!(
        name[dot + 1..].to_ascii_lowercase().as_str(),
        "js" | "mjs"
            | "cjs"
            | "css"
            | "map"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "webp"
            | "svg"
            | "ico"
            | "woff"
            | "woff2"
            | "ttf"
            | "otf"
            | "json"
            | "webmanifest"
            | "txt"
            | "wasm"
            | "mp4"
            | "webm"
            | "html"
            | "xml"
    )
}

pub async fn web_egui_file(
    path: Option<Path<String>>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Response<Body>, AppError> {
    let path = path.unwrap_or(Path("index.html".to_owned()));
    let full_path = into_host_path_safely(cfg.egui_resources_path.as_path(), &path)?;
    send_file(full_path, false).await
}

#[derive(Debug, Deserialize)]
pub struct DownloadQuery {
    #[serde(default)]
    owner: Option<String>,
    /// `play=1` serves a video inline with HTTP Range. Photos ignore it.
    #[serde(default)]
    play: Option<String>,
}

fn wants_inline_playback(play: Option<&str>) -> bool {
    matches!(play, Some("1") | Some("true") | Some("yes"))
}

#[named]
pub async fn api_download_file(
    locale: RequestLocale,
    token: Token,
    path: Option<Path<String>>,
    query: Query<DownloadQuery>,
    headers: HeaderMap,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Response<Body>, AppError> {
    eprintln!("{} called: {:?}", function_name!(), path);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let path_str = path.map(|p| p.0).unwrap_or_default();

    // If an owner is specified, verify share access via album meta
    let effective_root = if let Some(ref owner) = query.owner {
        if owner != &username {
            use super::album::verify_shared_photo_access;
            verify_shared_photo_access(&cfg, &username, owner, &path_str, locale.as_str()).await?;
            cfg.user_root_path(owner)
        } else {
            root_path
        }
    } else {
        root_path
    };

    let full_path = into_host_path_safely(effective_root.as_path(), &path_str)?;
    if wants_inline_playback(query.play.as_deref())
        && crate::backend::album::media::is_video_path(&full_path)
    {
        return send_media_range(full_path, &headers).await;
    }
    // Attachment disposition: iOS Safari would otherwise open the photo inline in a
    // fullscreen preview with no way back instead of downloading.
    send_file(full_path, true).await
}

#[derive(Debug, Deserialize)]
pub(crate) struct DeleteFilesParam {
    paths: Vec<String>,
}

#[named]
pub async fn api_delete_files(
    locale: RequestLocale,
    token: Token,
    cfg: Extension<Arc<AppState>>,
    payload: Json<DeleteFilesParam>,
) -> Result<Json<Value>, AppError> {
    eprintln!("{} called: {:?}", function_name!(), payload.paths);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let param = payload.0;
    if param.paths.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.no_paths_delete",
        )));
    }
    let mut resolved = Vec::new();
    for p in &param.paths {
        let full = into_host_path_safely(root_path.as_path(), p)?;
        if !full.exists() {
            return Err(AppError::NotFound(trf(
                locale.as_str(),
                "error.resource_not_found",
                &[("value", p.clone())],
            )));
        }
        resolved.push(full);
    }
    let task_name = if param.paths.len() == 1 {
        trf(
            locale.as_str(),
            "task.delete_single",
            &[("path", param.paths[0].clone())],
        )
    } else {
        trf(
            locale.as_str(),
            "task.delete_multiple",
            &[("count", param.paths.len().to_string())],
        )
    };
    let task = Arc::new(DeleteFiles::new(
        task_name,
        username,
        root_path.clone(),
        resolved,
    ));
    cfg.task_mgr.add_task(task.clone()).await;
    let message = tr(locale.as_str(), "response.delete_task_created");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "task_id": task.uuid().to_string(),
        })),
    ))
}

#[derive(Debug, Deserialize)]
pub(crate) struct RenameParams {
    new_path: String,
}

#[named]
pub async fn api_rename_file(
    locale: RequestLocale,
    token: Token,
    path: Option<Path<String>>,
    query: Query<RenameParams>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    eprintln!("{} called: {:?}, {:?}", function_name!(), path, query);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let path = path.unwrap_or(Path(String::new())).0;
    // `Query` already percent-decoded this; a second pass would mangle literal `%`.
    let request_new_path = query.new_path.clone();
    let Ok(old_path) = into_host_path_safely(root_path.as_path(), &path) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_old_path",
        )));
    };
    let Ok(new_path) = into_host_path_safely(root_path.as_path(), &request_new_path) else {
        return Err(AppError::BadRequest(trf(
            locale.as_str(),
            "error.invalid_new_path",
            &[("path", request_new_path.clone())],
        )));
    };
    if !old_path.exists() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", path.clone())],
        )));
    }
    if old_path == new_path {
        let message = tr(locale.as_str(), "response.rename_success");
        return Ok(create_response(true, &message, 0, None));
    }
    if new_path.exists() {
        return Err(AppError::FileAlreadyExists(trf(
            locale.as_str(),
            "error.file_exists",
            &[("value", request_new_path.clone())],
        )));
    }
    move_file_or_dir(old_path.as_path(), new_path.as_path())
        .await
        .map_err(|e| AppError::Unknown(format!("{}", e)))?;

    let message = tr(locale.as_str(), "response.move_success");
    Ok(create_response(true, &message, 0, None))
}

#[named]
pub async fn api_new_folder(
    locale: RequestLocale,
    token: Token,
    Path(path): Path<String>,
    cfg: Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    eprintln!("{} called: {:?}", function_name!(), path);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let path = into_host_path_safely(root_path.as_path(), &path)?;
    if path.exists() {
        return Err(AppError::FileAlreadyExists(trf(
            locale.as_str(),
            "error.file_exists",
            &[("value", path.to_string_lossy().to_string())],
        )));
    }
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|e| AppError::Unknown(format!("{:?}", e)))?;
    let message = tr(locale.as_str(), "response.folder_created");
    Ok(create_response(true, &message, 0, None))
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConflictAction {
    /// Auto-rename the incoming file (e.g. "file (1).txt").
    #[default]
    Rename,
    /// Overwrite the existing file.
    Overwrite,
    /// Skip the incoming file if a conflict exists.
    Skip,
}

/// Find an available path by appending " (N)" before the extension.
fn find_free_path(path: &std::path::Path) -> PathBuf {
    if !path.exists() {
        return path.to_owned();
    }
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = path.parent().unwrap_or(std::path::Path::new(""));
    for i in 1u32.. {
        let candidate = parent.join(format!("{} ({}){}", stem, i, ext));
        if !candidate.exists() {
            return candidate;
        }
    }
    path.to_owned()
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct CopyFilesParam {
    sources: Vec<String>,
    destinations: Vec<String>,
    #[serde(default)]
    conflict: ConflictAction,
}

#[named]
pub async fn api_copy_files(
    locale: RequestLocale,
    token: Token,
    cfg: Extension<Arc<AppState>>,
    payload: Json<CopyFilesParam>,
) -> Result<Json<Value>, AppError> {
    eprintln!("{} called: {:?}", function_name!(), payload);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let param = payload.0;
    if param.sources.len() != param.destinations.len() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.path_counts_mismatch",
        )));
    }
    let (mut src_vec, mut dst_vec) = (Vec::new(), Vec::new());
    let task_name = trf(
        locale.as_str(),
        "task.copy_to",
        &[(
            "path",
            param.destinations.first().cloned().unwrap_or_default(),
        )],
    );
    for (src, dst) in param.sources.iter().zip(param.destinations) {
        let src = into_host_path_safely(root_path.as_path(), src)?;
        let dst = into_host_path_safely(root_path.as_path(), &dst)?;
        src_vec.push(src);
        dst_vec.push(dst);
    }
    let task = Arc::new(CopyFiles::new(
        task_name,
        username,
        root_path.clone(),
        src_vec,
        dst_vec,
        param.conflict,
    ));
    cfg.task_mgr.add_task(task.clone()).await;
    let message = tr(locale.as_str(), "response.task_created");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "task_id": task.uuid().to_string(),
        })),
    ))
}

#[named]
pub async fn api_move_files(
    locale: RequestLocale,
    token: Token,
    cfg: Extension<Arc<AppState>>,
    payload: Json<CopyFilesParam>,
) -> Result<Json<Value>, AppError> {
    eprintln!("{} called: {:?}", function_name!(), payload);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let param = payload.0;
    if param.sources.len() != param.destinations.len() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.path_counts_mismatch",
        )));
    }

    let (mut src_vec, mut dst_vec) = (Vec::new(), Vec::new());
    let task_name = trf(
        locale.as_str(),
        "task.move_to",
        &[(
            "path",
            param.destinations.first().cloned().unwrap_or_default(),
        )],
    );

    for (src, dst) in param.sources.iter().zip(param.destinations) {
        let src = into_host_path_safely(root_path.as_path(), src)?;
        let dst = into_host_path_safely(root_path.as_path(), &dst)?;
        src_vec.push(src);
        dst_vec.push(dst);
    }

    let task = Arc::new(MoveFiles::new(
        task_name,
        username,
        root_path.clone(),
        src_vec,
        dst_vec,
        param.conflict,
    ));
    cfg.task_mgr.add_task(task.clone()).await;
    let message = tr(locale.as_str(), "response.task_created");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "task_id": task.uuid().to_string(),
        })),
    ))
}

/// Resolve each upload entry against `dest_path` (already confined to the user root).
/// `Path::join` leaves `..` in place; `into_host_path_safely` collapses and rejects escape.
fn confine_upload_files(
    root_path: &std::path::Path,
    dest_path: &std::path::Path,
    files: &[UploadFileMeta],
    locale: &str,
) -> Result<Vec<UploadFileMeta>, AppError> {
    let mut confined = Vec::with_capacity(files.len());
    for meta in files {
        if meta.path.trim().is_empty() {
            return Err(AppError::BadRequest(tr(locale, "error.invalid_path")));
        }
        let path = into_host_path_safely(dest_path, &meta.path)?;
        // The client names chunks relative to the data root (`destination/name`).
        let request_path = path
            .strip_prefix(root_path)
            .map(|rel| rel.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|_| meta.path.clone());
        confined.push(UploadFileMeta {
            bytes: meta.bytes,
            is_dir: meta.is_dir,
            path: path.to_string_lossy().into_owned(),
            request_path,
        });
    }
    Ok(confined)
}

fn normalize_upload_list(
    base_path: &std::path::Path,
    metas: &[UploadFileMeta],
) -> Vec<UploadFileMeta> {
    let mut ret = metas.to_vec();
    let mut path_set = metas.iter().map(|m| m.path.clone()).collect::<HashSet<_>>();
    loop {
        let mut to_insert = Vec::new();
        for meta in metas {
            let path = PathBuf::from(&meta.path);
            if let Some(parent) = path.parent()
                && base_path != parent
            {
                let path_str: String = parent.to_string_lossy().into();
                if !path_set.contains(&path_str) {
                    path_set.insert(path_str.clone());
                    to_insert.push(UploadFileMeta {
                        bytes: 0,
                        is_dir: true,
                        path: path_str,
                        request_path: String::new(),
                    });
                }
            }
        }
        if to_insert.is_empty() {
            break;
        }
        ret.append(&mut to_insert);
    }

    ret.sort_by(|a, b| {
        let depth_a = a.path.matches('/').count();
        let depth_b = b.path.matches('/').count();

        if depth_a != depth_b {
            return depth_a.cmp(&depth_b);
        }
        a.path.cmp(&b.path)
    });
    ret
}

#[named]
pub async fn api_upload(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    metas: Json<UploadMetas>,
) -> Result<Json<Value>, AppError> {
    eprintln!("{} called: {:?}", function_name!(), metas);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    if metas.files.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.no_files_upload",
        )));
    }

    let dest_path = into_host_path_safely(root_path.as_path(), &metas.destination)?;
    let mut metas = metas.0;
    metas.files = confine_upload_files(&root_path, &dest_path, &metas.files, locale.as_str())?;
    metas.files = normalize_upload_list(&dest_path, &metas.files);
    let task_name = trf(
        locale.as_str(),
        "task.upload_to",
        &[("path", metas.destination.clone())],
    );
    let task = Arc::new(UploadFiles::new(
        task_name,
        username,
        root_path.clone(),
        metas.files,
        metas.conflict,
    ));
    cfg.task_mgr.add_task(task.clone()).await;

    let message = tr(locale.as_str(), "response.upload_started");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "task_id": task.uuid().to_string(),
        })),
    ))
}

#[named]
pub async fn api_upload_feed(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<Value>, AppError> {
    eprintln!("{} called", function_name!());
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let mut path = String::new();
    let mut task_id = String::new();
    // The chunk is spooled to disk instead of being held in memory: one request
    // used to pin up to the body limit, and the channel behind it held ten more.
    let mut spool: Option<TempPath> = None;
    let spool_dir = cfg.user_root_path(&username).join("temp");
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::BadRequest(tr(locale.as_str(), "error.multipart_payload")))?
    {
        let field_name = field.name().unwrap_or("").to_string();

        match field_name.as_str() {
            "path" => {
                path = field.text().await.map_err(|_| {
                    AppError::BadRequest(tr(locale.as_str(), "error.multipart_path"))
                })?;
            }
            "task_id" => {
                task_id = field.text().await.map_err(|_| {
                    AppError::BadRequest(tr(locale.as_str(), "error.multipart_task_id"))
                })?;
            }
            "file" => {
                if let Some(previous) = spool.take() {
                    drop(previous);
                }
                if let Err(error) = tokio::fs::create_dir_all(&spool_dir).await {
                    return Err(AppError::Unknown(error.to_string()));
                }
                let temp =
                    TempPath::new(spool_dir.join(format!("upload-feed-{}.part", Uuid::new_v4())));
                let mut handle = tokio::fs::File::create(temp.as_path()).await.map_err(|_| {
                    AppError::BadRequest(tr(locale.as_str(), "error.multipart_file_data"))
                })?;
                let mut written = 0u64;
                loop {
                    let chunk = match field.chunk().await {
                        Ok(Some(chunk)) => chunk,
                        Ok(None) => break,
                        Err(_) => {
                            return Err(AppError::BadRequest(tr(
                                locale.as_str(),
                                "error.multipart_file_data",
                            )));
                        }
                    };
                    written += chunk.len() as u64;
                    if written > UPLOAD_FEED_MAX_BYTES {
                        return Err(AppError::BadRequest(tr(
                            locale.as_str(),
                            "error.multipart_file_data",
                        )));
                    }
                    if let Err(error) = handle.write_all(&chunk).await {
                        return Err(AppError::Unknown(error.to_string()));
                    }
                }
                if let Err(error) = handle.sync_all().await {
                    return Err(AppError::Unknown(error.to_string()));
                }
                spool = Some(temp);
            }
            _ => {}
        }
    }
    let Some(spool) = spool else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.multipart_file_data",
        )));
    };
    let task_uuid = Uuid::from_str(task_id.as_str())
        .map_err(|_| AppError::BadRequest(tr(locale.as_str(), "error.request_id_missing")))?;
    let Some(task) = cfg.task_mgr.get_task_for_user(&task_uuid, &username).await else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.task_id_invalid",
        )));
    };
    let Some(task) = (&*task as &dyn Any).downcast_ref::<UploadFiles>() else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.upload_task_mismatch",
        )));
    };
    if path.trim().is_empty() || path.len() > 4096 {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.multipart_path",
        )));
    }
    // The chunk only names which file it belongs to; the confined host path came
    // from `/api/upload` and is the only one ever written.
    let mut reader = match tokio::fs::File::open(spool.as_path()).await {
        Ok(reader) => reader,
        Err(error) => return Err(AppError::Unknown(error.to_string())),
    };
    let mut buffer = vec![0u8; UPLOAD_FEED_PIECE];
    loop {
        let read = match reader.read(&mut buffer).await {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) => return Err(AppError::Unknown(error.to_string())),
        };
        task.feed_bytes(path.clone(), Bytes::copy_from_slice(&buffer[..read]))
            .await?;
    }
    drop(reader);
    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(true, &message, 0, None))
}

// Re-export DefaultBodyLimit for main.rs route configuration

#[cfg(test)]
mod range_tests {
    use super::{
        ExecControl, UploadFileMeta, confine_upload_files, copy_file_with_progress,
        parse_byte_range,
    };
    use crate::backend::common::into_host_path_safely;

    #[test]
    fn parses_open_closed_and_suffix_ranges() {
        let open = parse_byte_range("bytes=0-", 1000).unwrap();
        assert_eq!(open.start, 0);
        assert_eq!(open.end_inclusive, 999);

        let closed = parse_byte_range("bytes=100-199", 1000).unwrap();
        assert_eq!(closed.start, 100);
        assert_eq!(closed.end_inclusive, 199);

        let clamped = parse_byte_range("bytes=900-5000", 1000).unwrap();
        assert_eq!(clamped.end_inclusive, 999);

        let suffix = parse_byte_range("bytes=-200", 1000).unwrap();
        assert_eq!(suffix.start, 800);
        assert_eq!(suffix.end_inclusive, 999);
    }

    #[test]
    fn rejects_unsatisfiable_ranges() {
        assert!(parse_byte_range("bytes=1000-", 1000).is_err());
        assert!(parse_byte_range("bytes=20-10", 1000).is_err());
        assert!(parse_byte_range("bytes=-", 1000).is_err());
        assert!(parse_byte_range("items=0-1", 1000).is_err());
        assert!(parse_byte_range("bytes=0-", 0).is_err());
    }

    #[tokio::test]
    async fn copy_replaces_atomically_and_cancel_keeps_the_old_file() {
        use futures::FutureExt;

        let dir = std::env::temp_dir().join(format!("lumeno-copy-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("src.bin");
        let payload = vec![7u8; 200_000];
        std::fs::write(&source, &payload).unwrap();

        let dest = dir.join("dst.bin");
        std::fs::write(&dest, b"keep").unwrap();
        copy_file_with_progress(&source, &dest, |_, _| {
            async { ExecControl::Continue }.boxed()
        })
        .await
        .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), payload);

        let dest2 = dir.join("dst2.bin");
        std::fs::write(&dest2, b"keep").unwrap();
        let cancelled = copy_file_with_progress(&source, &dest2, |_, _| {
            async { ExecControl::Cancel }.boxed()
        })
        .await;
        assert!(cancelled.is_err(), "a cancelled copy reports failure");
        assert_eq!(
            std::fs::read(&dest2).unwrap(),
            b"keep",
            "the destination is untouched until the copy completes"
        );

        let leftovers = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().contains(".part"))
            .count();
        assert_eq!(leftovers, 0, "temp files are cleaned up");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn upload_file_paths_stay_under_destination() {
        let root =
            std::env::temp_dir().join(format!("lumeno-upload-confine-{}", uuid::Uuid::new_v4()));
        let dest = root.join("albums").join("foo");
        std::fs::create_dir_all(&dest).unwrap();
        let dest = into_host_path_safely(&root, "albums/foo").unwrap();

        let ok = confine_upload_files(
            &root,
            &dest,
            &[UploadFileMeta {
                bytes: 1,
                is_dir: false,
                path: "dir/a.jpg".into(),
                request_path: String::new(),
            }],
            "en",
        )
        .unwrap();
        assert_eq!(ok[0].path, dest.join("dir/a.jpg").to_string_lossy());
        assert_eq!(ok[0].request_path, "albums/foo/dir/a.jpg");

        assert!(
            confine_upload_files(
                &root,
                &dest,
                &[UploadFileMeta {
                    bytes: 1,
                    is_dir: true,
                    path: "../hmac_key.bin".into(),
                    request_path: String::new(),
                }],
                "en",
            )
            .is_err()
        );
        assert!(
            confine_upload_files(
                &root,
                &dest,
                &[UploadFileMeta {
                    bytes: 1,
                    is_dir: false,
                    path: "../../other/secret.jpg".into(),
                    request_path: String::new(),
                }],
                "en",
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
