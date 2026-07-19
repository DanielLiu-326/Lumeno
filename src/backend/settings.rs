use axum::extract::ConnectInfo;
use axum::extract::Multipart;
use axum::http::header;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::Mutex;
use uuid::Uuid;

use super::auth::Role;
use super::common::{AppError, AppState, RateLimiter, Token, create_response};
use super::i18n::{RequestLocale, SUPPORTED_LOCALES, normalize_locale, tr, trf};

pub struct SettingsChange {
    pub changed_keys: Vec<String>,
    pub settings: Value,
}

pub type SettingsChangeHandler = dyn Fn(&SettingsChange) + Send + Sync;

pub struct SettingsManager {
    settings: Mutex<Value>,
    persist_path: PathBuf,
    change_handlers: StdMutex<Vec<Arc<SettingsChangeHandler>>>,
}

impl SettingsManager {
    pub fn new(persist_path: PathBuf) -> Self {
        let settings = if persist_path.exists() {
            std::fs::read_to_string(&persist_path)
                .ok()
                .and_then(|data| serde_json::from_str::<Value>(&data).ok())
                .unwrap_or_else(|| json!({}))
        } else {
            json!({})
        };
        Self {
            settings: Mutex::new(settings),
            persist_path,
            change_handlers: StdMutex::new(Vec::new()),
        }
    }

    pub fn register_change_handler<F>(&self, handler: F)
    where
        F: Fn(&SettingsChange) + Send + Sync + 'static,
    {
        self.change_handlers.lock().unwrap().push(Arc::new(handler));
    }

    pub async fn get_all(&self) -> Value {
        self.settings.lock().await.clone()
    }

    pub async fn set_value(&self, key: &str, value: Value) {
        let mut locked = self.settings.lock().await;
        let mut changed = false;
        if let Some(obj) = locked.as_object_mut() {
            changed = obj.get(key) != Some(&value);
            obj.insert(key.to_owned(), value);
        }
        let settings = locked.clone();
        drop(locked);
        self.save_to_disk(&settings);
        if changed {
            self.notify_change(vec![key.to_owned()], settings);
        }
    }

    pub async fn set_many(&self, updates: Value) {
        let mut locked = self.settings.lock().await;
        let mut changed_keys = Vec::new();
        if let (Some(target), Some(source)) = (locked.as_object_mut(), updates.as_object()) {
            for (k, v) in source {
                if target.get(k) != Some(v) {
                    changed_keys.push(k.clone());
                }
                target.insert(k.clone(), v.clone());
            }
        }
        let settings = locked.clone();
        drop(locked);
        self.save_to_disk(&settings);
        if !changed_keys.is_empty() {
            self.notify_change(changed_keys, settings);
        }
    }

    fn notify_change(&self, changed_keys: Vec<String>, settings: Value) {
        let handlers = self.change_handlers.lock().unwrap().clone();
        let change = SettingsChange {
            changed_keys,
            settings,
        };
        for handler in handlers {
            handler(&change);
        }
    }

    // ── Typed getters ────────────────────────────────────────────────────────

    /// Days after which cache files are deleted. 0 = GC disabled (default).
    pub async fn cache_retention_days(&self) -> u64 {
        self.settings
            .lock()
            .await
            .get("cache_retention_days")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    }

