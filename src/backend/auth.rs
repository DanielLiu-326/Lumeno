use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::common::{
    AppError, AppState, RateLimiter, Token, create_response, should_use_secure_cookie,
};
use super::i18n::{RequestLocale, default_locale, normalize_locale, tr, trf};

const SESSION_TTL_LONG_DAYS: i64 = 7;
const SESSION_TTL_SHORT_HOURS: i64 = 24;

// ── Role enum ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    User,
}

impl Default for Role {
    fn default() -> Self {
        Role::User
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Role::Admin => write!(f, "admin"),
            Role::User => write!(f, "user"),
        }
    }
}

// ── User management ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    passphrase_hash: String,
    #[serde(default = "default_locale")]
    pub locale: String,
    pub role: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserSummary {
    pub username: String,
    pub locale: String,
    pub role: Role,
    pub avatar: Option<String>,
}

pub struct UserManager {
    users: Mutex<HashMap<String, UserInfo>>,
    persist_path: Option<PathBuf>,
}

impl UserManager {
    pub fn new(persist_path: Option<PathBuf>) -> Self {
        let mut users = HashMap::new();
        if let Some(ref path) = persist_path {
            if let Ok(data) = std::fs::read_to_string(path) {
                if let Ok(loaded) = serde_json::from_str::<HashMap<String, UserInfo>>(&data) {
                    users = loaded;
                }
            }
        }
        Self {
            users: Mutex::new(users),
            persist_path,
        }
    }

    pub async fn new_user(
        &self,
        username: &str,
        passphrase: &str,
        role: Role,
        preferred_locale: &str,
        request_locale: &str,
    ) -> Result<(), AppError> {
        if self.users.lock().await.contains_key(username) {
            return Err(AppError::UserAlreadyExists(trf(
                request_locale,
                "error.user_exists",
                &[("username", username.to_owned())],
            )));
        }
        if username.is_empty()
            || !username
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            || username.len() > 16
        {
            return Err(AppError::UserNameInvalid(trf(
                request_locale,
                "error.invalid_username",
                &[("username", username.to_owned())],
            )));
        }
        let passphrase = passphrase.to_owned();
        let passphrase_hash = tokio::task::spawn_blocking(move || {
            let salt = SaltString::generate(&mut OsRng);
            Argon2::default()
                .hash_password(passphrase.as_bytes(), &salt)
                .ok()
                .map(|hash| hash.serialize().to_string())
        })
        .await
        .map_err(|e| AppError::Unknown(format!("password hash task failed: {e}")))?;
        let Some(passphrase_hash) = passphrase_hash else {
            return Err(AppError::BadRequest(tr(
                request_locale,
                "error.argon2_failed",
            )));
        };

        let mut users_locked = self.users.lock().await;
        if users_locked.contains_key(username) {
            return Err(AppError::UserAlreadyExists(trf(
                request_locale,
                "error.user_exists",
                &[("username", username.to_owned())],
            )));
        }
        users_locked.insert(
            username.to_owned(),
            UserInfo {
                passphrase_hash,
                locale: normalize_locale(preferred_locale),
                role,
                avatar: None,
            },
        );
        self.save_to_disk(&users_locked);
        Ok(())
    }

    pub async fn login(&self, username: &str, passphrase: &str) -> bool {
        let Some(passphrase_hash) = self
            .users
            .lock()
            .await
            .get(username)
            .map(|info| info.passphrase_hash.clone())
        else {
            return false;
        };
        let passphrase = passphrase.to_owned();
        tokio::task::spawn_blocking(move || {
            let Ok(parsed) = argon2::PasswordHash::new(&passphrase_hash) else {
                return false;
            };
            Argon2::default()
                .verify_password(passphrase.as_bytes(), &parsed)
                .is_ok()
        })
        .await
        .unwrap_or(false)
    }

    pub async fn user_exists(&self, username: &str) -> bool {
        self.users.lock().await.contains_key(username)
    }

    pub async fn list_usernames(&self) -> Vec<String> {
        self.users.lock().await.keys().cloned().collect()
    }

    pub async fn list_users(&self) -> Vec<UserSummary> {
        let mut users = self
            .users
            .lock()
            .await
            .iter()
            .map(|(username, info)| UserSummary {
                username: username.clone(),
                locale: normalize_locale(&info.locale),
                role: info.role,
                avatar: info.avatar.clone(),
            })
            .collect::<Vec<_>>();
        users.sort_by(|left, right| left.username.cmp(&right.username));
        users
    }

    pub async fn admin_count(&self) -> usize {
        self.users
            .lock()
            .await
            .values()
            .filter(|user| user.role == Role::Admin)
            .count()
    }

