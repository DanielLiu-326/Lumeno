#![feature(normalize_lexically)]
#![feature(trait_alias)]
#![feature(generic_atomic)]

use async_recursion::async_recursion;
use async_trait::async_trait;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, FromRequestParts, Multipart, Path, Query};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Extension, Json, Router, routing::get};
use base64::Engine;
use function_name::named;
use futures::FutureExt;
use futures::future::BoxFuture;
use headers::{ContentType, HeaderMapExt};
use percent_encoding::{percent_decode, percent_decode_str};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha512};
use std::any::Any;
use std::collections::HashSet;
use std::fmt::Debug;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use thiserror::Error;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

mod backend;

use backend::auth::{SessionInfo, SessionManager, UserManager, extract_token_from_cookie};
use backend::tasks::{Task, TaskBase, TaskManager, TaskStatus};

macro_rules! if_or {
    // 基本用法: if_or!(condition, then_value, else_value)
    ($condition:expr, $then:expr, $else:expr,) => {
        if $condition { $then } else { $else }
    };
    ($condition:expr, $then:expr, $else:expr) => {
        if $condition { $then } else { $else }
    };
}

pub struct ScopeGuard<F: FnOnce()> {
    drop_fn: Option<F>,
}

impl<F: FnOnce()> ScopeGuard<F> {
    pub fn new(drop_fn: F) -> Self {
        Self {
            drop_fn: Some(drop_fn),
        }
    }

    pub fn defuse(mut self) {
        self.drop_fn.take();
    }
}

impl<F: FnOnce()> Drop for ScopeGuard<F> {
    fn drop(&mut self) {
        if let Some(f) = self.drop_fn.take() {
            f();
        }
    }
}

pub fn sha512_string_base64(data: &[u8]) -> String {
    let mut hasher = Sha512::new();
    hasher.update(data);
    let result = hasher.finalize();
    base64::engine::general_purpose::STANDARD.encode(result)
}

struct Token(String);

impl AsRef<String> for Token {
    fn as_ref(&self) -> &String {
        &self.0
    }
}

impl Into<String> for Token {
    fn into(self) -> String {
        self.0.clone()
    }
}

impl<S> FromRequestParts<S> for Token
where
    S: Send + Sync,
{
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        async {
            let Some(state) = parts.extensions.get::<Arc<AppState>>() else {
                return Err(AppError::Unknown("No AppConfig found".to_owned()));
            };

            let Some(token) = extract_token_from_cookie(&parts.headers) else {
                return Err(AppError::Unauthorized);
            };
            if state
                .session_manager
                .get_session_info(&token)
                .await
                .is_none()
            {
                return Err(AppError::Expired);
            }
            Ok(Token(token))
        }
    }
}

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

    let metadata = tokio::fs::metadata(source).await?;
    let total_bytes = metadata.len();

    let mut source_file = tokio::fs::File::open(source).await?;
    let mut dest_file = tokio::fs::File::create(destination).await?;
    let mut buffer = vec![0u8; 8192];
    let mut copied = 0u64;
    loop {
        let bytes_read = source_file.read(&mut buffer).await?;
        if bytes_read == 0 {
            break;
        }
        dest_file.write(&buffer[..bytes_read]).await?;
        copied += bytes_read as u64;
        if progress_callback(copied, total_bytes).await == ExecControl::Cancel {
            tokio::fs::remove_file(destination).await?;
            return Err(tokio::io::ErrorKind::BrokenPipe.into());
        }
    }
    dest_file.sync_all().await?;
    Ok(())
}

