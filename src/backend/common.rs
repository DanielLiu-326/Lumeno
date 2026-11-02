use axum::Json;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use serde_json::{Value, json};
use sha2::Sha512;
use std::collections::HashMap;
use std::net;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;
use tokio::sync::{Mutex, Notify};

use super::ai::AiService;
use super::auth::{SessionInfo, SessionManager, UserManager, extract_token_from_cookie};
use super::cache_areas::{self, CacheGc, CacheLayout};
use super::i18n::{resolve_locale, tr};
use super::settings::{SettingsChange, SettingsManager};
use super::tasks::TaskManager;

#[macro_export]
macro_rules! if_or {
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

type HmacSha512 = Hmac<Sha512>;

pub fn hmac_sha512_base64(key: &[u8], data: &[u8]) -> String {
    let mut mac = HmacSha512::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    let result = mac.finalize();
    base64::engine::general_purpose::STANDARD.encode(result.into_bytes())
}

pub struct Token(pub String);

impl AsRef<String> for Token {
    fn as_ref(&self) -> &String {
        &self.0
    }
}

impl Into<String> for Token {
    fn into(self) -> String {
        self.0
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

            let locale = resolve_locale(state, &parts.headers).await;
            let Some(token) = extract_token_from_cookie(&parts.headers) else {
                return Err(AppError::unauthorized(&locale));
            };
            if state
                .session_manager
                .get_session_info(&token)
                .await
                .is_none()
            {
                return Err(AppError::expired(&locale));
            }
            Ok(Token(token))
        }
    }
}

/// The request context every authenticated handler needs, resolved once.
///
/// `Token` already looks the session up to reject expired logins; carrying the
/// result here means handlers do not repeat `get_session_and_root` one to three
/// times, and the locale lookup happens once instead of per extractor.
pub struct Ctx {
    pub token: Token,
    pub session: SessionInfo,
    pub owner_root: PathBuf,
    pub locale: String,
}

impl Ctx {
    pub fn username(&self) -> &str {
        self.session.username.as_str()
    }

    pub fn is_admin(&self) -> bool {
        self.session.is_admin()
    }

    /// Envelope for a successful reply. `message_key` is a `locales/*.yml` key;
    /// the frontend only ever uses `data` plus `success`.
    pub fn ok(&self, message_key: &str, data: Option<Value>) -> Json<Value> {
        create_response(true, &tr(&self.locale, message_key), 0, data)
    }
}

/// The shape authenticated handlers return.
pub type ApiResult = Result<Json<Value>, AppError>;

impl<S> FromRequestParts<S> for Ctx
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
            let locale = resolve_locale(state, &parts.headers).await;
            let Some(token) = extract_token_from_cookie(&parts.headers) else {
                return Err(AppError::unauthorized(&locale));
            };
            let Some(session) = state.session_manager.get_session_info(&token).await else {
                return Err(AppError::expired(&locale));
            };
            let owner_root = state.data_path.join(&session.username);
            Ok(Ctx {
                token: Token(token),
                session,
                owner_root,
                locale,
            })
        }
    }
}

pub struct AppState {
    pub started_at: Instant,
    pub data_path: PathBuf,
    pub resources_path: PathBuf,
    pub egui_resources_path: PathBuf,
    pub hmac_key: Vec<u8>,
    pub task_mgr: Arc<TaskManager>,
    pub user_manager: Arc<UserManager>,
    pub session_manager: Arc<SessionManager>,
    pub settings_manager: Arc<SettingsManager>,
    pub trust_proxy_headers: bool,
    pub thumbnail_limiter: Arc<ConcurrencyLimiter>,
    pub darktable_limiter: Arc<ConcurrencyLimiter>,
    pub ai_limiter: Arc<ConcurrencyLimiter>,
    pub ai_service: Arc<AiService>,
}

struct ConcurrencyLimiterState {
    limit: usize,
    active: usize,
}

/// A resizable concurrency limiter. Lowering its limit does not interrupt
/// current work; it blocks new work until the active count drops below it.
pub struct ConcurrencyLimiter {
    state: std::sync::Mutex<ConcurrencyLimiterState>,
    available: Notify,
}