    pub async fn update_user(
        &self,
        username: &str,
        role: Option<Role>,
        locale: Option<&str>,
    ) -> Option<UserSummary> {
        let mut users_locked = self.users.lock().await;
        let user = users_locked.get_mut(username)?;

        if let Some(role) = role {
            user.role = role;
        }
        if let Some(locale) = locale {
            user.locale = normalize_locale(locale);
        }

        let summary = UserSummary {
            username: username.to_owned(),
            locale: normalize_locale(&user.locale),
            role: user.role,
            avatar: user.avatar.clone(),
        };
        self.save_to_disk(&users_locked);
        Some(summary)
    }

    pub async fn get_locale(&self, username: &str) -> String {
        self.users
            .lock()
            .await
            .get(username)
            .map(|u| normalize_locale(&u.locale))
            .unwrap_or_else(default_locale)
    }

    pub async fn set_locale(&self, username: &str, locale: &str) -> bool {
        let mut users_locked = self.users.lock().await;
        let Some(user) = users_locked.get_mut(username) else {
            return false;
        };
        user.locale = normalize_locale(locale);
        self.save_to_disk(&users_locked);
        true
    }

    pub async fn get_role(&self, username: &str) -> Role {
        self.users
            .lock()
            .await
            .get(username)
            .map(|u| u.role)
            .unwrap_or_default()
    }

    pub async fn get_avatar(&self, username: &str) -> Option<String> {
        self.users
            .lock()
            .await
            .get(username)
            .and_then(|u| u.avatar.clone())
    }

    pub async fn set_avatar(&self, username: &str, avatar: Option<&str>) -> bool {
        let mut users_locked = self.users.lock().await;
        let Some(user) = users_locked.get_mut(username) else {
            return false;
        };
        user.avatar = avatar.map(|s| s.to_owned());
        self.save_to_disk(&users_locked);
        true
    }

    pub async fn update_passphrase(
        &self,
        username: &str,
        current_passphrase: &str,
        next_passphrase: &str,
        locale: &str,
    ) -> Result<(), AppError> {
        if current_passphrase.is_empty() || next_passphrase.is_empty() {
            return Err(AppError::BadRequest(tr(locale, "error.password_required")));
        }

        let Some(current_hash) = self
            .users
            .lock()
            .await
            .get(username)
            .map(|user| user.passphrase_hash.clone())
        else {
            return Err(AppError::unauthorized(locale));
        };

        let current_passphrase = current_passphrase.to_owned();
        let next_passphrase = next_passphrase.to_owned();
        let next_hash = tokio::task::spawn_blocking(move || {
            let Ok(parsed) = argon2::PasswordHash::new(&current_hash) else {
                return None;
            };
            let argon2 = Argon2::default();
            if argon2
                .verify_password(current_passphrase.as_bytes(), &parsed)
                .is_err()
            {
                return None;
            }
            let salt = SaltString::generate(&mut OsRng);
            argon2
                .hash_password(next_passphrase.as_bytes(), &salt)
                .ok()
                .map(|hash| hash.serialize().to_string())
        })
        .await
        .map_err(|e| AppError::Unknown(format!("password update task failed: {e}")))?;
        let Some(next_hash) = next_hash else {
            return Err(AppError::BadRequest(tr(
                locale,
                "error.invalid_current_password",
            )));
        };

        let mut users_locked = self.users.lock().await;
        let Some(user) = users_locked.get_mut(username) else {
            return Err(AppError::unauthorized(locale));
        };
        user.passphrase_hash = next_hash;
        self.save_to_disk(&users_locked);
        Ok(())
    }

    pub async fn is_empty(&self) -> bool {
        self.users.lock().await.is_empty()
    }

    pub async fn delete_user(&self, username: &str) -> bool {
        let mut users_locked = self.users.lock().await;
        let removed = users_locked.remove(username).is_some();
        if removed {
            self.save_to_disk(&users_locked);
        }
        removed
    }