struct CopyFiles {
    the_base: TaskBase,
    src: Vec<PathBuf>,
    dst: Vec<PathBuf>,
    collected: Mutex<Vec<(PathBuf, PathBuf)>>,
    failures: Mutex<Vec<PathBuf>>,
    cancel: AtomicBool,
}
impl CopyFiles {
    fn new(name: String, owner: String, src: Vec<PathBuf>, dst: Vec<PathBuf>) -> Self {
        Self {
            the_base: TaskBase::new(name, owner),
            src,
            dst,
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
        for (idx, (src, dst)) in collected.iter().enumerate() {
            if src.is_file() {
                *base.current_task.lock().await =
                    src.file_name().unwrap_or_default().to_string_lossy().into();
                let other_cost = base.processed_cost.load(Ordering::Relaxed);
                #[rustfmt::skip]
                let cb = |copied, _| async move {
                    base.processed_cost.store(other_cost + copied, Ordering::Release);
                    if_or!(self.cancel.load(Ordering::Relaxed),ExecControl::Cancel, ExecControl::Continue)
                }.boxed();
                base.processed_count.fetch_add(1, Ordering::Relaxed);
                if let Err(_) = copy_file_with_progress(src.as_path(), dst.as_path(), cb).await {
                    self.failures.lock().await.push(src.clone());
                }
            } else if src.is_dir() {
                if let Err(e) = tokio::fs::create_dir_all(dst).await {
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

struct MoveFiles {
    base: TaskBase,
    src: Vec<PathBuf>,
    dst: Vec<PathBuf>,
    failures: Mutex<Vec<PathBuf>>,
    cancel: AtomicBool,
}

impl MoveFiles {
    fn new(name: String, owner: String, src: Vec<PathBuf>, dst: Vec<PathBuf>) -> Self {
        Self {
            base: TaskBase::new(name, owner),
            src,
            dst,
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

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Pending).await;
        self.base.set_total_count(self.src.len());
        self.base.set_total_cost(0);
        self.base.set_status(TaskStatus::Running).await;

        for (src, dst) in self.src.iter().zip(self.dst.iter()) {
            if self.cancel.load(Ordering::Relaxed) {
                break;
            }
            self.base
                .set_cur_task(src.file_name().unwrap_or_default().to_string_lossy().into())
                .await;

            if src == dst {
                self.base.add_processed_count(1);
                continue;
            }

            if dst.exists() {
                self.failures.lock().await.push(src.clone());
                continue;
            }

            if move_file_or_dir(src.as_path(), dst.as_path())
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
                .set_error_msg(format!("{} 项移动失败", failure_count))
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

#[derive(Debug, Deserialize, Clone)]
struct UploadFileMeta {
    bytes: u64,
    is_dir: bool,
    path: String,
}

#[derive(Debug, Deserialize)]
struct UploadMetas {
    files: Vec<UploadFileMeta>,
    destination: String,
}

struct UploadChunk {
    path: PathBuf,
    bytes: Bytes,
}

struct UploadFiles {
    base: TaskBase,
    files: Vec<UploadFileMeta>,
    ch_receiver: Mutex<tokio::sync::mpsc::Receiver<UploadChunk>>,
    ch_sender: Mutex<tokio::sync::mpsc::Sender<UploadChunk>>,
}

impl UploadFiles {
    fn new(name: String, owner: String, metas: Vec<UploadFileMeta>) -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(10);
        Self {
            base: TaskBase::new(name, owner),
            files: metas,
            ch_receiver: Mutex::new(receiver),
            ch_sender: Mutex::new(sender),
        }
    }
    async fn feed_bytes(&self, file: PathBuf, bytes: Bytes) -> Result<(), AppError> {
        self.ch_sender
            .lock()
            .await
            .send(UploadChunk { path: file, bytes })
            .await
            .map_err(|_| AppError::BadRequest("task finished".to_string()))
    }
}

#[async_trait]
impl Task for UploadFiles {
    fn base(&self) -> &TaskBase {
        &self.base
    }

    async fn execute(&self) {
        // 初始化
        self.base.set_status(TaskStatus::Pending).await;
        self.base.set_total_count(self.files.len());
        self.base
            .set_total_cost(self.files.iter().fold(0, |acc, f| acc + f.bytes));
        self.base.set_status(TaskStatus::Running).await;

        let set_error_and_stop = async |e| {
            self.base.set_status(TaskStatus::Failed).await;
            self.base.set_error_msg(e).await;
        };
        // 开始接收
        for file_meta in &self.files {
            self.base.set_cur_task(file_meta.path.clone()).await;
            let path = std::path::Path::new(&file_meta.path);
            // 如果是目录
            if file_meta.is_dir {
                if let Err(e) = tokio::fs::create_dir_all(path).await {
                    set_error_and_stop(e.to_string()).await;
                    return;
                }
                self.base.add_processed_count(1);
                continue;
            }
            // 如果是文件
            if let Some(parent) = path.parent()
                && let Err(e) = tokio::fs::create_dir_all(parent).await
            {
                set_error_and_stop(e.to_string()).await;
                return;
            }
            let mut file = match tokio::fs::File::create(path).await {
                Ok(f) => f,
                Err(e) => {
                    set_error_and_stop(e.to_string()).await;
                    return;
                }
            };
            let mut bytes_written = 0;
            loop {
                println!("{}:{},{}", &file_meta.path, bytes_written, file_meta.bytes);
                if bytes_written == file_meta.bytes {
                    break;
                }
                let Some(chunk) = self.ch_receiver.lock().await.recv().await else {
                    set_error_and_stop("task timeout".to_string()).await;
                    return;
                };
                println!("write {} bytes", chunk.bytes.len());
                if chunk.path != path {
                    set_error_and_stop(format!(
                        "expected path{:?}:, request path:{:?}",
                        path,
                        chunk.path.as_path()
                    ))
                    .await;
                    return;
                }
                if bytes_written + chunk.bytes.len() as u64 > file_meta.bytes {
                    set_error_and_stop("error file length".to_string()).await;
                    return;
                }
                if let Err(e) = file.write_all(&chunk.bytes).await {
                    set_error_and_stop(e.to_string()).await;
                    return;
                }
                bytes_written += chunk.bytes.len() as u64;
                self.base.add_processed_cost(chunk.bytes.len() as u64);
            }
            self.base.add_processed_count(1);
        }
        self.base.set_status(TaskStatus::Finished).await;
        return;
    }

    async fn cancel(&self) {}
}

struct AppState {
    data_path: PathBuf,
    resources_path: PathBuf,
    task_mgr: Arc<TaskManager>,
    user_manager: Arc<UserManager>,
    session_manager: Arc<SessionManager>,
}

impl AppState {
    async fn get_session_info(&self, token: &Token) -> Option<SessionInfo> {
        self.session_manager
            .get_session_info(token.0.as_str())
            .await
    }

    async fn get_username(&self, token: &Token) -> Option<String> {
        self.get_session_info(token)
            .await
            .map(|session| session.username)
    }

    async fn get_root_path(&self, token: &Token) -> Option<PathBuf> {
        let Some(session) = self.get_session_info(token).await else {
            return None;
        };
        Some(self.data_path.join(session.username))
    }
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("未找到资源: {0}")]
    NotFound(String),

    #[error("文件已存在: {0}")]
    FileAlreadyExists(String),

    #[error("无效请求: {0}")]
    BadRequest(String),

    #[error("无效任务: {0}")]
    InvalidTask(Uuid),

    #[error("用户已存在: {0}")]
    UserAlreadyExists(String),

    #[error("无效用户名: {0}")]
    UserNameInvalid(String),

    #[error("用户名或密码错误: {0}")]
    ErrorUserNameOrPassword(String),

    #[error("未验证的请求")]
    Unauthorized,

    #[error("过期的认证")]
    Expired,

    #[error("传输错误: {0}")]
    TransportError(String),

    #[error("未知错误: {0}")]
    Unknown(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let msg = format!("{}", &self);
        let (code, msg, success) = match self {
            AppError::NotFound(_) => (1, msg, false),
            AppError::BadRequest(_) => (2, msg, false),
            AppError::FileAlreadyExists(_) => (3, msg, false),
            AppError::InvalidTask(_) => (4, msg, false),
            AppError::UserAlreadyExists(_) => (5, msg, false),
            AppError::UserNameInvalid(_) => (6, msg, false),
            AppError::ErrorUserNameOrPassword(_) => (7, msg, false),
            AppError::Unauthorized => (8, msg, false),
            AppError::Expired => (9, msg, false),
            AppError::TransportError(_) => (10, msg, false),
            AppError::Unknown(_) => (10000, msg, false),
        };

        (
            StatusCode::INTERNAL_SERVER_ERROR,
            create_response(success, msg.as_str(), code, None),
        )
            .into_response()
    }
}

fn create_response(ok: bool, msg: &str, code: isize, data: Option<Value>) -> Json<Value> {
    Json(json!({
        "success": ok,
        "message": msg,
        "code": code,
        "data": data,
    }))
}

#[named]
fn into_host_path_safely(base_path: &std::path::Path, path: &String) -> Result<PathBuf, AppError> {
    let create_err = || AppError::NotFound(path.clone());
    let path = percent_decode(path.as_bytes()).decode_utf8().unwrap();
    let path = std::path::PathBuf::from(path.as_ref());
    let Ok(full_path) = base_path.join(path.as_path()).normalize_lexically() else {
        return Err(create_err());
    };
    if !full_path.starts_with(base_path.normalize_lexically().unwrap()) {
        return Err(create_err());
    }

    Ok(full_path)
}

fn should_use_secure_cookie(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.eq_ignore_ascii_case("https"))
        .or_else(|| {
            headers
                .get("origin")
                .and_then(|value| value.to_str().ok())
                .map(|value| value.starts_with("https://"))
        })
        .or_else(|| {
            headers
                .get("referer")
                .and_then(|value| value.to_str().ok())
                .map(|value| value.starts_with("https://"))
        })
        .unwrap_or(false)
}

#[named]
async fn api_get_file_list(
    token: Token,
    path: Option<Path<String>>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    println!("{} called: {:?}", function_name!(), path);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::Unauthorized);
    };
    println!("{:?}", root_path);
    let path = path.unwrap_or(Path(String::new()));
    let full_path = into_host_path_safely(root_path.as_path(), &path)?;
    if !full_path.exists() {
        return Err(AppError::NotFound(path.clone()));
    }
    if !full_path.is_dir() {
        return Ok(create_response(true, "不是文件夹", 0, None));
    }
    let mut entries = tokio::fs::read_dir(&full_path)
        .await
        .map_err(|_| AppError::NotFound(path.0.clone()))?;

    let mut files = Vec::new();
    let mut directories = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|_| AppError::NotFound(path.0.clone()))?
    {
        let metadata = entry
            .metadata()
            .await
            .map_err(|_| AppError::NotFound(path.0.clone()))?;
        let name = entry.file_name().to_string_lossy().to_string();
        // 跳过隐藏文件（以 . 开头）
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

    let response_data = json!({
        "dirs": directories,
        "files": files,
    });
    Ok(create_response(true, "success", 0, Some(response_data)))
}

async fn send_file(full_path: PathBuf, disposition: bool) -> Result<Response<Body>, AppError> {
    println!("send_file {}", full_path.display());
    if !full_path.exists() {
        return Err(AppError::Unauthorized);
    }
    if full_path.is_dir() {
        return Err(AppError::NotFound(full_path.display().to_string()));
    }

    let file = match tokio::fs::File::open(&full_path).await {
        Ok(file) => file,
        Err(e) => {
            eprintln!("无法打开文件 {:?}: {}", full_path, e);
            return Err(AppError::NotFound(full_path.display().to_string()));
        }
    };
    let metadata = tokio::fs::metadata(&full_path)
        .await
        .map_err(|e| AppError::NotFound(full_path.display().to_string()))?;

    let file_name = full_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    let mime_type = mime_guess::from_path(&full_path).first_or_octet_stream();
    let mut headers = HeaderMap::new();
    headers.typed_insert(ContentType::from(mime_type));

    if disposition {
        let content_disposition = format!("attachment; filename=\"{}\"", file_name);
        headers.insert("Content-Disposition", content_disposition.parse().unwrap());
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

#[named]
async fn api_download_file(
    token: Token,
    path: Option<Path<String>>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Response<Body>, AppError> {
    println!("{} called: {:?}", function_name!(), path);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let full_path =
        into_host_path_safely(root_path.as_path(), &path.unwrap_or(Path(String::new())))?;
    send_file(full_path, false).await
}

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
    // directories are pushed after their children so we can remove depth-first
    collected.push(path.to_owned());
    (count, bytes)
}

struct DeleteFiles {
    base: TaskBase,
    paths: Vec<PathBuf>,
    cancel: AtomicBool,
}

impl DeleteFiles {
    fn new(name: String, owner: String, paths: Vec<PathBuf>) -> Self {
        Self {
            base: TaskBase::new(name, owner),
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
            base.set_cur_task(
                entry
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into(),
            )
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

#[derive(Debug, Deserialize)]
struct DeleteFilesParam {
    paths: Vec<String>,
}

#[named]
async fn api_delete_files(
    token: Token,
    cfg: Extension<Arc<AppState>>,
    payload: Json<DeleteFilesParam>,
) -> Result<Json<Value>, AppError> {
    println!("{} called: {:?}", function_name!(), payload.paths);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let param = payload.0;
    if param.paths.is_empty() {
        return Err(AppError::BadRequest("没有要删除的路径".to_owned()));
    }
    let mut resolved = Vec::new();
    for p in &param.paths {
        let full = into_host_path_safely(root_path.as_path(), p)?;
        if !full.exists() {
            return Err(AppError::NotFound(p.clone()));
        }
        resolved.push(full);
    }
    let task_name = if param.paths.len() == 1 {
        format!("删除 {}", param.paths[0])
    } else {
        format!("删除 {} 项", param.paths.len())
    };
    let task = Arc::new(DeleteFiles::new(task_name, username, resolved));
    cfg.task_mgr.add_task(task.clone()).await;
    Ok(create_response(
        true,
        "成功创建删除任务",
        0,
        Some(json!({
            "task_id": task.uuid().to_string(),
        })),
    ))
}

pub async fn move_file_or_dir(
    source: impl AsRef<std::path::Path>,
    destination: impl AsRef<std::path::Path>,
) -> Result<(), tokio::io::Error> {
    let source = source.as_ref();
    let destination = destination.as_ref();
    if let Some(parent) = destination.parent() {
        if !parent.exists() {
            tokio::fs::create_dir_all(parent).await?;
        }
    }
    tokio::fs::rename(source, destination).await
}

#[derive(Debug, Deserialize)]
struct RenameParams {
    new_path: String,
}
#[named]
async fn api_rename_file(
    token: Token,
    path: Option<Path<String>>,
    query: Query<RenameParams>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    println!("{} called: {:?}, {:?}", function_name!(), path, query);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let path = path.unwrap_or(Path(String::new())).0;
    let request_new_path = percent_decode_str(&query.new_path)
        .decode_utf8()
        .map_err(|_| AppError::BadRequest("new_path 编码无效".to_string()))?
        .to_string();
    let Ok(old_path) = into_host_path_safely(root_path.as_path(), &path) else {
        return Err(AppError::BadRequest("old_path".to_owned()));
    };
    let Ok(new_path) = into_host_path_safely(root_path.as_path(), &request_new_path) else {
        return Err(AppError::BadRequest(format!(
            "new_path={}",
            request_new_path
        )));
    };
    if !old_path.exists() {
        return Err(AppError::NotFound(path));
    }
    if old_path == new_path {
        return Ok(create_response(true, "重命名成功", 0, None));
    }
    if new_path.exists() {
        return Err(AppError::FileAlreadyExists(request_new_path));
    }
    move_file_or_dir(old_path.as_path(), new_path.as_path())
        .await
        .map_err(|e| AppError::Unknown(format!("{}", e)))?;

    Ok(create_response(true, "成功移动目录", 0, None))
}

#[named]
async fn web_file(
    path: Option<Path<String>>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Response<Body>, AppError> {
    println!("{} called: {:?}", function_name!(), path);
    let full_path = into_host_path_safely(
        cfg.resources_path.as_path(),
        &path.unwrap_or(Path(String::new())),
    )?;
    send_file(full_path, false).await
}

#[named]
async fn api_get_task_status(
    token: Token,
    Path(uuid): Path<Uuid>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let task = cfg.task_mgr.get_task_for_user(&uuid, &username).await;
    let Some(task) = task else {
        return Err(AppError::InvalidTask(uuid));
    };
    let status = task.status().await;
    Ok(create_response(true, "success", 0, Some(status)))
}

#[named]
async fn api_get_task_list(
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let status_map = cfg.task_mgr.get_task_status_map_for_user(&username).await;
    Ok(create_response(true, "success", 0, Some(status_map.into())))
}

#[named]
async fn api_clear_finished_tasks(
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    println!("{} called:", function_name!());
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let count = cfg.task_mgr.clear_finished_for_user(&username).await;
    Ok(create_response(
        true,
        "success",
        0,
        Some(json!({
            "cleared": count,
        })),
    ))
}

#[named]
async fn api_remove_tasks(
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<CopyFilesParam>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!({})))
}

#[derive(Debug, Deserialize, Serialize)]
struct CopyFilesParam {
    sources: Vec<String>,
    destinations: Vec<String>,
}

#[named]
async fn api_copy_files(
    token: Token,
    cfg: Extension<Arc<AppState>>,
    payload: Json<CopyFilesParam>,
) -> Result<Json<Value>, AppError> {
    println!("{} called: {:?}", function_name!(), payload);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let param = payload.0;
    if param.sources.len() != param.destinations.len() {
        return Err(AppError::BadRequest("源路径和目标路径个数不同".to_owned()));
    }
    let (mut src_vec, mut dst_vec) = (Vec::new(), Vec::new());
    let task_name = format!(
        "复制到 {}",
        param.destinations.first().cloned().unwrap_or_default()
    );
    for (src, dst) in param.sources.iter().zip(param.destinations) {
        let src = into_host_path_safely(root_path.as_path(), src)?;
        let dst = into_host_path_safely(root_path.as_path(), &dst)?;
        src_vec.push(src);
        dst_vec.push(dst);
    }
    let task = Arc::new(CopyFiles::new(task_name, username, src_vec, dst_vec));
    cfg.task_mgr.add_task(task.clone()).await;
    Ok(create_response(
        true,
        "成功创建任务",
        0,
        Some(json!({
            "task_id": task.uuid().to_string(),
        })),
    ))
}

#[named]
async fn api_move_files(
    token: Token,
    cfg: Extension<Arc<AppState>>,
    payload: Json<CopyFilesParam>,
) -> Result<Json<Value>, AppError> {
    println!("{} called: {:?}", function_name!(), payload);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let param = payload.0;
    if param.sources.len() != param.destinations.len() {
        return Err(AppError::BadRequest("源路径和目标路径个数不同".to_owned()));
    }

    let (mut src_vec, mut dst_vec) = (Vec::new(), Vec::new());
    let task_name = format!(
        "移动到 {}",
        param.destinations.first().cloned().unwrap_or_default()
    );

    for (src, dst) in param.sources.iter().zip(param.destinations) {
        let src = into_host_path_safely(root_path.as_path(), src)?;
        let dst = into_host_path_safely(root_path.as_path(), &dst)?;
        src_vec.push(src);
        dst_vec.push(dst);
    }

    let task = Arc::new(MoveFiles::new(task_name, username, src_vec, dst_vec));
    cfg.task_mgr.add_task(task.clone()).await;
    Ok(create_response(
        true,
        "成功创建任务",
        0,
        Some(json!({
            "task_id": task.uuid().to_string(),
        })),
    ))
}

#[named]
async fn api_new_folder(
    token: Token,
    Path(path): Path<String>,
    cfg: Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    println!("{} called: {:?}", function_name!(), path);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let path = into_host_path_safely(root_path.as_path(), &path)?;
    if path.exists() {
        return Err(AppError::FileAlreadyExists(path.to_string_lossy().into()));
    }
    tokio::fs::create_dir_all(path)
        .await
        .map_err(|e| AppError::Unknown(format!("{:?}", e)))?;
    Ok(create_response(true, "创建成功", 0, None))
}

#[named]
fn normalize_upload_list(
    base_path: &std::path::Path,
    metas: &[UploadFileMeta],
) -> Vec<UploadFileMeta> {
    let mut ret = metas.to_vec();
    let mut path_set = metas.iter().map(|m| m.path.clone()).collect::<HashSet<_>>();
    let mut to_insert = Vec::new();
    let mut inserted = true;
    while inserted {
        inserted = false;
        for meta in metas {
            let path = PathBuf::from(&meta.path);
            if let Some(parent) = path.parent()
                && base_path != parent
            {
                let path_str: String = parent.to_string_lossy().into();
                if !path_set.contains(&path_str) {
                    path_set.insert(path_str);
                    to_insert.push(UploadFileMeta {
                        bytes: 0,
                        is_dir: true,
                        path: parent.to_string_lossy().into(),
                    });
                }
            }
        }
        inserted = !to_insert.is_empty();
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
    println!("{ret:?}");
    ret
}

#[named]
async fn api_upload(
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    metas: Json<UploadMetas>,
) -> Result<Json<Value>, AppError> {
    println!("{} called: {:?}", function_name!(), metas);
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::Unauthorized);
    };
    if metas.files.is_empty() {
        return Err(AppError::BadRequest("no files to upload".to_string()));
    }

    let dest_path = into_host_path_safely(root_path.as_path(), &metas.destination)?;
    let mut metas = metas.0;
    for meta in &mut metas.files {
        let absolute_path = dest_path.join(std::path::Path::new(&meta.path));
        meta.path = absolute_path.to_string_lossy().to_string();
    }
    metas.files = normalize_upload_list(&dest_path, &metas.files);
    let task_name = format!("上传文件到： {}", metas.destination);
    let task = Arc::new(UploadFiles::new(task_name, username, metas.files));
    cfg.task_mgr.add_task(task.clone()).await;

    Ok(create_response(
        true,
        "成功创建任务",
        0,
        Some(json!({
            "task_id": task.uuid().to_string(),
        })),
    ))
}
#[named]
async fn api_upload_feed(
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<Value>, AppError> {
    println!("{} called", function_name!());
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::Unauthorized);
    };
    let mut path = String::new();
    let mut task_id = String::new();
    let mut file_data = Bytes::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::BadRequest("multipart".to_string()))?
    {
        let field_name = field.name().unwrap_or("").to_string();

        match field_name.as_str() {
            "path" => {
                path = field
                    .text()
                    .await
                    .map_err(|_| AppError::BadRequest("path of multipart".to_string()))?;
            }
            "task_id" => {
                task_id = field
                    .text()
                    .await
                    .map_err(|_| AppError::BadRequest("task_id of multipart".to_string()))?;
            }
            "file" => {
                file_data = field
                    .bytes()
                    .await
                    .map_err(|_| AppError::BadRequest("file data".to_string()))?;
            }
            _ => {}
        }
    }
    let task_uuid = Uuid::from_str(task_id.as_str())
        .map_err(|_| AppError::BadRequest("request id".to_string()))?;
    let Some(task) = cfg.task_mgr.get_task_for_user(&task_uuid, &username).await else {
        return Err(AppError::BadRequest("task id invalid".to_string()));
    };
    let Some(task) = (&*task as &dyn Any).downcast_ref::<UploadFiles>() else {
        return Err(AppError::BadRequest(
            "task is not upload file task.".to_string(),
        ));
    };
    let path = into_host_path_safely(&root_path, &path)?;
    task.feed_bytes(path, file_data).await?;
    Ok(create_response(true, "success", 0, None))
}

#[derive(Debug, Deserialize)]
struct LoginPayload {
    username: String,
    passphrase: String,
    keep_login: bool,
}

#[named]
async fn api_login(
    headers: HeaderMap,
    cfg: Extension<Arc<AppState>>,
    payload: Json<LoginPayload>,
) -> impl IntoResponse {
    println!("{} called: {:?}", function_name!(), payload.username);
    type Result = std::result::Result<Json<Value>, AppError>;
    let LoginPayload {
        username,
        passphrase,
        keep_login,
    } = payload.0;
    let passphrase = sha512_string_base64(passphrase.as_bytes());
    if !cfg.user_manager.login(&username, &passphrase).await {
        return Result::Err(AppError::ErrorUserNameOrPassword(
            "用户名或密码错误".to_owned(),
        ))
        .into_response();
    }
    let session_info = SessionInfo {
        username: username.clone(),
    };
    let token = cfg.session_manager.add_session(session_info).await;
    let secure = should_use_secure_cookie(&headers);
    let cookie_value = if keep_login {
        format!(
            "auth_token={}; HttpOnly;{} SameSite=Strict; Path=/; Max-Age={}",
            token,
            if secure { " Secure;" } else { "" },
            7 * 24 * 60 * 60 // 7天
        )
    } else {
        format!(
            "auth_token={}; HttpOnly;{} SameSite=Strict; Path=/",
            token,
            if secure { " Secure;" } else { "" }
        )
    };
    let response_body = create_response(true, "登录成功", 0, None);
    let resp: Response = Response::builder()
        .status(StatusCode::OK)
        .header("Set-Cookie", HeaderValue::from_str(&cookie_value).unwrap())
        .body(response_body.into_response().into_body())
        .unwrap()
        .into();
    resp
}

#[tokio::main]
async fn main() {
    let app_state = Arc::new(AppState {
        data_path: std::path::PathBuf::from("./dist/data")
            .canonicalize()
            .unwrap(),
        resources_path: std::path::PathBuf::from("./dist/resources")
            .canonicalize()
            .unwrap(),
        task_mgr: Arc::new(TaskManager::default()),
        user_manager: Arc::new(UserManager::default()),
        session_manager: Arc::new(SessionManager::default()),
    });
    {
        let passphrase = sha512_string_base64("admin".as_bytes());
        app_state
            .user_manager
            .new_user("admin", passphrase.as_str())
            .await
            .unwrap();
        println!("username{}\n passphrase:{}", "admin", passphrase);
    }
    let index_html = "index.html".to_owned();
    let app = Router::new()
        .route("/api/list_files/{*path}", get(api_get_file_list))
        .route("/api/list_files/", get(api_get_file_list))
        .route("/api/download/{*path}", get(api_download_file))
        .route("/api/download/", get(api_download_file))
        .route("/api/delete", post(api_delete_files))
        .route("/api/rename/{*path}", get(api_rename_file))
        .route("/api/new_folder/{*path}", get(api_new_folder))
        .route("/api/copy_files", post(api_copy_files))
        .route("/api/move_files", post(api_move_files))
        .route("/api/get_task_status/{*path}", get(api_get_task_status))
        .route("/api/clear_finished_tasks", post(api_clear_finished_tasks))
        .route("/api/get_task_list/", get(api_get_task_list))
        .route("/api/upload", post(api_upload))
        .route(
            "/api/upload_feed",
            post(api_upload_feed).layer(DefaultBodyLimit::max(32 * 1024 * 1024)),
        )
        .route("/api/login", post(api_login))
        .route("/{*path}", get(web_file))
        .route("/", get(|e| web_file(Some(Path(index_html)), e)))
        .layer(Extension(app_state));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}