pub struct ConcurrencyPermit {
    limiter: Arc<ConcurrencyLimiter>,
    active: usize,
    limit: usize,
}

impl ConcurrencyLimiter {
    async fn new<F>(
        settings_manager: &Arc<SettingsManager>,
        setting_key: &'static str,
        read_limit: F,
    ) -> Arc<Self>
    where
        F: Fn(&serde_json::Value) -> usize + Send + Sync + 'static,
    {
        let settings = settings_manager.get_all().await;
        let limiter = Arc::new(Self {
            state: std::sync::Mutex::new(ConcurrencyLimiterState {
                limit: read_limit(&settings),
                active: 0,
            }),
            available: Notify::new(),
        });
        let weak_limiter = Arc::downgrade(&limiter);
        settings_manager.register_change_handler(move |change: &SettingsChange| {
            if change.changed_keys.iter().any(|key| key == setting_key) {
                if let Some(limiter) = weak_limiter.upgrade() {
                    limiter.set_limit(read_limit(&change.settings));
                }
            }
        });
        limiter
    }

    pub async fn for_thumbnails(settings_manager: &Arc<SettingsManager>) -> Arc<Self> {
        Self::new(
            settings_manager,
            "thumbnail_concurrency",
            SettingsManager::thumbnail_concurrency_from,
        )
        .await
    }

    pub async fn for_darktable(settings_manager: &Arc<SettingsManager>) -> Arc<Self> {
        Self::new(
            settings_manager,
            "darktable_concurrency",
            SettingsManager::darktable_concurrency_from,
        )
        .await
    }

    pub async fn for_ai(settings_manager: &Arc<SettingsManager>) -> Arc<Self> {
        Self::new(
            settings_manager,
            "ai_concurrency",
            SettingsManager::ai_concurrency_from,
        )
        .await
    }

    pub async fn acquire(self: &Arc<Self>) -> ConcurrencyPermit {
        loop {
            let notified = self.available.notified();
            {
                let mut state = self.state.lock().unwrap();
                if state.active < state.limit {
                    state.active += 1;
                    return ConcurrencyPermit {
                        limiter: self.clone(),
                        active: state.active,
                        limit: state.limit,
                    };
                }
            }
            notified.await;
        }
    }

    pub fn set_limit(&self, limit: usize) {
        self.state.lock().unwrap().limit = limit;
        self.available.notify_waiters();
    }

    pub fn limit(&self) -> usize {
        self.state.lock().unwrap().limit
    }
}

impl ConcurrencyPermit {
    pub fn active(&self) -> usize {
        self.active
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn limiter_id(&self) -> usize {
        Arc::as_ptr(&self.limiter) as usize
    }
}

impl Drop for ConcurrencyPermit {
    fn drop(&mut self) {
        self.limiter.state.lock().unwrap().active -= 1;
        self.limiter.available.notify_one();
    }
}

impl AppState {
    pub async fn get_session_info(&self, token: &Token) -> Option<SessionInfo> {
        self.session_manager
            .get_session_info(token.0.as_str())
            .await
    }

    pub async fn get_username(&self, token: &Token) -> Option<String> {
        self.get_session_info(token)
            .await
            .map(|session| session.username)
    }

    pub async fn get_root_path(&self, token: &Token) -> Option<PathBuf> {
        let Some(session) = self.get_session_info(token).await else {
            return None;
        };
        Some(self.data_path.join(session.username))
    }

    /// Resolve username and user root in a single session lock.
    pub async fn get_session_and_root(&self, token: &Token) -> Option<(String, PathBuf)> {
        let session = self.get_session_info(token).await?;
        let root = self.data_path.join(&session.username);
        Some((session.username, root))
    }

    /// Session lookup plus admin guard. Admin-only endpoints must answer the same
    /// way for "no session" and "not an admin", so they share this entry point.
    pub async fn require_admin(
        &self,
        token: &Token,
        locale: &str,
        message_key: &str,
    ) -> Result<SessionInfo, AppError> {
        let Some(session) = self.get_session_info(token).await else {
            return Err(AppError::unauthorized(locale));
        };
        session.require_admin(locale, message_key)?;
        Ok(session)
    }