    pub async fn reset_passphrase(
        &self,
        username: &str,
        new_passphrase: &str,
        locale: &str,
    ) -> Result<(), AppError> {
        if new_passphrase.is_empty() {
            return Err(AppError::BadRequest(tr(locale, "error.password_required")));
        }
        if !self.users.lock().await.contains_key(username) {
            return Err(AppError::NotFound(trf(
                locale,
                "error.member_not_found",
                &[("username", username.to_owned())],
            )));
        }
        let new_passphrase = new_passphrase.to_owned();
        let passphrase_hash = tokio::task::spawn_blocking(move || {
            let salt = SaltString::generate(&mut OsRng);
            Argon2::default()
                .hash_password(new_passphrase.as_bytes(), &salt)
                .ok()
                .map(|hash| hash.serialize().to_string())
        })
        .await
        .map_err(|e| AppError::Unknown(format!("password reset task failed: {e}")))?;
        let Some(passphrase_hash) = passphrase_hash else {
            return Err(AppError::BadRequest(tr(locale, "error.argon2_failed")));
        };

        let mut users_locked = self.users.lock().await;
        let Some(user) = users_locked.get_mut(username) else {
            return Err(AppError::NotFound(trf(
                locale,
                "error.member_not_found",
                &[("username", username.to_owned())],
            )));
        };
        user.passphrase_hash = passphrase_hash;
        self.save_to_disk(&users_locked);
        Ok(())
    }

    fn save_to_disk(&self, users: &HashMap<String, UserInfo>) {
        if let Some(ref path) = self.persist_path {
            let data = serde_json::to_string_pretty(users).unwrap_or_default();
            let path = path.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&path, data);
            });
        }
    }
}

// ── Session management ───────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionInfo {
    pub username: String,
    #[serde(default)]
    pub role: Role,
    #[serde(default = "default_expires_at")]
    pub expires_at: DateTime<Utc>,
}

fn default_expires_at() -> DateTime<Utc> {
    Utc::now() + Duration::days(SESSION_TTL_LONG_DAYS)
}

impl SessionInfo {
    pub fn new(username: String, role: Role, keep_login: bool) -> Self {
        let ttl = if keep_login {
            Duration::days(SESSION_TTL_LONG_DAYS)
        } else {
            Duration::hours(SESSION_TTL_SHORT_HOURS)
        };
        Self {
            username,
            role,
            expires_at: Utc::now() + ttl,
        }
    }
}

pub struct SessionManager {
    sessions: Mutex<HashMap<String, SessionInfo>>,
    persist_path: Option<PathBuf>,
}

impl SessionManager {
    pub fn new(persist_path: Option<PathBuf>) -> Self {
        let mut sessions = HashMap::new();
        if let Some(ref path) = persist_path {
            if let Ok(data) = std::fs::read_to_string(path) {
                if let Ok(map) = serde_json::from_str::<HashMap<String, serde_json::Value>>(&data) {
                    let now = Utc::now();
                    for (token, val) in map {
                        if let Ok(info) = serde_json::from_value::<SessionInfo>(val.clone()) {
                            if info.expires_at > now {
                                sessions.insert(token, info);
                            }
                        } else if let Some(username) = val.as_str() {
                            // Legacy format: { token: username }
                            sessions.insert(
                                token,
                                SessionInfo {
                                    username: username.to_owned(),
                                    role: Role::User,
                                    expires_at: default_expires_at(),
                                },
                            );
                        }
                    }
                }
            }
        }
        Self {
            sessions: Mutex::new(sessions),
            persist_path,
        }
    }

    pub async fn get_session_info(&self, token: &str) -> Option<SessionInfo> {
        let mut locked = self.sessions.lock().await;
        let info = locked.get(token)?;
        if info.expires_at <= Utc::now() {
            locked.remove(token);
            self.save_to_disk(&locked);
            return None;
        }
        Some(info.clone())
    }

    pub async fn add_session(&self, info: SessionInfo) -> String {
        let token = generate_secure_token();
        let mut locked = self.sessions.lock().await;
        locked.insert(token.clone(), info);
        self.save_to_disk(&locked);
        token
    }

    pub async fn remove_session(&self, token: &str) {
        let mut locked = self.sessions.lock().await;
        locked.remove(token);
        self.save_to_disk(&locked);
    }

    pub async fn remove_sessions_for_user(&self, username: &str) {
        let mut locked = self.sessions.lock().await;
        locked.retain(|_, info| info.username != username);
        self.save_to_disk(&locked);
    }

    fn save_to_disk(&self, sessions: &HashMap<String, SessionInfo>) {
        if let Some(ref path) = self.persist_path {
            let data = sessions
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        serde_json::json!({
                            "username": v.username,
                            "role": v.role,
                            "expires_at": v.expires_at,
                        }),
                    )
                })
                .collect::<std::collections::HashMap<String, serde_json::Value>>();
            let data = serde_json::to_string(&data).unwrap_or_default();
            let path = path.clone();
            let _ = tokio::task::spawn_blocking(move || {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::write(&path, data);
            });
        }
    }
}

pub fn extract_token_from_cookie(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("Cookie")
        .and_then(|header| header.to_str().ok())
        .and_then(|cookie_header| {
            cookie_header
                .split(';')
                .find(|part| part.trim().starts_with("auth_token="))
                .and_then(|part| part.split('=').nth(1))
                .map(|token| token.trim().to_string())
        })
}