    pub fn thumbnail_concurrency_from(settings: &Value) -> usize {
        settings
            .get("thumbnail_concurrency")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).max(1).min(32))
            .unwrap_or(3)
    }

    pub fn darktable_concurrency_from(settings: &Value) -> usize {
        settings
            .get("darktable_concurrency")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).max(1).min(32))
            .unwrap_or(1)
    }

    pub async fn ai_enabled(&self) -> bool {
        self.settings
            .lock()
            .await
            .get("ai_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    pub async fn ai_provider(&self) -> String {
        Self::ai_provider_from(&*self.settings.lock().await)
    }

    pub fn ai_provider_from(settings: &Value) -> String {
        match settings
            .get("ai_provider")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .unwrap_or("local")
        {
            "remote" => "remote".to_string(),
            _ => "local".to_string(),
        }
    }

    pub async fn ai_batch_size(&self) -> usize {
        Self::ai_batch_size_from(&*self.settings.lock().await)
    }

    pub fn ai_batch_size_from(settings: &Value) -> usize {
        settings
            .get("ai_batch_size")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).clamp(1, 256))
            .unwrap_or(32)
    }

    pub async fn ai_similarity_threshold(&self) -> f32 {
        Self::ai_similarity_threshold_from(&*self.settings.lock().await)
    }

    pub fn ai_similarity_threshold_from(settings: &Value) -> f32 {
        settings
            .get("ai_similarity_threshold")
            .and_then(|v| v.as_f64())
            .map(|n| (n as f32).clamp(0.0, 1.0))
            .unwrap_or(0.35)
    }

    pub async fn ai_remote_url(&self) -> Option<String> {
        Self::remote_ai_url_from(self, &*self.settings.lock().await)
    }

    pub fn ai_concurrency_from(settings: &Value) -> usize {
        settings
            .get("ai_concurrency")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).max(1).min(16))
            .unwrap_or(1)
    }

    pub fn remote_ai_url_from(&self, settings: &Value) -> Option<String> {
        settings
            .get("ai_remote_url")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    }

    pub async fn background_thumbnail_generation(&self) -> bool {
        Self::background_thumbnail_generation_from(&*self.settings.lock().await)
    }

    pub fn background_thumbnail_generation_from(settings: &Value) -> bool {
        settings
            .get("thumbnail_generation_mode")
            .and_then(|value| value.as_str())
            == Some("background")
    }

    pub async fn thumbnail_scan_interval(&self) -> std::time::Duration {
        Self::thumbnail_scan_interval_from(&*self.settings.lock().await)
    }

    pub fn thumbnail_scan_interval_from(settings: &Value) -> std::time::Duration {
        let hours = match settings
            .get("thumbnail_scan_interval")
            .and_then(|value| value.as_str())
        {
            Some("12h") => 12,
            Some("3d") => 3 * 24,
            Some("7d") => 7 * 24,
            _ => 24,
        };
        std::time::Duration::from_secs(hours * 60 * 60)
    }

    /// Whether public user registration is allowed (default true).
    pub async fn allow_registration(&self) -> bool {
        self.settings
            .lock()
            .await
            .get("allow_registration")
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }

    fn save_to_disk(&self, settings: &Value) {
        let path = self.persist_path.clone();
        let data = serde_json::to_string_pretty(settings).unwrap_or_default();
        let _ = tokio::task::spawn_blocking(move || {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, data);
        });
    }

    pub fn clamp_ai_settings(settings: &mut Value) {
        let Some(obj) = settings.as_object_mut() else {
            return;
        };
        if let Some(value) = obj.get_mut("ai_concurrency") {
            if let Some(n) = value.as_u64() {
                *value = json!((n as usize).clamp(1, 16));
            }
        }
        if let Some(value) = obj.get_mut("ai_batch_size") {
            if let Some(n) = value.as_u64() {
                *value = json!((n as usize).clamp(1, 256));
            }
        }
        if let Some(value) = obj.get_mut("ai_similarity_threshold") {
            if let Some(n) = value.as_f64() {
                *value = json!((n as f32).clamp(0.0, 1.0));
            }
        }
        if let Some(value) = obj.get_mut("ai_provider") {
            if let Some(raw) = value.as_str() {
                *value = json!(match raw.trim().to_ascii_lowercase().as_str() {
                    "remote" => "remote",
                    _ => "local",
                });
            }
        }
        if let Some(value) = obj.get_mut("ai_remote_url") {
            if let Some(raw) = value.as_str() {
                *value = json!(raw.trim());
            }
        }
        if let Some(value) = obj.get_mut("ai_fallback_interim") {
            if let Some(flag) = value.as_bool() {
                *value = json!(flag);
            } else {
                *value = json!(false);
            }
        }
    }
}