    pub fn user_root_path(&self, username: &str) -> PathBuf {
        self.data_path.join(username)
    }

    pub fn prehash_passphrase(&self, passphrase: &str) -> String {
        hmac_sha512_base64(&self.hmac_key, passphrase.as_bytes())
    }
}

#[derive(Error, Debug)]
pub enum AppError {
    #[error("{0}")]
    NotFound(String),

    #[error("{0}")]
    FileAlreadyExists(String),

    #[error("{0}")]
    BadRequest(String),

    #[error("{0}")]
    InvalidTask(String),

    #[error("{0}")]
    UserAlreadyExists(String),

    #[error("{0}")]
    UserNameInvalid(String),

    #[error("{0}")]
    ErrorUserNameOrPassword(String),

    #[error("{0}")]
    Unauthorized(String),

    #[error("{0}")]
    Expired(String),

    #[error("{0}")]
    Forbidden(String),

    #[error("{0}")]
    TransportError(String),

    #[error("{message}")]
    GradeRejected {
        message: String,
        reason: &'static str,
    },

    #[error("{0}")]
    Unknown(String),
}

impl AppError {
    pub fn unauthorized(locale: &str) -> Self {
        Self::Unauthorized(tr(locale, "error.unauthorized"))
    }

    pub fn expired(locale: &str) -> Self {
        Self::Expired(tr(locale, "error.expired"))
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        if let AppError::GradeRejected { message, reason } = &self {
            return (
                StatusCode::BAD_REQUEST,
                create_response(false, message, 2, Some(json!({ "reason": reason }))),
            )
                .into_response();
        }
        let msg = format!("{}", &self);
        let (status, code, msg, success) = match self {
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, 1, msg, false),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, 2, msg, false),
            AppError::FileAlreadyExists(_) => (StatusCode::CONFLICT, 3, msg, false),
            AppError::InvalidTask(_) => (StatusCode::NOT_FOUND, 4, msg, false),
            AppError::UserAlreadyExists(_) => (StatusCode::CONFLICT, 5, msg, false),
            AppError::UserNameInvalid(_) => (StatusCode::BAD_REQUEST, 6, msg, false),
            AppError::ErrorUserNameOrPassword(_) => (StatusCode::UNAUTHORIZED, 7, msg, false),
            AppError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, 8, msg, false),
            AppError::Expired(_) => (StatusCode::UNAUTHORIZED, 9, msg, false),
            AppError::TransportError(_) => (StatusCode::BAD_REQUEST, 10, msg, false),
            AppError::Forbidden(_) => (StatusCode::FORBIDDEN, 11, msg, false),
            AppError::GradeRejected { .. } => (StatusCode::BAD_REQUEST, 2, msg, false),
            AppError::Unknown(_) => (StatusCode::INTERNAL_SERVER_ERROR, 10000, msg, false),
        };

        (status, create_response(success, msg.as_str(), code, None)).into_response()
    }
}

pub fn create_response(ok: bool, msg: &str, code: isize, data: Option<Value>) -> Json<Value> {
    Json(json!({
        "success": ok,
        "message": msg,
        "code": code,
        "data": data,
    }))
}

/// Join `path` under `base_path`, collapsing `.`/`..` lexically, refusing escapes,
/// and refusing anything whose resolved form leaves `base_path`.
///
/// `path` is already decoded: axum decodes `Path`/`Query` params, and JSON bodies
/// carry literal text. Decoding here would decode twice (breaking names with a
/// literal `%`) and could panic on malformed sequences.
///
/// A lexical check alone lets a symlink inside the root point outside it. This
/// resolves the deepest existing ancestor (so targets that do not exist yet —
/// upload destinations, new folders, rename targets — are covered too) and
/// requires the resolved path to stay under the resolved root. The returned path
/// is still the lexical one; callers open it, and the resolution above is what
/// keeps that open inside the root.
pub fn into_host_path_safely(base_path: &std::path::Path, path: &str) -> Result<PathBuf, AppError> {
    let create_err = || AppError::NotFound(path.to_owned());
    if path.contains('\0') {
        return Err(create_err());
    }
    let Ok(base) = base_path.normalize_lexically() else {
        return Err(create_err());
    };
    let Ok(full_path) = base.join(path).normalize_lexically() else {
        return Err(create_err());
    };
    if !full_path.starts_with(&base) {
        return Err(create_err());
    }
    if !resolves_inside(&base, &full_path) {
        return Err(create_err());
    }
    Ok(full_path)
}

