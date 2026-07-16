use axum::extract::FromRequestParts;
use axum::http::{HeaderMap, request::Parts};
use std::sync::Arc;

use super::auth::extract_token_from_cookie;
use super::common::{AppError, AppState};

pub const SUPPORTED_LOCALES: [&str; 2] = ["en", "zh"];

pub fn default_locale() -> String {
    "en".to_owned()
}

pub fn normalize_locale(input: &str) -> String {
    let value = input.trim().to_ascii_lowercase();
    if value.starts_with("zh") {
        "zh".to_owned()
    } else {
        "en".to_owned()
    }
}

pub fn tr(locale: &str, key: &str) -> String {
    let normalized = normalize_locale(locale);
    rust_i18n::t!(key, locale = normalized.as_str()).to_string()
}

pub fn trf(locale: &str, key: &str, params: &[(&str, String)]) -> String {
    let mut text = tr(locale, key);
    for (name, value) in params {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

pub fn locale_from_headers(headers: &HeaderMap) -> String {
    headers
        .get("accept-language")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(normalize_locale)
        .unwrap_or_else(default_locale)
}

pub async fn resolve_locale(state: &AppState, headers: &HeaderMap) -> String {
    if let Some(token) = extract_token_from_cookie(headers)
        && let Some(session) = state.session_manager.get_session_info(&token).await
    {
        return state.user_manager.get_locale(&session.username).await;
    }
    locale_from_headers(headers)
}

#[derive(Clone, Debug)]
pub struct RequestLocale(pub String);

impl RequestLocale {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl<S> FromRequestParts<S> for RequestLocale
where
    S: Send + Sync,
{
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        _state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        async {
            let locale = if let Some(state) = parts.extensions.get::<Arc<AppState>>() {
                resolve_locale(state, &parts.headers).await
            } else {
                locale_from_headers(&parts.headers)
            };
            Ok(Self(locale))
        }
    }
}