// ── API: Get settings ────────────────────────────────────────────────────────

pub async fn api_settings_get(
    locale: RequestLocale,
    _token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let settings = cfg.settings_manager.get_all().await;
    let message = tr(locale.as_str(), "response.ok");
    Ok(create_response(true, &message, 0, Some(settings)))
}

// ── API: Update settings (admin only) ────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpdateSettingsPayload {
    settings: Value,
}

pub async fn api_settings_update(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(payload): Json<UpdateSettingsPayload>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    if session.role != Role::Admin {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.admin_only_settings",
        )));
    }
    let mut settings = payload.settings;
    SettingsManager::clamp_ai_settings(&mut settings);
    if let Err(error) = super::ai::validate_ai_settings_update(&settings) {
        return Err(AppError::BadRequest(trf(
            locale.as_str(),
            "error.bad_request",
            &[("value", error)],
        )));
    }
    cfg.settings_manager.set_many(settings).await;
    let settings = cfg.settings_manager.get_all().await;
    let message = tr(locale.as_str(), "response.settings_saved");
    Ok(create_response(true, &message, 0, Some(settings)))
}

pub async fn api_thumbnail_scan_now(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    if session.role != Role::Admin {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.admin_only_settings",
        )));
    }
    super::album::queue_background_thumbnail_tasks(&cfg).await;
    Ok(create_response(
        true,
        &tr(locale.as_str(), "response.thumbnail_scan_started"),
        0,
        None,
    ))
}

#[derive(Debug, Deserialize)]
pub struct UpdatePreferencesPayload {
    locale: String,
}

pub async fn api_preferences_get(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let user_locale = cfg.user_manager.get_locale(&username).await;
    let message = tr(&user_locale, "response.ok");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "locale": user_locale,
            "supported_locales": SUPPORTED_LOCALES,
        })),
    ))
}

pub async fn api_preferences_update(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(payload): Json<UpdatePreferencesPayload>,
) -> Result<Json<Value>, AppError> {
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let next_locale = normalize_locale(&payload.locale);
    if !cfg.user_manager.set_locale(&username, &next_locale).await {
        return Err(AppError::unauthorized(locale.as_str()));
    }
    let message = tr(&next_locale, "response.preferences_saved");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "locale": next_locale,
            "supported_locales": SUPPORTED_LOCALES,
        })),
    ))
}

#[derive(Debug, Deserialize)]
pub struct UpdatePasswordPayload {
    current_password: String,
    new_password: String,
}

pub async fn api_password_update(
    locale: RequestLocale,
    token: Token,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Extension(rate_limiter): Extension<RateLimiter>,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(payload): Json<UpdatePasswordPayload>,
) -> Result<Json<Value>, AppError> {
    if !rate_limiter.check(addr.ip()).await {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.rate_limited",
        )));
    }
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let current_password = cfg.prehash_passphrase(&payload.current_password);
    let new_password = cfg.prehash_passphrase(&payload.new_password);

    cfg.user_manager
        .update_passphrase(&username, &current_password, &new_password, locale.as_str())
        .await?;

    let message = tr(locale.as_str(), "response.password_updated");
    Ok(create_response(true, &message, 0, None))
}

#[derive(Debug, Deserialize)]
pub struct UpdateMemberPayload {
    username: String,
    #[serde(default)]
    role: Option<Role>,
    #[serde(default)]
    locale: Option<String>,
}

pub async fn api_admin_users_get(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    if session.role != Role::Admin {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.admin_only_member_management",
        )));
    }

    let users = cfg.user_manager.list_users().await;
    let message = tr(locale.as_str(), "response.ok");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({ "users": users })),
    ))
}