/// Whether `path` really resolves under `base` once symlinks are followed.
///
/// Missing path components are walked up to the closest existing ancestor, so a
/// path that does not exist yet can still be confined. A resolution error other
/// than "not there" is treated as an escape: we cannot prove it is inside.
fn resolves_inside(base: &std::path::Path, path: &std::path::Path) -> bool {
    let Ok(base_real) = base.canonicalize() else {
        // The root itself is missing, so there is nothing inside it to follow.
        return true;
    };
    let mut probe = path;
    loop {
        match probe.canonicalize() {
            Ok(real) => return real.starts_with(&base_real),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
                ) =>
            {
                match probe.parent() {
                    Some(parent) if parent != probe => probe = parent,
                    _ => return false,
                }
            }
            Err(_) => return false,
        }
    }
}

/// Replace `path` with `data` atomically: write a sibling temp file, fsync it,
/// rename over the target, then fsync the directory.
///
/// A crash leaves either the previous file or the new one, never a truncated
/// JSON that the next start would parse as empty.
pub fn write_file_atomic(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let parent = path.parent().unwrap_or_else(|| std::path::Path::new("."));
    std::fs::create_dir_all(parent)?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let temp = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(data)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        // Best effort: makes the rename itself durable across a power loss.
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

pub fn should_use_secure_cookie(headers: &HeaderMap, trust_proxy: bool) -> bool {
    if trust_proxy {
        if let Some(proto) = headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
        {
            return proto.eq_ignore_ascii_case("https");
        }
    }
    // Fallback: check origin/referer (these are set by the browser, not the proxy)
    headers
        .get("origin")
        .and_then(|value| value.to_str().ok())
        .map(|value| value.starts_with("https://"))
        .or_else(|| {
            headers
                .get("referer")
                .and_then(|value| value.to_str().ok())
                .map(|value| value.starts_with("https://"))
        })
        .unwrap_or(false)
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

// ── Rate limiter ─────────────────────────────────────────────────────────────

const RATE_LIMIT_WINDOW_SECS: u64 = 60;
const RATE_LIMIT_MAX_ATTEMPTS: usize = 10;

#[derive(Clone)]
pub struct RateLimiter {
    attempts: Arc<Mutex<HashMap<net::IpAddr, Vec<Instant>>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            attempts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Whether this address already used up its attempts in the window.
    /// Does not record anything, so a successful request can be let through.
    pub async fn is_limited(&self, addr: net::IpAddr) -> bool {
        let mut map = self.attempts.lock().await;
        let now = Instant::now();
        let window = std::time::Duration::from_secs(RATE_LIMIT_WINDOW_SECS);
        // Prune stale entries for all IPs to prevent unbounded map growth.
        map.retain(|_, times| {
            times.retain(|t| now.duration_since(*t) < window);
            !times.is_empty()
        });
        map.get(&addr)
            .is_some_and(|entries| entries.len() >= RATE_LIMIT_MAX_ATTEMPTS)
    }

    /// Record one failed attempt. Callers record only failures, so a client that
    /// keeps authenticating correctly is never throttled.
    pub async fn record_failure(&self, addr: net::IpAddr) {
        let mut map = self.attempts.lock().await;
        map.entry(addr).or_default().push(Instant::now());
    }

    /// Record one attempt and answer the shared localized refusal once the
    /// window is exhausted. Every password endpoint goes through this.
    pub async fn check_for(&self, addr: net::IpAddr, locale: &str) -> Result<(), AppError> {
        if self.check(addr).await {
            return Ok(());
        }
        Err(AppError::BadRequest(tr(locale, "error.rate_limited")))
    }

    pub async fn check(&self, addr: net::IpAddr) -> bool {
        if self.is_limited(addr).await {
            return false;
        }
        self.record_failure(addr).await;
        true
    }
}

// ── Security headers middleware ──────────────────────────────────────────────

pub async fn security_headers(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    headers.insert(
        "Content-Security-Policy",
        HeaderValue::from_static(
            "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'",
        ),
    );
    headers.insert(
        "Referrer-Policy",
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        "Permissions-Policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );
    resp
}

// ── Cache GC ─────────────────────────────────────────────────────────────────

/// Background cache garbage collector.
/// Periodically scans cache subdirectories and removes files whose last
/// modification time is older than the configured retention period.
pub fn spawn_cache_gc(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(6 * 3600)).await;

            let retention_days = state.settings_manager.cache_retention_days().await;
            if retention_days == 0 {
                continue; // GC disabled
            }

            let cutoff = std::time::SystemTime::now()
                - std::time::Duration::from_secs(retention_days * 86400);
            let mut removed = 0u64;
            for area in cache_areas::CACHE_AREAS {
                if area.gc == CacheGc::Never {
                    continue;
                }
                removed += gc_cache_area(state.data_path.as_path(), area, cutoff).await;
            }

            if removed > 0 {
                eprintln!("Cache GC: removed {removed} file(s) older than {retention_days} day(s)");
            }
        }
    });
}

