use argon2::password_hash::PasswordHashString;
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use base64::Engine;
use rand::Rng;
use std::collections::HashMap;
use tokio::sync::Mutex;

use crate::AppError;

#[derive(Debug, Clone)]
pub struct UserInfo {
    passphrase_hash: PasswordHashString,
}

#[derive(Default)]
pub struct UserManager {
    users: Mutex<HashMap<String, UserInfo>>,
}

impl UserManager {
    pub async fn new_user(&self, username: &str, passphrase: &str) -> Result<(), AppError> {
        let mut users_locked = self.users.lock().await;
        if users_locked.contains_key(username) {
            return Err(AppError::UserAlreadyExists(username.to_string()));
        }
        if !username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_')
            || username.len() > 16
        {
            return Err(AppError::UserNameInvalid(username.to_string()));
        }
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();
        let Ok(pass_hash) = argon2.hash_password(passphrase.as_bytes(), &salt) else {
            return Err(AppError::BadRequest("Argon2 failed".to_owned()));
        };
        users_locked.insert(
            username.to_owned(),
            UserInfo {
                passphrase_hash: pass_hash.into(),
            },
        );
        Ok(())
    }

    pub async fn login(&self, username: &str, passphrase: &str) -> bool {
        let locked = self.users.lock().await;
        let Some(info) = locked.get(username) else {
            return false;
        };
        let argon2 = Argon2::default();
        argon2
            .verify_password(passphrase.as_bytes(), &info.passphrase_hash.password_hash())
            .is_ok()
    }
}

#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub username: String,
}

#[derive(Default)]
pub struct SessionManager {
    sessions: Mutex<HashMap<String, SessionInfo>>,
}

impl SessionManager {
    pub async fn get_session_info(&self, token: &str) -> Option<SessionInfo> {
        self.sessions.lock().await.get(token).cloned()
    }

    pub async fn add_session(&self, info: SessionInfo) -> String {
        let token = generate_secure_token();
        self.sessions.lock().await.insert(token.clone(), info);
        token
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