pub async fn api_admin_users_update(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(payload): Json<UpdateMemberPayload>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    if session.role != Role::Admin {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.admin_only_member_management",
        )));
    }

    let target_username = payload.username.trim().to_owned();
    if !cfg.user_manager.user_exists(&target_username).await {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.member_not_found",
            &[("username", target_username.clone())],
        )));
    }

    if target_username == session.username
        && matches!(payload.role, Some(role) if role != Role::Admin)
    {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.cannot_change_own_role",
        )));
    }

    if matches!(payload.role, Some(Role::User))
        && cfg.user_manager.get_role(&target_username).await == Role::Admin
        && cfg.user_manager.admin_count().await <= 1
    {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.cannot_remove_last_admin",
        )));
    }

    let next_locale = payload.locale.as_deref().map(normalize_locale);
    let updated_user = cfg
        .user_manager
        .update_user(&target_username, payload.role, next_locale.as_deref())
        .await
        .ok_or_else(|| {
            AppError::NotFound(trf(
                locale.as_str(),
                "error.member_not_found",
                &[("username", target_username.clone())],
            ))
        })?;

    let message = tr(locale.as_str(), "response.member_updated");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({ "user": updated_user })),
    ))
}

// ── API: Delete member (admin only) ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DeleteMemberPayload {
    username: String,
}

pub async fn api_admin_users_delete(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(payload): Json<DeleteMemberPayload>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    if session.role != Role::Admin {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.admin_only_member_management",
        )));
    }

    let target = payload.username.trim().to_owned();
    if target == session.username {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.cannot_delete_self",
        )));
    }
    if !cfg.user_manager.user_exists(&target).await {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.member_not_found",
            &[("username", target.clone())],
        )));
    }
    if cfg.user_manager.get_role(&target).await == Role::Admin
        && cfg.user_manager.admin_count().await <= 1
    {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.cannot_remove_last_admin",
        )));
    }

    // Fetch avatar before deleting user
    let avatar = cfg.user_manager.get_avatar(&target).await;

    cfg.user_manager.delete_user(&target).await;
    cfg.session_manager.remove_sessions_for_user(&target).await;

    if let Err(error) = cfg.ai_service.store().purge_owner(&target) {
        eprintln!("failed to purge ai index for deleted user {target}: {error}");
    }

    // Delete avatar file if present
    if let Some(avatar) = avatar {
        let avatar_path = cfg.data_path.join(AVATAR_DIR).join(&avatar);
        let _ = tokio::fs::remove_file(&avatar_path).await;
    }

    let message = trf(
        locale.as_str(),
        "response.member_deleted",
        &[("username", target)],
    );
    Ok(create_response(true, &message, 0, None))
}

// ── API: Admin reset member password ─────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ResetPasswordPayload {
    username: String,
    new_password: String,
}

pub async fn api_admin_reset_password(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(payload): Json<ResetPasswordPayload>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    if session.role != Role::Admin {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.admin_only_member_management",
        )));
    }

    let target = payload.username.trim().to_owned();
    if !cfg.user_manager.user_exists(&target).await {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.member_not_found",
            &[("username", target.clone())],
        )));
    }

    let new_password = cfg.prehash_passphrase(&payload.new_password);
    cfg.user_manager
        .reset_passphrase(&target, &new_password, locale.as_str())
        .await?;

    // Invalidate all sessions for the target user
    cfg.session_manager.remove_sessions_for_user(&target).await;

    let message = trf(
        locale.as_str(),
        "response.member_password_reset",
        &[("username", target)],
    );
    Ok(create_response(true, &message, 0, None))
}

// ── Avatar constants ─────────────────────────────────────────────────────────

const AVATAR_SIZE: u32 = 256;
const AVATAR_DIR: &str = "config/avatars";

// ── API: Upload avatar ───────────────────────────────────────────────────────