/// Walk one area according to its declared layout. Owner and encoder
/// directories are never deleted, only the files inside them.
async fn gc_cache_area(
    data_path: &std::path::Path,
    area: &cache_areas::CacheArea,
    cutoff: std::time::SystemTime,
) -> u64 {
    let root = area.root(data_path);
    match area.layout {
        CacheLayout::Flat => gc_cache_dir_files(&root, cutoff).await,
        CacheLayout::PerOwner => {
            let mut removed = 0u64;
            // Files directly in the area root come from the old flat layout
            // (e.g. `cache/darktable/*.jpg` before it moved per owner). They are
            // unreachable by the owner walk, so age them out here.
            removed += gc_cache_root_files(&root, cutoff).await;
            let Ok(mut owners) = tokio::fs::read_dir(&root).await else {
                return 0;
            };
            while let Ok(Some(owner)) = owners.next_entry().await {
                let Ok(meta) = owner.metadata().await else {
                    continue;
                };
                if !meta.is_dir() {
                    continue;
                }
                removed += gc_cache_dir_files(&owner.path(), cutoff).await;
            }
            removed
        }
        CacheLayout::PerOwnerPerKind => {
            let mut removed = 0u64;
            let Ok(mut owners) = tokio::fs::read_dir(&root).await else {
                return 0;
            };
            while let Ok(Some(owner)) = owners.next_entry().await {
                let Ok(meta) = owner.metadata().await else {
                    continue;
                };
                if !meta.is_dir() {
                    continue;
                }
                let Ok(mut kinds) = tokio::fs::read_dir(owner.path()).await else {
                    continue;
                };
                while let Ok(Some(kind)) = kinds.next_entry().await {
                    let Ok(meta) = kind.metadata().await else {
                        continue;
                    };
                    if meta.is_dir() {
                        removed += gc_cache_dir_files(&kind.path(), cutoff).await;
                    }
                }
            }
            removed
        }
    }
}

/// Age out files sitting directly in an area root, ignoring directories.
async fn gc_cache_root_files(dir: &std::path::Path, cutoff: std::time::SystemTime) -> u64 {
    let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
        return 0;
    };
    let mut removed = 0u64;
    while let Ok(Some(entry)) = rd.next_entry().await {
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let last_used = meta
            .accessed()
            .ok()
            .into_iter()
            .chain(meta.modified().ok())
            .max();
        if let Some(t) = last_used
            && t < cutoff
        {
            let _ = tokio::fs::remove_file(entry.path()).await;
            removed += 1;
        }
    }
    removed
}

