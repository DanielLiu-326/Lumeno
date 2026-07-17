use axum::Json;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use hmac::{Hmac, KeyInit, Mac};
use percent_encoding::percent_decode;
use serde_json::{Value, json};
use sha2::Sha512;
use std::collections::HashMap;
use std::net;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use thiserror::Error;
use tokio::sync::{Mutex, Notify};

use super::auth::{SessionInfo, SessionManager, UserManager, extract_token_from_cookie};
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

pub fn into_host_path_safely(base_path: &std::path::Path, path: &str) -> Result<PathBuf, AppError> {
    let create_err = || AppError::NotFound(path.to_owned());
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

    pub async fn check(&self, addr: net::IpAddr) -> bool {
        let mut map = self.attempts.lock().await;
        let now = Instant::now();
        let window = std::time::Duration::from_secs(RATE_LIMIT_WINDOW_SECS);
        // Prune stale entries for all IPs to prevent unbounded map growth.
        map.retain(|_, times| {
            times.retain(|t| now.duration_since(*t) < window);
            !times.is_empty()
        });
        let entries = map.entry(addr).or_default();
        if entries.len() >= RATE_LIMIT_MAX_ATTEMPTS {
            return false;
        }
        entries.push(now);
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
        let cache_dirs = ["thumbnails", "exif", "darktable", "exports"];
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(6 * 3600)).await;

            let retention_days = state.settings_manager.cache_retention_days().await;
            if retention_days == 0 {
                continue; // GC disabled
            }

            let cutoff = std::time::SystemTime::now()
                - std::time::Duration::from_secs(retention_days * 86400);
            let cache_root = state.data_path.join("cache");
            let mut removed = 0u64;

            for dir_name in &cache_dirs {
                let dir = cache_root.join(dir_name);
                let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
                    continue;
                };
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
                    if let Some(t) = last_used {
                        if t < cutoff {
                            let _ = tokio::fs::remove_file(entry.path()).await;
                            removed += 1;
                        }
                    }
                }
            }

            if removed > 0 {
                eprintln!("Cache GC: removed {removed} file(s) older than {retention_days} day(s)");
            }
        }
    });
}