pub async fn api_avatar_upload(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<Value>, AppError> {
    let Some(username) = cfg.get_username(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("{e}")))?
    else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.no_files_upload",
        )));
    };

    let data = field
        .bytes()
        .await
        .map_err(|e| AppError::BadRequest(format!("{e}")))?;

    if data.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.no_files_upload",
        )));
    }

    let avatars_dir = cfg.data_path.join(AVATAR_DIR);
    tokio::fs::create_dir_all(&avatars_dir)
        .await
        .map_err(|e| AppError::Unknown(format!("create avatar dir failed: {e}")))?;

    // Delete old avatar file if present
    if let Some(old_avatar) = cfg.user_manager.get_avatar(&username).await {
        let old_path = avatars_dir.join(&old_avatar);
        let _ = tokio::fs::remove_file(&old_path).await;
    }

    let filename = format!("{}.webp", Uuid::new_v4());
    let output_path = avatars_dir.join(&filename);

    let output_blocking = output_path.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let img = image::load_from_memory(&data).map_err(|e| format!("open image failed: {e}"))?;
        let thumb = img.thumbnail(AVATAR_SIZE, AVATAR_SIZE);
        thumb
            .save_with_format(&output_blocking, image::ImageFormat::WebP)
            .map_err(|e| format!("save webp avatar failed: {e}"))?;
        Ok(())
    })
    .await
    .map_err(|e| AppError::Unknown(format!("avatar task join failed: {e}")))?
    .map_err(AppError::Unknown)?;

    cfg.user_manager
        .set_avatar(&username, Some(&filename))
        .await;

    let message = tr(locale.as_str(), "response.avatar_updated");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({ "avatar": filename })),
    ))
}

// ── API: Serve avatar ────────────────────────────────────────────────────────

pub async fn api_avatar_get(
    locale: RequestLocale,
    _token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    axum::extract::Path(filename): axum::extract::Path<String>,
) -> Result<impl IntoResponse, AppError> {
    // Validate filename: must be a simple UUID.webp, no path traversal
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(AppError::BadRequest(trf(
            locale.as_str(),
            "error.bad_request",
            &[("value", filename.clone())],
        )));
    }

    let avatars_dir = cfg.data_path.join(AVATAR_DIR);
    let file_path = avatars_dir.join(&filename);

    if !file_path.exists() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", filename.clone())],
        )));
    }

    let data = tokio::fs::read(&file_path)
        .await
        .map_err(|e| AppError::Unknown(format!("read avatar failed: {e}")))?;

    Ok((
        [
            (header::CONTENT_TYPE, "image/webp"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        data,
    ))
}

// ── WebDAV share management ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebdavShare {
    pub username: String,
    pub path: String,
    pub readonly: bool,
    #[serde(default)]
    pub anonymous: bool,
}

fn webdav_shares_path(data_path: &std::path::Path) -> PathBuf {
    data_path.join("webdav_shares.json")
}

async fn load_webdav_shares(data_path: &std::path::Path) -> Vec<WebdavShare> {
    let path = webdav_shares_path(data_path);
    let Ok(raw) = tokio::fs::read_to_string(&path).await else {
        return vec![];
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub(crate) async fn webdav_share_count(data_path: &std::path::Path) -> usize {
    load_webdav_shares(data_path).await.len()
}

async fn save_webdav_shares(
    data_path: &std::path::Path,
    shares: &[WebdavShare],
) -> Result<(), AppError> {
    let json_path = webdav_shares_path(data_path);
    let content = serde_json::to_string_pretty(shares)
        .map_err(|e| AppError::Unknown(format!("serialize webdav shares: {e}")))?;
    tokio::fs::write(&json_path, content)
        .await
        .map_err(|e| AppError::Unknown(format!("write webdav shares json: {e}")))?;
    Ok(())
}

pub async fn api_webdav_shares_get(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let username = &session.username;

    let all_shares = load_webdav_shares(&cfg.data_path).await;
    let user_shares: Vec<&WebdavShare> = all_shares
        .iter()
        .filter(|s| s.username == *username)
        .collect();

    let message = tr(locale.as_str(), "response.ok");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "shares": user_shares,
        })),
    ))
}

#[derive(Debug, Deserialize)]
pub struct WebdavSharePayload {
    action: String,
    path: String,
    #[serde(default = "default_true")]
    readonly: bool,
    #[serde(default)]
    anonymous: bool,
}

fn default_true() -> bool {
    true
}

