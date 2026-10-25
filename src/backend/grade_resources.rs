//! Album-level LUT and preset library. The id is a content hash. Names stay out of paths.

use std::sync::Arc;

use axum::Extension;
use axum::Json;
use axum::extract::{Multipart, Query};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::backend::common::{AppError, AppState, Token, create_response};
use crate::backend::i18n::{RequestLocale, tr};
use crate::grade::{
    GradeError, ResourceKind, delete_resource, import_lut, import_preset, list_resources,
    rename_resource,
};

#[derive(Deserialize)]
pub struct KindQuery {
    kind: String,
}

#[derive(Deserialize)]
pub struct RenameBody {
    kind: String,
    hash: String,
    display_name: String,
}

#[derive(Deserialize)]
pub struct DeleteBody {
    kind: String,
    hash: String,
}

pub async fn api_grade_resources_list(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Query(query): Query<KindQuery>,
) -> Result<Json<Value>, AppError> {
    let root = owner_root(&locale, &token, &cfg).await?;
    let kind = kind_or_reject(&locale, &query.kind)?;
    let locale_code = locale.as_str().to_string();
    let items = tokio::task::spawn_blocking(move || list_resources(&root, kind))
        .await
        .map_err(|error| AppError::Unknown(error.to_string()))?
        .map_err(|error| resource_error(&locale_code, error))?;
    Ok(create_response(
        true,
        "",
        0,
        Some(json!({ "items": items })),
    ))
}

pub async fn api_grade_resources_lut(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<Value>, AppError> {
    let root = owner_root(&locale, &token, &cfg).await?;
    let (name, bytes) = read_upload(&locale, &mut multipart).await?;
    let locale_code = locale.as_str().to_string();
    let record = tokio::task::spawn_blocking(move || import_lut(&root, &name, &bytes))
        .await
        .map_err(|error| AppError::Unknown(error.to_string()))?
        .map_err(|error| resource_error(&locale_code, error))?;
    Ok(create_response(true, "", 0, Some(json!(record))))
}

pub async fn api_grade_resources_preset(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<Value>, AppError> {
    let root = owner_root(&locale, &token, &cfg).await?;
    let (name, bytes) = read_upload(&locale, &mut multipart).await?;
    let locale_code = locale.as_str().to_string();
    let record = tokio::task::spawn_blocking(move || import_preset(&root, &name, &bytes))
        .await
        .map_err(|error| AppError::Unknown(error.to_string()))?
        .map_err(|error| resource_error(&locale_code, error))?;
    Ok(create_response(true, "", 0, Some(json!(record))))
}

pub async fn api_grade_resources_rename(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(body): Json<RenameBody>,
) -> Result<Json<Value>, AppError> {
    let root = owner_root(&locale, &token, &cfg).await?;
    let kind = kind_or_reject(&locale, &body.kind)?;
    let locale_code = locale.as_str().to_string();
    let hash = body.hash;
    let display_name = body.display_name;
    let record =
        tokio::task::spawn_blocking(move || rename_resource(&root, kind, &hash, &display_name))
            .await
            .map_err(|error| AppError::Unknown(error.to_string()))?
            .map_err(|error| resource_error(&locale_code, error))?;
    Ok(create_response(true, "", 0, Some(json!(record))))
}

pub async fn api_grade_resources_delete(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(body): Json<DeleteBody>,
) -> Result<Json<Value>, AppError> {
    let root = owner_root(&locale, &token, &cfg).await?;
    let kind = kind_or_reject(&locale, &body.kind)?;
    let locale_code = locale.as_str().to_string();
    let hash = body.hash;
    tokio::task::spawn_blocking(move || delete_resource(&root, kind, &hash))
        .await
        .map_err(|error| AppError::Unknown(error.to_string()))?
        .map_err(|error| resource_error(&locale_code, error))?;
    Ok(create_response(true, "", 0, None))
}

async fn owner_root(
    locale: &RequestLocale,
    token: &Token,
    cfg: &AppState,
) -> Result<std::path::PathBuf, AppError> {
    let Some((_, root)) = cfg.get_session_and_root(token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    Ok(root)
}

fn kind_or_reject(locale: &RequestLocale, kind: &str) -> Result<ResourceKind, AppError> {
    ResourceKind::parse(kind)
        .ok_or_else(|| resource_error(locale.as_str(), GradeError::Rejected("bad-id")))
}

async fn read_upload(
    locale: &RequestLocale,
    multipart: &mut Multipart,
) -> Result<(String, Vec<u8>), AppError> {
    let mut name = String::new();
    let mut bytes = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| AppError::BadRequest(error.to_string()))?
    {
        if field.name() != Some("file") {
            continue;
        }
        name = field
            .file_name()
            .unwrap_or("untitled")
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("untitled")
            .to_string();
        let data = field
            .bytes()
            .await
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
        bytes = Some(data.to_vec());
    }
    let Some(bytes) = bytes else {
        return Err(resource_error(
            locale.as_str(),
            GradeError::Rejected("cube-invalid"),
        ));
    };
    Ok((name, bytes))
}

fn resource_error(locale: &str, error: GradeError) -> AppError {
    let GradeError::Rejected(code) = error else {
        return AppError::Unknown(error.to_string());
    };
    let key = match code {
        "cube-size" => "error.grade_cube_size",
        "cube-edge" => "error.grade_cube_edge",
        "cube-invalid" => "error.grade_cube_invalid",
        "preset-size" => "error.grade_preset_size",
        "preset-invalid" => "error.grade_preset_invalid",
        "preset-lut" => "error.grade_preset_lut",
        "referenced" => "error.grade_resource_referenced",
        "missing" => "error.grade_resource_missing",
        "bad-id" => "error.grade_resource_bad_id",
        "io" => "error.grade_resource_io",
        _ => return AppError::BadRequest(code.to_string()),
    };
    if code == "missing" {
        return AppError::NotFound(tr(locale, key));
    }
    AppError::GradeRejected {
        message: tr(locale, key),
        reason: code,
    }
}