fn generate_secure_token() -> String {
    let mut rng = rand::rng();
    let mut random_bytes = [0u8; 32];
    rng.fill(&mut random_bytes);
    base64::engine::general_purpose::STANDARD_NO_PAD.encode(random_bytes.as_ref())
}

// ── Auth API handlers ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LoginPayload {
    username: String,
    passphrase: String,
    keep_login: bool,
}

pub async fn api_login(
    locale: RequestLocale,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Extension(rate_limiter): Extension<RateLimiter>,
    cfg: Extension<Arc<AppState>>,
    payload: Json<LoginPayload>,
) -> impl IntoResponse {
    type Result = std::result::Result<Json<serde_json::Value>, AppError>;

    if !rate_limiter.check(addr.ip()).await {
        return Result::Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.rate_limited",
        )))
        .into_response();
    }

    let LoginPayload {
        username,
        passphrase,
        keep_login,
    } = payload.0;
    let passphrase = cfg.prehash_passphrase(&passphrase);
    let response_locale = cfg.user_manager.get_locale(&username).await;
    if !cfg.user_manager.login(&username, &passphrase).await {
        return Result::Err(AppError::ErrorUserNameOrPassword(tr(
            &response_locale,
            "error.invalid_credentials",
        )))
        .into_response();
    }
    let session_info = SessionInfo::new(
        username.clone(),
        cfg.user_manager.get_role(&username).await,
        keep_login,
    );
    let token = cfg.session_manager.add_session(session_info).await;
    let secure = should_use_secure_cookie(&headers, cfg.trust_proxy_headers);
    let cookie_value = if keep_login {
        format!(
            "auth_token={}; HttpOnly;{} SameSite=Strict; Path=/; Max-Age={}",
            token,
            if secure { " Secure;" } else { "" },
            7 * 24 * 60 * 60 // 7 days
        )
    } else {
        format!(
            "auth_token={}; HttpOnly;{} SameSite=Strict; Path=/",
            token,
            if secure { " Secure;" } else { "" }
        )
    };
    let success_locale = if response_locale.is_empty() {
        locale.as_str().to_owned()
    } else {
        response_locale
    };
    let success_message = tr(&success_locale, "response.login_success");
    let response_body = create_response(true, &success_message, 0, None);
    let resp: Response = Response::builder()
        .status(StatusCode::OK)
        .header("Set-Cookie", HeaderValue::from_str(&cookie_value).unwrap())
        .body(response_body.into_response().into_body())
        .unwrap()
        .into();
    resp
}

#[derive(Debug, Deserialize)]
pub struct RegisterPayload {
    username: String,
    passphrase: String,
}

pub async fn api_register(
    locale: RequestLocale,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Extension(rate_limiter): Extension<RateLimiter>,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<RegisterPayload>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !rate_limiter.check(addr.ip()).await {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.rate_limited",
        )));
    }
    let RegisterPayload {
        username,
        passphrase,
    } = payload.0;

    // Check if registration is allowed
    if !cfg.settings_manager.allow_registration().await {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.registration_disabled",
        )));
    }

    let passphrase = cfg.prehash_passphrase(&passphrase);
    cfg.user_manager
        .new_user(
            &username,
            &passphrase,
            Role::User,
            locale.as_str(),
            locale.as_str(),
        )
        .await?;

    // Create user data directory
    let user_dir = cfg.data_path.join(&username);
    let _ = tokio::fs::create_dir_all(&user_dir).await;

    let message = tr(locale.as_str(), "response.registration_success");
    Ok(create_response(true, &message, 0, None))
}

pub async fn api_whoami(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let user_locale = cfg.user_manager.get_locale(&session.username).await;
    let user_avatar = cfg.user_manager.get_avatar(&session.username).await;
    let message = tr(&user_locale, "response.ok");
    Ok(create_response(
        true,
        &message,
        0,
        Some(serde_json::json!({
            "username": session.username,
            "role": session.role,
            "locale": user_locale,
            "avatar": user_avatar,
        })),
    ))
}

pub async fn api_logout(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> impl IntoResponse {
    cfg.session_manager.remove_session(token.0.as_str()).await;
    let cookie_value = "auth_token=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0";
    let message = tr(locale.as_str(), "response.logged_out");
    Response::builder()
        .status(StatusCode::OK)
        .header("Set-Cookie", cookie_value)
        .body(
            create_response(true, &message, 0, None)
                .into_response()
                .into_body(),
        )
        .unwrap()
}