pub async fn api_webdav_shares_update(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(payload): Json<WebdavSharePayload>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let username = session.username.clone();

    let clean_path = payload.path.trim().trim_matches('/').to_string();
    if clean_path.is_empty() || clean_path.contains("..") {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_path",
        )));
    }
    let target = cfg.data_path.join(&username).join(&clean_path);
    if !target.is_dir() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_path",
        )));
    }

    let mut shares = load_webdav_shares(&cfg.data_path).await;

    match payload.action.as_str() {
        "add" => {
            shares.retain(|s| !(s.username == username && s.path == clean_path));
            shares.push(WebdavShare {
                username: username.clone(),
                path: clean_path,
                readonly: payload.readonly,
                anonymous: payload.anonymous,
            });
        }
        "remove" => {
            shares.retain(|s| !(s.username == username && s.path == clean_path));
        }
        _ => {
            return Err(AppError::BadRequest("invalid action".into()));
        }
    }

    save_webdav_shares(&cfg.data_path, &shares).await?;

    let user_shares: Vec<&WebdavShare> =
        shares.iter().filter(|s| s.username == *username).collect();

    let message = tr(locale.as_str(), "response.settings_saved");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({ "shares": user_shares })),
    ))
}

/// WebDAV request handler — serves `/dav/{username}/{path}` using dav-server
/// with HTTP Basic Auth. Each request is authenticated against the user store.
pub async fn api_webdav_handler(
    Extension(cfg): Extension<Arc<AppState>>,
    req: axum::extract::Request,
) -> axum::response::Response<dav_server::body::Body> {
    use axum::http::StatusCode;
    use base64::Engine;
    use dav_server::{DavHandler, fakels::FakeLs, localfs::LocalFs};

    let unauthorized = || {
        axum::http::Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .header("WWW-Authenticate", "Basic realm=\"Album WebDAV\"")
            .body(dav_server::body::Body::empty())
            .unwrap()
    };

    let not_found = || {
        axum::http::Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(dav_server::body::Body::empty())
            .unwrap()
    };

    // Extract Basic Auth credentials
    let auth_header = req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let credentials = if let Some(encoded) = auth_header.strip_prefix("Basic ") {
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|s| {
                let mut parts = s.splitn(2, ':');
                let user = parts.next()?.to_string();
                let pass = parts.next()?.to_string();
                Some((user, pass))
            })
    } else {
        None
    };

    // Parse /dav/{username}/{share_path}/... from URI
    let uri_path = req.uri().path().to_string();
    let stripped = uri_path.trim_start_matches("/dav/");
    let segments: Vec<&str> = stripped.splitn(2, '/').collect();
    let target_user = segments.first().unwrap_or(&"");
    if target_user.is_empty() {
        return not_found();
    }

    // Load shares & find the matching one
    let shares = load_webdav_shares(&cfg.data_path).await;
    let rest = segments.get(1).unwrap_or(&"");

    // Find the longest matching share path prefix
    let matched_share = shares
        .iter()
        .filter(|s| s.username == *target_user)
        .filter(|s| {
            rest.starts_with(&s.path)
                && (rest.len() == s.path.len() || rest.as_bytes().get(s.path.len()) == Some(&b'/'))
        })
        .max_by_key(|s| s.path.len());

    let Some(share) = matched_share else {
        return not_found();
    };

    // Authenticate: skip if share allows anonymous access
    if !share.anonymous {
        let authenticated = if let Some((ref user, ref pass)) = credentials {
            if user == *target_user {
                let prehashed = cfg.prehash_passphrase(pass);
                cfg.user_manager.login(user, &prehashed).await
            } else {
                false
            }
        } else {
            false
        };

        if !authenticated {
            return unauthorized();
        }
    }

    // Build the LocalFs root at the share's directory
    let fs_root = cfg.data_path.join(&share.username).join(&share.path);
    let is_readonly = share.readonly;

    let dav = DavHandler::builder()
        .filesystem(LocalFs::new(&fs_root, false, !is_readonly, false))
        .locksystem(FakeLs::new())
        .strip_prefix(&format!("/dav/{}/{}", share.username, share.path))
        .build_handler();

    dav.handle(req).await
}