async fn gc_cache_dir_files(dir: &std::path::Path, cutoff: std::time::SystemTime) -> u64 {
    let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
        return 0;
    };
    let mut removed = 0u64;
    while let Ok(Some(entry)) = rd.next_entry().await {
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let last_used = meta
            .accessed()
            .ok()
            .into_iter()
            .chain(meta.modified().ok())
            .max();
        if let Some(t) = last_used
            && t < cutoff
        {
            let _ = tokio::fs::remove_file(entry.path()).await;
            removed += 1;
        }
    }
    removed
}

/// Ensure `data/cache/*` exists (permissions follow host umask / ownership).
pub fn ensure_data_cache_layout(data_path: &Path) {
    let _ = std::fs::create_dir_all(cache_areas::cache_root(data_path));
    for area in cache_areas::CACHE_AREAS {
        let _ = std::fs::create_dir_all(area.root(data_path));
    }
}

#[cfg(test)]
mod cache_gc_tests {
    use super::{CacheGc, CacheLayout, cache_areas, gc_cache_area};

    fn temp_data(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("lumeno-gc-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_old(path: &std::path::Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"old").unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(400 * 86400);
        let file = std::fs::File::options().write(true).open(path).unwrap();
        file.set_modified(old).unwrap();
    }

    #[tokio::test]
    async fn gc_follows_the_declared_layout_and_keeps_gc_never_areas() {
        let data = temp_data("layout");
        // A future cutoff makes every file eligible, so this test is about the
        // layout walk rather than about access times.
        let cutoff = std::time::SystemTime::now() + std::time::Duration::from_secs(86400);

        // Flat area: only files, never the root itself.
        let exports = cache_areas::area("exports").unwrap();
        write_old(&exports.root(&data).join("a.zip"));
        // Per-owner area: files one level in.
        let darktable = cache_areas::area("darktable").unwrap();
        write_old(&darktable.owner_dir(&data, "daniel").join("preview.jpg"));
        // Per-owner area: files one level in.
        let thumbnails = cache_areas::area("thumbnails").unwrap();
        write_old(&thumbnails.owner_dir(&data, "daniel").join("b.webp"));
        // Per-owner-per-kind area: files two levels in.
        let video = cache_areas::area("video-encode").unwrap();
        write_old(&video.owner_dir(&data, "daniel").join("h264").join("c.mp4"));
        // Never area: untouched even when old.
        let ai = cache_areas::area("ai").unwrap();
        assert_eq!(ai.gc, CacheGc::Never);
        write_old(&ai.owner_dir(&data, "daniel").join("emb.json"));

        for area in [exports, darktable, thumbnails, video] {
            assert_eq!(area.gc, CacheGc::FilesOlderThan);
            let removed = gc_cache_area(&data, area, cutoff).await;
            assert_eq!(removed, 1, "{}", area.name);
        }
        assert!(!exports.root(&data).join("a.zip").exists());
        assert!(
            !darktable
                .owner_dir(&data, "daniel")
                .join("preview.jpg")
                .exists()
        );
        assert!(
            !thumbnails
                .owner_dir(&data, "daniel")
                .join("b.webp")
                .exists()
        );
        assert!(
            !video
                .owner_dir(&data, "daniel")
                .join("h264")
                .join("c.mp4")
                .exists()
        );
        // Directories stay so the next write does not have to recreate them.
        assert!(thumbnails.owner_dir(&data, "daniel").is_dir());
        assert!(darktable.owner_dir(&data, "daniel").is_dir());
        assert!(video.owner_dir(&data, "daniel").join("h264").is_dir());
        // The never area keeps its file.
        assert!(ai.owner_dir(&data, "daniel").join("emb.json").is_file());

        let _ = std::fs::remove_dir_all(&data);
    }

    #[tokio::test]
    async fn fresh_files_survive_the_retention_window() {
        let data = temp_data("fresh");
        let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(30 * 86400);
        let area = cache_areas::area("exports").unwrap();
        std::fs::create_dir_all(area.root(&data)).unwrap();
        std::fs::write(area.root(&data).join("now.zip"), b"new").unwrap();
        assert_eq!(gc_cache_area(&data, area, cutoff).await, 0);
        assert!(area.root(&data).join("now.zip").is_file());
        let _ = std::fs::remove_dir_all(&data);
    }
}

#[cfg(test)]
mod path_tests {
    use super::into_host_path_safely;
    use std::path::{Path, PathBuf};

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lumeno-path-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn keeps_literal_percent_in_names() {
        let base = temp_root("percent");
        let path = into_host_path_safely(&base, "albums/100%.jpg").unwrap();
        assert_eq!(path, base.join("albums/100%.jpg"));
        // A decoded-once value is not decoded again.
        let path = into_host_path_safely(&base, "albums/a%20b.jpg").unwrap();
        assert_eq!(path, base.join("albums/a%20b.jpg"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rejects_escapes_and_nul() {
        let base = temp_root("escape");
        assert!(into_host_path_safely(&base, "../bob/secret.jpg").is_err());
        assert!(into_host_path_safely(&base, "albums/../../bob").is_err());
        assert!(into_host_path_safely(&base, "/etc/passwd").is_err());
        assert!(into_host_path_safely(&base, "albums/a\0b.jpg").is_err());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn collapses_inner_parent_that_stays_inside() {
        let base = temp_root("collapse");
        let path = into_host_path_safely(&base, "albums/x/../y.jpg").unwrap();
        assert_eq!(path, base.join("albums/y.jpg"));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinks_that_leave_the_root() {
        let base = temp_root("root");
        let outside = temp_root("outside");
        std::fs::write(outside.join("secret.jpg"), b"secret").unwrap();

        // A symlinked directory pointing out of the root.
        std::os::unix::fs::symlink(&outside, base.join("linked")).unwrap();
        assert!(
            into_host_path_safely(&base, "linked/secret.jpg").is_err(),
            "a symlinked directory must not escape the root"
        );

        // A symlinked file pointing out of the root.
        std::os::unix::fs::symlink(outside.join("secret.jpg"), base.join("file.jpg")).unwrap();
        assert!(
            into_host_path_safely(&base, "file.jpg").is_err(),
            "a symlinked file must not escape the root"
        );

        // A not-yet-existing child under an escaping symlink is refused too.
        assert!(into_host_path_safely(&base, "linked/new/upload.jpg").is_err());

        // Symlinks that stay inside the root are fine.
        std::fs::create_dir_all(base.join("real")).unwrap();
        std::fs::write(base.join("real/a.jpg"), b"x").unwrap();
        std::os::unix::fs::symlink(base.join("real"), base.join("alias")).unwrap();
        assert!(into_host_path_safely(&base, "alias/a.jpg").is_ok());

        let _ = std::fs::remove_dir_all(&base);
        let _ = std::fs::remove_dir_all(&outside);
    }

    #[test]
    fn allows_paths_that_do_not_exist_yet() {
        let base = temp_root("new");
        std::fs::create_dir_all(base.join("albums")).unwrap();
        let path = into_host_path_safely(&base, "albums/new album/photo.jpg").unwrap();
        assert_eq!(path, base.join("albums/new album/photo.jpg"));
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod rate_limit_tests {
    use super::{RATE_LIMIT_MAX_ATTEMPTS, RateLimiter};

    #[tokio::test]
    async fn limits_after_repeated_failures_and_only_failures_count() {
        let limiter = RateLimiter::new();
        let addr: std::net::IpAddr = "10.0.0.7".parse().unwrap();

        // A successful request records nothing, so it is never throttled.
        for _ in 0..(RATE_LIMIT_MAX_ATTEMPTS * 3) {
            assert!(!limiter.is_limited(addr).await);
        }

        for _ in 0..RATE_LIMIT_MAX_ATTEMPTS {
            assert!(!limiter.is_limited(addr).await);
            limiter.record_failure(addr).await;
        }
        assert!(limiter.is_limited(addr).await);

        // The same address stays limited for the window; another one is unaffected.
        assert!(!limiter.is_limited("10.0.0.8".parse().unwrap()).await);
    }
}
