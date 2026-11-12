use async_recursion::async_recursion;
use axum::extract::Query;
use axum::{Extension, Json};
use chrono::{Datelike, NaiveDate};
use function_name::named;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::u8;
use uuid::Uuid;

use crate::backend::common::{AppError, AppState, Token, create_response, into_host_path_safely};
use crate::backend::i18n::{RequestLocale, tr, trf};
use crate::backend::identity::{AlbumName, AlbumRef, Owner};
use crate::backend::tasks::{Task, TaskStatus};

use super::helpers::*;
use super::media::{is_album_media_path, is_image_path};
use super::types::*;

// ── Album API handlers ───────────────────────────────────────────────────────

#[named]
pub async fn api_albums_list(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let albums_dir = albums_root(root_path.as_path());
    tokio::fs::create_dir_all(&albums_dir)
        .await
        .map_err(|e| AppError::Unknown(format!("create albums dir failed: {e}")))?;

    let mut album_names = Vec::new();
    let mut entries = tokio::fs::read_dir(&albums_dir)
        .await
        .map_err(|e| AppError::Unknown(format!("read albums dir failed: {e}")))?;
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| AppError::Unknown(format!("read album entry failed: {e}")))?
    {
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if meta.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if is_user_visible_album_dir(&name) {
                album_names.push(name);
            }
        }
    }
    album_names.sort();

    let mut albums = Vec::new();
    for dir_name in &album_names {
        let album_dir = albums_dir.join(dir_name);
        let cache = ensure_album_cache_id(album_dir.as_path())
            .await
            .unwrap_or_default();
        let meta = read_album_meta(album_dir.as_path()).await;
        let display_name = album_display_name(&meta, dir_name);
        let index = load_or_queue_album_index(
            cfg.as_ref(),
            &username,
            root_path.as_path(),
            &username,
            dir_name,
            locale.as_str(),
        )
        .await
        .ok()
        .flatten();
        let photo_count = index.map(|i| i.entries.len()).unwrap_or(0);
        albums.push(json!({
            "dir_name": dir_name,
            "name": display_name,
            "cache": cache,
            "cover": meta.cover,
            "description": meta.description,
            "photo_count": photo_count,
            "shares": meta.shares,
        }));
    }

    let shared_albums = collect_shared_albums(&cfg, &username, locale.as_str()).await;

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "albums": albums,
            "shared_albums": shared_albums,
        })),
    ))
}

#[named]
pub async fn api_albums_create(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<CreateAlbumPayload>,
) -> Result<Json<Value>, AppError> {
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(name) = sanitize_album_name(&payload.name) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_album_name",
        )));
    };

    let album_dir = albums_root(root_path.as_path()).join(&name);
    tokio::fs::create_dir_all(&album_dir)
        .await
        .map_err(|e| AppError::Unknown(format!("create album dir failed: {e}")))?;
    let meta = AlbumMeta {
        name: name.clone(),
        cache: new_album_cache_id(),
        cover: String::new(),
        description: payload.description.clone().unwrap_or_default(),
        ..Default::default()
    };
    write_album_meta(album_dir.as_path(), &meta).await?;
    let message = tr(locale.as_str(), "response.album_created");
    Ok(create_response(true, &message, 0, None))
}

#[named]
pub async fn api_albums_update(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<UpdateAlbumPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let resolved = resolve_album_access(
        &cfg,
        &username,
        root_path.as_path(),
        &payload.dir_name,
        locale.as_str(),
    )
    .await?;
    if !resolved.can_write {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.no_write_permission_album",
        )));
    }
    let albums = albums_root(&resolved.root_path);
    let dir_name = resolved.album_name.clone();
    let mut album_dir = albums.join(&dir_name);
    if !album_dir.exists() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", payload.dir_name.clone())],
        )));
    }

    // Pin cache id before any directory rename so the index file stays put.
    let cache_id = ensure_album_cache_id(album_dir.as_path()).await?;
    let mut meta = read_album_meta(album_dir.as_path()).await;
    if meta.cache.is_empty() {
        meta.cache = cache_id.clone();
    }

    let mut effective_dir = dir_name.clone();

    // Changing the album name also renames the folder; cache pointer stays.
    if let Some(raw_name) = payload.name.as_ref() {
        if resolved.is_owner {
            let trimmed = raw_name.trim().to_string();
            let Some(new_dir) = sanitize_album_name(&trimmed) else {
                return Err(AppError::BadRequest(tr(
                    locale.as_str(),
                    "error.invalid_album_name",
                )));
            };
            meta.name = trimmed;
            if new_dir != dir_name {
                let dest = albums.join(&new_dir);
                if dest.exists() {
                    return Err(AppError::BadRequest(trf(
                        locale.as_str(),
                        "error.album_dir_exists",
                        &[("value", new_dir.clone())],
                    )));
                }
                tokio::fs::rename(&album_dir, &dest)
                    .await
                    .map_err(|e| AppError::Unknown(format!("rename album dir failed: {e}")))?;
                album_dir = dest;
                effective_dir = new_dir;
                let old_prefix = format!("albums/{dir_name}/");
                let new_prefix = format!("albums/{effective_dir}/");
                if let Some(rest) = meta.cover.strip_prefix(&old_prefix) {
                    meta.cover = format!("{new_prefix}{rest}");
                }
                let index_path =
                    album_index_path(cfg.data_path.as_path(), &resolved.owner, &meta.cache);
                if index_path.exists() {
                    if let Ok(raw) = tokio::fs::read_to_string(&index_path).await {
                        if let Ok(mut index) = serde_json::from_str::<AlbumIndex>(&raw) {
                            for entry in &mut index.entries {
                                if let Some(rest) = entry.path.strip_prefix(&old_prefix) {
                                    entry.path = format!("{new_prefix}{rest}");
                                }
                            }
                            if let Ok(data) = serde_json::to_string(&index) {
                                let _ = tokio::fs::write(&index_path, data).await;
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(cover) = &payload.cover {
        let Some(normalized) = normalize_album_cover_path(cover, &effective_dir, &dir_name) else {
            return Err(AppError::BadRequest(tr(
                locale.as_str(),
                "error.invalid_image_path",
            )));
        };
        if !normalized.is_empty() {
            let source = into_host_path_safely(resolved.root_path.as_path(), &normalized)?;
            if !source.is_file() || !is_image_path(source.as_path()) {
                return Err(AppError::NotFound(trf(
                    locale.as_str(),
                    "error.resource_not_found",
                    &[("value", normalized)],
                )));
            }
        }
        meta.cover = normalized;
    }
    if let Some(desc) = &payload.description {
        meta.description = desc.clone();
    }
    write_album_meta(album_dir.as_path(), &meta).await?;
    let message = tr(locale.as_str(), "response.album_updated");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "dir_name": album_access_key(&username, &resolved.owner, &effective_dir),
            "cache": meta.cache,
            "name": meta.name,
            "cover": meta.cover,
            "owner": resolved.owner,
        })),
    ))
}

#[named]
pub async fn api_albums_delete(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<DeleteAlbumPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(dir_name) = sanitize_album_name(&payload.dir_name) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_album_identifier",
        )));
    };
    let album_dir = albums_root(root_path.as_path()).join(&dir_name);
    if !album_dir.exists() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", dir_name.clone())],
        )));
    }
    let cache_id = ensure_album_cache_id(album_dir.as_path())
        .await
        .unwrap_or_else(|_| dir_name.clone());
    tokio::fs::remove_dir_all(&album_dir)
        .await
        .map_err(|e| AppError::Unknown(format!("delete album dir failed: {e}")))?;
    let index_path = album_index_path(cfg.data_path.as_path(), &username, &cache_id);
    let _ = tokio::fs::remove_file(&index_path).await;
    // Legacy indexes keyed by directory name before meta.cache existed.
    if cache_id != dir_name {
        let legacy = album_index_path(cfg.data_path.as_path(), &username, &dir_name);
        let _ = tokio::fs::remove_file(&legacy).await;
    }
    let message = tr(locale.as_str(), "response.album_deleted");
    Ok(create_response(true, &message, 0, None))
}

#[named]
pub async fn api_albums_import(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<ImportAlbumPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let ImportAlbumPayload {
        album,
        sources,
        move_files,
        conflict,
    } = payload.0;

    let Some(album_name) = sanitize_album_name(&album) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_album_name",
        )));
    };
    if sources.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.import_sources_empty",
        )));
    }

    for source in &sources {
        let _ = into_host_path_safely(root_path.as_path(), source)?;
    }

    // All sources must be from the same directory
    {
        let first_parent = std::path::Path::new(sources.first().unwrap())
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .to_string();
        for source in &sources[1..] {
            let parent = std::path::Path::new(source)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or("")
                .to_string();
            if parent != first_parent {
                return Err(AppError::BadRequest(tr(
                    locale.as_str(),
                    "error.import_sources_different_dirs",
                )));
            }
        }
    }

    let (task_id, _) = super::workflow::queue_import_workflow(
        cfg.clone(),
        username.clone(),
        root_path,
        username,
        album_name,
        sources,
        move_files.unwrap_or(false),
        conflict,
        locale.as_str().to_string(),
    )
    .await;

    let message = tr(locale.as_str(), "response.album_import_task_created");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({ "task_id": task_id.to_string() })),
    ))
}

// ── Date histogram ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DateHistogramQuery {
    album: String,
}

#[named]
pub async fn api_albums_date_histogram(
    locale: RequestLocale,
    token: Token,
    query: Query<DateHistogramQuery>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let resolved =
        resolve_album_access(&cfg, &username, &root_path, &query.album, locale.as_str()).await?;
    let album_dir = albums_root(resolved.root_path.as_path()).join(&resolved.album_name);

    if !album_dir.exists() {
        let message = tr(locale.as_str(), "response.success");
        return Ok(create_response(
            true,
            &message,
            0,
            Some(json!({ "buckets": [] })),
        ));
    }

    let Some(index) = load_or_queue_album_index(
        cfg.as_ref(),
        &username,
        resolved.root_path.as_path(),
        &resolved.owner,
        &resolved.album_name,
        locale.as_str(),
    )
    .await?
    else {
        let message = tr(locale.as_str(), "response.success");
        return Ok(create_response(
            true,
            &message,
            0,
            Some(json!({ "buckets": [] })),
        ));
    };

    // Aggregate counts by YYYY-MM-DD
    let mut day_counts: Vec<(String, usize)> = Vec::new();
    for entry in &index.entries {
        let date = if entry.date.len() >= 10 {
            &entry.date[..10]
        } else {
            &entry.date
        };
        if let Some(last) = day_counts.last_mut() {
            if last.0 == date {
                last.1 += 1;
                continue;
            }
        }
        day_counts.push((date.to_string(), 1));
    }

    // Index entries are sorted by date desc, so re-sort day_counts ascending
    day_counts.sort_by(|a, b| a.0.cmp(&b.0));

    let buckets: Vec<Value> = day_counts
        .iter()
        .map(|(d, c)| json!({ "date": d, "count": c }))
        .collect();

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({ "buckets": buckets })),
    ))
}

// ── Collections ───────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Default)]
pub(crate) struct CollectionFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    from: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    camera: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lens: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ratings: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    color_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CollectionNode {
    id: String,
    label: String,
    count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter: Option<CollectionFilter>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    children: Vec<CollectionNode>,
}

/// GET /api/albums/collections?album=<name> — Build a Darktable-style collection tree
/// from the album index. Each node carries a `filter` object that the frontend applies
/// directly to the photos query, so the backend needs no separate logic.
#[named]
pub async fn api_albums_collections(
    locale: RequestLocale,
    token: Token,
    query: Query<DateHistogramQuery>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let resolved =
        resolve_album_access(&cfg, &username, &root_path, &query.album, locale.as_str()).await?;
    let album_dir = albums_root(resolved.root_path.as_path()).join(&resolved.album_name);

    if !album_dir.exists() {
        let message = tr(locale.as_str(), "response.success");
        return Ok(create_response(
            true,
            &message,
            0,
            Some(json!({ "collections": [] })),
        ));
    }

    let Some(index) = load_or_queue_album_index(
        cfg.as_ref(),
        &username,
        resolved.root_path.as_path(),
        &resolved.owner,
        &resolved.album_name,
        locale.as_str(),
    )
    .await?
    else {
        let message = tr(locale.as_str(), "response.success");
        return Ok(create_response(
            true,
            &message,
            0,
            Some(json!({ "collections": [] })),
        ));
    };

    let total = index.entries.len();

    // ── All Photos ──
    let all_node = CollectionNode {
        id: "all".to_string(),
        label: "all".to_string(),
        count: total,
        filter: Some(CollectionFilter::default()),
        children: vec![],
    };

    // ── By Date: year → month ──
    let mut date_tree: std::collections::BTreeMap<i32, std::collections::BTreeMap<u32, usize>> =
        std::collections::BTreeMap::new();
    for entry in &index.entries {
        if entry.date.len() >= 7 {
            if let (Ok(year), Ok(month)) = (
                entry.date[..4].parse::<i32>(),
                entry.date[5..7].parse::<u32>(),
            ) {
                if (1..=12).contains(&month) {
                    *date_tree.entry(year).or_default().entry(month).or_default() += 1;
                }
            }
        }
    }
    let date_children: Vec<CollectionNode> = date_tree
        .iter()
        .rev()
        .map(|(&year, months)| {
            let year_count: usize = months.values().sum();
            let month_children: Vec<CollectionNode> = months
                .iter()
                .rev()
                .filter_map(|(&month, &count)| {
                    let first_day = format!("{:04}-{:02}-01", year, month);
                    let last_day = NaiveDate::from_ymd_opt(
                        if month == 12 { year + 1 } else { year },
                        if month == 12 { 1 } else { month + 1 },
                        1,
                    )?
                    .pred_opt()?
                    .format("%Y-%m-%d")
                    .to_string();
                    Some(CollectionNode {
                        id: format!("date:{:04}-{:02}", year, month),
                        label: format!("{:04}-{:02}", year, month),
                        count,
                        filter: Some(CollectionFilter {
                            from: Some(first_day),
                            to: Some(last_day),
                            ..Default::default()
                        }),
                        children: vec![],
                    })
                })
                .collect();
            CollectionNode {
                id: format!("date:{}", year),
                label: year.to_string(),
                count: year_count,
                filter: Some(CollectionFilter {
                    from: Some(format!("{:04}-01-01", year)),
                    to: Some(format!("{:04}-12-31", year)),
                    ..Default::default()
                }),
                children: month_children,
            }
        })
        .collect();
    let date_group = CollectionNode {
        id: "date".to_string(),
        label: "date".to_string(),
        count: total,
        filter: None,
        children: date_children,
    };

    // ── By Camera ──
    let mut camera_map: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for entry in &index.entries {
        let cam = format!("{} {}", entry.camera_make, entry.camera_model)
            .trim()
            .to_string();
        if !cam.is_empty() {
            *camera_map.entry(cam).or_default() += 1;
        }
    }
    let camera_children: Vec<CollectionNode> = camera_map
        .iter()
        .map(|(cam, &count)| CollectionNode {
            id: format!("camera:{}", cam),
            label: cam.clone(),
            count,
            filter: Some(CollectionFilter {
                camera: Some(cam.clone()),
                ..Default::default()
            }),
            children: vec![],
        })
        .collect();
    let camera_group = CollectionNode {
        id: "camera".to_string(),
        label: "camera".to_string(),
        count: camera_children.iter().map(|c| c.count).sum(),
        filter: None,
        children: camera_children,
    };

    // ── By Lens ──
    let mut lens_map: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for entry in &index.entries {
        if !entry.lens.is_empty() {
            *lens_map.entry(entry.lens.clone()).or_default() += 1;
        }
    }
    let lens_children: Vec<CollectionNode> = lens_map
        .iter()
        .map(|(lens, &count)| CollectionNode {
            id: format!("lens:{}", lens),
            label: lens.clone(),
            count,
            filter: Some(CollectionFilter {
                lens: Some(lens.clone()),
                ..Default::default()
            }),
            children: vec![],
        })
        .collect();
    let lens_group = CollectionNode {
        id: "lens".to_string(),
        label: "lens".to_string(),
        count: lens_children.iter().map(|c| c.count).sum(),
        filter: None,
        children: lens_children,
    };

    // ── By Tag ──
    let mut tag_map: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for entry in &index.entries {
        for tag in &entry.tags {
            *tag_map.entry(tag.clone()).or_default() += 1;
        }
    }
    let tag_children: Vec<CollectionNode> = tag_map
        .iter()
        .map(|(tag, &count)| CollectionNode {
            id: format!("tag:{}", tag),
            label: tag.clone(),
            count,
            filter: Some(CollectionFilter {
                tags: Some(tag.clone()),
                ..Default::default()
            }),
            children: vec![],
        })
        .collect();
    let tag_group = CollectionNode {
        id: "tag".to_string(),
        label: "tag".to_string(),
        count: tag_children.iter().map(|c| c.count).sum(),
        filter: None,
        children: tag_children,
    };

    // ── By Rating ──
    let mut rating_map: std::collections::BTreeMap<u8, usize> = std::collections::BTreeMap::new();
    for entry in &index.entries {
        *rating_map.entry(entry.rating).or_default() += 1;
    }
    let rating_children: Vec<CollectionNode> = (1u8..=5)
        .rev()
        .filter_map(|star| {
            rating_map.get(&star).map(|&count| CollectionNode {
                id: format!("rating:{}", star),
                label: "★".repeat(star as usize),
                count,
                filter: Some(CollectionFilter {
                    ratings: Some(star.to_string()),
                    ..Default::default()
                }),
                children: vec![],
            })
        })
        .collect();
    let rating_group = CollectionNode {
        id: "rating".to_string(),
        label: "rating".to_string(),
        count: rating_children.iter().map(|c| c.count).sum(),
        filter: None,
        children: rating_children,
    };

    // ── By Color Label ──
    let mut color_map: HashMap<String, usize> = HashMap::new();
    for entry in &index.entries {
        for cl in &entry.color_label {
            let s = cl.as_str();
            if !s.is_empty() {
                *color_map.entry(s.to_string()).or_default() += 1;
            }
        }
    }
    let color_order = ["red", "yellow", "green", "blue", "purple"];
    let color_children: Vec<CollectionNode> = color_order
        .iter()
        .filter_map(|&key| {
            color_map.get(key).map(|&count| {
                let mut label = key.to_string();
                label.get_mut(0..1).map(|s| s.make_ascii_uppercase());
                CollectionNode {
                    id: format!("color:{}", key),
                    label,
                    count,
                    filter: Some(CollectionFilter {
                        color_label: Some(key.to_string()),
                        ..Default::default()
                    }),
                    children: vec![],
                }
            })
        })
        .collect();
    let color_group = CollectionNode {
        id: "color".to_string(),
        label: "color".to_string(),
        count: color_children.iter().map(|c| c.count).sum(),
        filter: None,
        children: color_children,
    };

    // ── By Format ──
    let mut format_map: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();
    for entry in &index.entries {
        if let Some(ext) = std::path::Path::new(&entry.name)
            .extension()
            .and_then(|e| e.to_str())
        {
            *format_map.entry(ext.to_ascii_uppercase()).or_default() += 1;
        }
    }
    let format_children: Vec<CollectionNode> = format_map
        .iter()
        .map(|(fmt, &count)| CollectionNode {
            id: format!("format:{}", fmt.to_ascii_lowercase()),
            label: fmt.clone(),
            count,
            filter: Some(CollectionFilter {
                format: Some(fmt.to_ascii_lowercase()),
                ..Default::default()
            }),
            children: vec![],
        })
        .collect();
    let format_group = CollectionNode {
        id: "format".to_string(),
        label: "format".to_string(),
        count: format_children.iter().map(|c| c.count).sum(),
        filter: None,
        children: format_children,
    };

    let collections = vec![
        all_node,
        date_group,
        camera_group,
        lens_group,
        tag_group,
        rating_group,
        color_group,
        format_group,
    ];

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({ "collections": collections })),
    ))
}

#[named]
pub async fn api_albums_photos(
    locale: RequestLocale,
    token: Token,
    query: Query<AlbumPhotosQuery>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let clauses = super::filters::clauses_from_legacy_flat(
        query.from.as_deref(),
        query.to.as_deref(),
        query.tags.as_deref(),
        query.ratings.as_deref(),
        query.color_label.as_deref(),
        query.cameras.as_deref(),
        query.camera.as_deref(),
        query.lenses.as_deref(),
        query.lens.as_deref(),
        query.formats.as_deref(),
        query.format.as_deref(),
        query.q.as_deref(),
        query.person_ids.as_deref(),
    );
    list_album_photos_with_filters(
        locale,
        token,
        cfg,
        &query.album,
        &clauses,
        query.sort.as_deref(),
        query.collapse.as_deref(),
        query.burst_max_distance,
        query.offset,
        query.limit,
        None,
    )
    .await
}

#[named]
pub async fn api_albums_photos_query(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(body): Json<AlbumPhotosBody>,
) -> Result<Json<Value>, AppError> {
    list_album_photos_with_filters(
        locale,
        token,
        cfg,
        &body.album,
        &body.filters,
        body.sort.as_deref(),
        body.collapse.as_deref(),
        body.burst_max_distance,
        body.offset,
        body.limit,
        body.paths.as_deref(),
    )
    .await
}

pub(crate) async fn list_album_photos_with_filters(
    locale: RequestLocale,
    token: Token,
    cfg: Arc<AppState>,
    album: &str,
    clauses: &[super::filters::FilterClause],
    sort: Option<&str>,
    collapse: Option<&str>,
    burst_max_distance: Option<u32>,
    offset: Option<u32>,
    limit: Option<u32>,
    paths: Option<&[String]>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let resolved =
        resolve_album_access(&cfg, &username, &root_path, album, locale.as_str()).await?;
    let album_name = resolved.album_name;
    let root_path = resolved.root_path;
    let owner = resolved.owner;

    let album_dir = albums_root(root_path.as_path()).join(&album_name);
    let empty_response = || {
        let message = tr(locale.as_str(), "response.success");
        create_response(
            true,
            &message,
            0,
            Some(json!({
                "photos": [],
                "available_tags": [],
                "available_cameras": [],
                "available_lenses": [],
                "available_formats": [],
                "total_count": 0,
                "photo_count": 0,
                "offset": 0,
                "limit": 0,
            })),
        )
    };
    if !album_dir.exists() {
        return Ok(empty_response());
    }

    let Some(index) = load_or_queue_album_index(
        cfg.as_ref(),
        &username,
        root_path.as_path(),
        &owner,
        &album_name,
        locale.as_str(),
    )
    .await?
    else {
        return Ok(empty_response());
    };

    let registry = super::filters::FilterRegistry::global();
    let compiled = registry.compile_clauses(clauses, locale.as_str())?;
    let _ai_effects = super::filters::extract_ai_text_effects(&compiled);

    let person_ids = super::filters::collect_person_ids(clauses);
    let person_paths = if person_ids.is_empty() {
        None
    } else {
        let paths = crate::backend::ai::collect_person_photo_paths_from_config_async(
            album_dir.as_path(),
            album_name.as_str(),
            &person_ids,
        )
        .await;
        Some(paths)
    };
    let filter_ctx = super::filters::FilterEvalCtx { person_paths };

    let path_allowlist = paths.map(|items| {
        items
            .iter()
            .map(|path| path.trim().to_string())
            .filter(|path| !path.is_empty())
            .collect::<HashSet<_>>()
    });

    let offset = offset.unwrap_or(0) as usize;
    let requested_limit = match limit {
        Some(n) => n,
        None => cfg.settings_manager.photos_per_page().await,
    };
    let limit = if requested_limit == 0 {
        usize::MAX
    } else {
        requested_limit as usize
    };
    let include_facets = offset == 0;

    let sort_criteria: Vec<(&str, bool)> = sort
        .unwrap_or("")
        .split(',')
        .filter_map(|s| {
            let s = s.trim();
            if s.is_empty() {
                return None;
            }
            let mut parts = s.splitn(2, ':');
            let field = parts.next().unwrap_or("").trim();
            let dir = parts.next().unwrap_or("asc").trim();
            if field.is_empty() {
                return None;
            }
            Some((field, dir == "desc"))
        })
        .collect();

    let matches_filters =
        |p: &AlbumPhotoItem| super::filters::matches_compiled(p, &compiled, &filter_ctx);

    let matches_path_allowlist = |p: &AlbumPhotoItem| {
        path_allowlist
            .as_ref()
            .map(|allow| allow.contains(&p.path))
            .unwrap_or(true)
    };

    let (available_tags, available_cameras, available_lenses, available_formats) = if include_facets
    {
        let mut all_tags = HashSet::new();
        let mut all_cameras = HashSet::new();
        let mut all_lenses = HashSet::new();
        let mut all_formats = HashSet::new();

        for entry in &index.entries {
            for t in &entry.tags {
                all_tags.insert(t.clone());
            }
            let cam = format!("{} {}", entry.camera_make, entry.camera_model)
                .trim()
                .to_string();
            if !cam.is_empty() {
                all_cameras.insert(cam);
            }
            if !entry.lens.is_empty() {
                all_lenses.insert(entry.lens.clone());
            }
            if let Some(ext) = std::path::Path::new(&entry.name)
                .extension()
                .and_then(|e| e.to_str())
            {
                all_formats.insert(ext.to_uppercase());
            }
        }

        let mut available_tags = all_tags.into_iter().collect::<Vec<_>>();
        available_tags.sort();
        let mut available_cameras = all_cameras.into_iter().collect::<Vec<_>>();
        available_cameras.sort();
        let mut available_lenses = all_lenses.into_iter().collect::<Vec<_>>();
        available_lenses.sort();
        let mut available_formats = all_formats.into_iter().collect::<Vec<_>>();
        available_formats.sort();

        (
            available_tags,
            available_cameras,
            available_lenses,
            available_formats,
        )
    } else {
        (Vec::new(), Vec::new(), Vec::new(), Vec::new())
    };

    let album_key = album_access_key(&username, &owner, &album_name);
    let collapse_requested = collapse == Some("bursts");
    let filters_empty = clauses.is_empty() && path_allowlist.is_none();
    let date_sort =
        sort_criteria.is_empty() || (sort_criteria.len() == 1 && sort_criteria[0].0 == "date");
    let can_collapse = collapse_requested && filters_empty && date_sort;
    let burst_max_distance = burst_max_distance
        .unwrap_or(super::bursts::BURST_MAX_HAMMING)
        .min(super::bursts::BURST_MAX_HAMMING_LIMIT);

    let mut filtered: Vec<&AlbumPhotoItem> = index
        .entries
        .iter()
        .filter(|photo| matches_path_allowlist(photo) && matches_filters(photo))
        .collect();
    if !sort_criteria.is_empty() {
        filtered.sort_by(|a, b| {
            for &(field, desc) in &sort_criteria {
                let ord = match field {
                    "name" => a.name.cmp(&b.name),
                    "date" => a.date.cmp(&b.date),
                    "size" => a.size.cmp(&b.size),
                    "rating" => a.rating.cmp(&b.rating),
                    "camera" => {
                        let ca = format!("{} {}", a.camera_make, a.camera_model);
                        let cb = format!("{} {}", b.camera_make, b.camera_model);
                        ca.cmp(&cb)
                    }
                    "lens" => a.lens.cmp(&b.lens),
                    _ => std::cmp::Ordering::Equal,
                };
                let ord = if desc { ord.reverse() } else { ord };
                if ord != std::cmp::Ordering::Equal {
                    return ord;
                }
            }
            std::cmp::Ordering::Equal
        });
    }

    let burst_lookup = if collapse_requested {
        Some(
            super::bursts::lookup(
                &cfg,
                &username,
                &owner,
                &album_name,
                album_dir.as_path(),
                &index,
                can_collapse,
                burst_max_distance,
                locale.as_str(),
            )
            .await,
        )
    } else {
        None
    };

    let photo_count = filtered.len();
    let mut paginated_values: Vec<Value> = Vec::new();
    let total_count;
    if let Some(cache) = burst_lookup
        .as_ref()
        .and_then(|lookup| lookup.cache.as_ref())
    {
        let mut items = super::bursts::display_items(&index, cache, &album_name);
        let sort_desc = sort_criteria.first().map(|(_, desc)| *desc).unwrap_or(true);
        items.sort_by(|left, right| {
            let (left_time, left_name) = match left {
                super::bursts::BurstDisplay::Photo(photo) => {
                    (photo.datetime.as_str(), photo.name.as_str())
                }
                super::bursts::BurstDisplay::Stack { stack, .. } => {
                    if sort_desc {
                        (stack.end_datetime.as_str(), stack.cover.as_str())
                    } else {
                        (stack.start_datetime.as_str(), stack.cover.as_str())
                    }
                }
            };
            let (right_time, right_name) = match right {
                super::bursts::BurstDisplay::Photo(photo) => {
                    (photo.datetime.as_str(), photo.name.as_str())
                }
                super::bursts::BurstDisplay::Stack { stack, .. } => {
                    if sort_desc {
                        (stack.end_datetime.as_str(), stack.cover.as_str())
                    } else {
                        (stack.start_datetime.as_str(), stack.cover.as_str())
                    }
                }
            };
            let time_order = if sort_desc {
                right_time.cmp(left_time)
            } else {
                left_time.cmp(right_time)
            };
            time_order.then_with(|| left_name.cmp(right_name))
        });
        total_count = items.len();
        for item in items.into_iter().skip(offset).take(limit) {
            let (photo, stack) = match item {
                super::bursts::BurstDisplay::Photo(photo) => (photo, None),
                super::bursts::BurstDisplay::Stack { stack, cover } => (cover, Some(stack)),
            };
            let mut value = serde_json::to_value(photo).unwrap_or_else(|_| json!({}));
            if let Some(object) = value.as_object_mut() {
                object.insert("owner".to_string(), json!(&owner));
                object.insert("album".to_string(), json!(&album_key));
                if let Some(stack) = stack {
                    object.insert("stack".to_string(), super::bursts::stack_json(stack));
                }
            }
            paginated_values.push(value);
        }
    } else {
        total_count = filtered.len();
        for photo in filtered.into_iter().skip(offset).take(limit) {
            let mut value = serde_json::to_value(photo).unwrap_or_else(|_| json!({}));
            if let Some(object) = value.as_object_mut() {
                object.insert("owner".to_string(), json!(&owner));
                object.insert("album".to_string(), json!(&album_key));
            }
            paginated_values.push(value);
        }
    }

    let bursts_value = burst_lookup.as_ref().map(|lookup| {
        let mut value = json!({
            "state": lookup.state,
            "algorithm": super::bursts::BURST_ALGORITHM,
            "max_distance": burst_max_distance,
            "stack_count": lookup.cache.as_ref().map(|cache| cache.stacks.len()).unwrap_or(0),
        });
        if let Some(reason) = lookup.reason {
            value["reason"] = json!(reason);
        }
        if let Some(task_key) = &lookup.task_key {
            value["task_key"] = json!(task_key);
        }
        value
    });
    let message = tr(locale.as_str(), "response.success");
    let mut data = json!({
        "photos": paginated_values,
        "available_tags": available_tags,
        "available_cameras": available_cameras,
        "available_lenses": available_lenses,
        "available_formats": available_formats,
        "total_count": total_count,
        "photo_count": photo_count,
        "offset": offset,
        "limit": requested_limit,
    });
    if let Some(bursts) = bursts_value {
        data["bursts"] = bursts;
    }
    Ok(create_response(true, &message, 0, Some(data)))
}

#[named]
pub async fn api_albums_stack(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(payload): Json<AlbumStackBody>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let resolved =
        resolve_album_access(&cfg, &username, &root_path, &payload.album, locale.as_str()).await?;
    let album_name = resolved.album_name;
    let owner = resolved.owner;
    let album_dir = albums_root(resolved.root_path.as_path()).join(&album_name);
    let Some(index) = load_existing_album_index(
        cfg.data_path.as_path(),
        resolved.root_path.as_path(),
        &owner,
        &album_name,
    )
    .await?
    else {
        return Err(AppError::NotFound(tr(
            locale.as_str(),
            "error.stack_not_found",
        )));
    };
    let cache_id = ensure_album_cache_id(album_dir.as_path()).await?;
    let Some(cache) = super::bursts::read_cache(cfg.data_path.as_path(), &owner, &cache_id).await
    else {
        return Err(AppError::NotFound(tr(
            locale.as_str(),
            "error.stack_not_found",
        )));
    };
    let Some(stack) = cache
        .stacks
        .iter()
        .find(|stack| stack.id == payload.stack_id)
    else {
        return Err(AppError::NotFound(tr(
            locale.as_str(),
            "error.stack_not_found",
        )));
    };

    let album_key = album_access_key(&username, &owner, &album_name);
    let mut photos = Vec::with_capacity(stack.members.len());
    for rel in &stack.members {
        if into_host_path_safely(album_dir.as_path(), rel).is_err() {
            return Err(AppError::NotFound(tr(
                locale.as_str(),
                "error.stack_not_found",
            )));
        }
        let api_path = format!("albums/{album_name}/{rel}");
        let Some(entry) = index.entries.iter().find(|entry| entry.path == api_path) else {
            continue;
        };
        let mut value = serde_json::to_value(entry).unwrap_or_else(|_| json!({}));
        if let Some(object) = value.as_object_mut() {
            object.insert("path".to_string(), json!(rel));
            object.insert("owner".to_string(), json!(&owner));
            object.insert("album".to_string(), json!(&album_key));
        }
        photos.push(value);
    }
    if photos.is_empty() {
        return Err(AppError::NotFound(tr(
            locale.as_str(),
            "error.stack_not_found",
        )));
    }

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "owner": owner,
            "album": album_key,
            "stack": {
                "id": stack.id,
                "count": stack.count,
                "cover": stack.cover,
            },
            "photos": photos,
        })),
    ))
}

#[named]
pub async fn api_albums_photo_meta(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<UpdatePhotoMetaPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, user_root)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    // Determine the effective root path and owner for the photo
    let (root_path, index_owner, album_name) = if let Some(album_ref) = &payload.album {
        let resolved =
            resolve_album_access(&cfg, &username, &user_root, album_ref, locale.as_str()).await?;
        if !resolved.can_write {
            return Err(AppError::Forbidden(tr(
                locale.as_str(),
                "error.no_write_permission_album",
            )));
        }
        (
            resolved.root_path,
            resolved.owner,
            Some(resolved.album_name),
        )
    } else {
        (user_root, username.clone(), None)
    };

    let photo = into_host_path_safely(root_path.as_path(), &payload.path)?;
    if let Some(album_name) = album_name.as_ref() {
        let album_dir = albums_root(root_path.as_path()).join(album_name);
        if !photo.starts_with(&album_dir) {
            return Err(AppError::Forbidden(tr(
                locale.as_str(),
                "error.invalid_shared_path",
            )));
        }
    }
    if !photo.exists() || !photo.is_file() || !is_album_media_path(photo.as_path()) {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_image_path",
        )));
    }

    let mut current = read_image_software_meta(photo.as_path()).await;
    if let Some(tags) = &payload.tags {
        current.tags = normalize_tags(tags);
    }
    if let Some(rating) = payload.rating {
        current.rating = rating.min(5);
    }
    if let Some(colors) = &payload.color_label {
        current.color_label = colors
            .iter()
            .map(|s| ColorLabel::from_str_label(s))
            .filter(|c| *c != ColorLabel::None)
            .collect();
    }
    write_image_software_meta_if_needed(photo.as_path(), &current).await?;

    let photo_rel = relative_display_path(root_path.as_path(), photo.as_path());
    if let Some(album_name) = photo_rel
        .strip_prefix("albums/")
        .and_then(|rest| rest.split('/').next())
    {
        let album_dir = albums_root(root_path.as_path()).join(album_name);
        if let Ok(index_path) =
            album_index_path_for_dir(cfg.data_path.as_path(), &index_owner, album_dir.as_path())
                .await
            && index_path.exists()
        {
            if let Ok(raw) = tokio::fs::read_to_string(&index_path).await {
                if let Ok(mut index) = serde_json::from_str::<AlbumIndex>(&raw) {
                    if let Some(entry) = index.entries.iter_mut().find(|e| e.path == photo_rel) {
                        entry.tags = current.tags.clone();
                        entry.rating = current.rating;
                        entry.color_label = current.color_label.clone();
                    }
                    if let Ok(data) = serde_json::to_string(&index) {
                        let _ = tokio::fs::write(&index_path, data).await;
                    }
                }
            }
        }
    }

    let message = tr(locale.as_str(), "response.metadata_updated");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "tags": current.tags,
            "rating": current.rating,
            "color_label": current.color_label,
        })),
    ))
}

#[derive(Debug, Deserialize)]
pub struct DeletePhotosPayload {
    album: String,
    paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct RebuildIndexPayload {
    album: String,
}

#[named]
pub async fn api_albums_rebuild_index(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<RebuildIndexPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, user_root)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let resolved =
        resolve_album_access(&cfg, &username, &user_root, &payload.album, locale.as_str()).await?;

    let (task_id, created) = super::workflow::queue_rebuild_workflow(
        cfg.clone(),
        username,
        resolved.root_path,
        resolved.owner,
        resolved.album_name,
        locale.as_str().to_string(),
    )
    .await;

    let message = if created {
        tr(locale.as_str(), "response.rebuild_index_task_created")
    } else {
        tr(
            locale.as_str(),
            "response.rebuild_index_task_already_running",
        )
    };
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({ "task_id": task_id.to_string(), "created": created })),
    ))
}

// ── Delete photos ────────────────────────────────────────────────────────────

#[named]
pub async fn api_albums_delete_photos(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<DeletePhotosPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, user_root)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let resolved =
        resolve_album_access(&cfg, &username, &user_root, &payload.album, locale.as_str()).await?;
    if !resolved.can_delete {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.no_delete_permission_album",
        )));
    }

    if payload.paths.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.no_photos_selected",
        )));
    }

    let album_dir = albums_root(&resolved.root_path).join(&resolved.album_name);
    let mut deleted = 0usize;
    for rel_path in &payload.paths {
        let full = into_host_path_safely(resolved.root_path.as_path(), rel_path)?;
        // Ensure the file is actually inside this album dir
        if !full.starts_with(&album_dir) {
            continue;
        }
        if full.is_file() {
            let _ = tokio::fs::remove_file(&full).await;
            // Also remove XMP sidecar metadata
            let sidecar = xmp_sidecar_path(&full);
            if sidecar.exists() {
                let _ = tokio::fs::remove_file(&sidecar).await;
            }
            deleted += 1;
        }
    }

    // Clean up empty directories inside the album
    cleanup_empty_dirs(&album_dir).await;

    let remaining = load_existing_album_index(
        cfg.data_path.as_path(),
        resolved.root_path.as_path(),
        &resolved.owner,
        &resolved.album_name,
    )
    .await
    .ok()
    .flatten()
    .map(|index| index.entries.len().saturating_sub(deleted))
    .unwrap_or(0);

    queue_rebuild_album_index(
        cfg.clone(),
        username.clone(),
        resolved.root_path.clone(),
        resolved.owner.clone(),
        resolved.album_name.clone(),
        locale.as_str(),
    )
    .await;

    let message = trf(
        locale.as_str(),
        "response.photos_deleted",
        &[("count", deleted.to_string())],
    );
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "deleted": deleted,
            "remaining": remaining,
        })),
    ))
}

// ── Copy photo to another album ──────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CopyPhotoPayload {
    photo_path: String,
    source_album: String,
    target_album: String,
}

pub async fn api_albums_copy_photo(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<CopyPhotoPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, user_root)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    // Resolve source album (read access required)
    let source = resolve_album_access(
        &cfg,
        &username,
        &user_root,
        &payload.source_album,
        locale.as_str(),
    )
    .await?;

    // Resolve target album (write access required)
    let target = resolve_album_access(
        &cfg,
        &username,
        &user_root,
        &payload.target_album,
        locale.as_str(),
    )
    .await?;
    if !target.can_write {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.no_write_permission_album",
        )));
    }

    // Validate that photo path is inside the source album directory
    let source_file = into_host_path_safely(source.root_path.as_path(), &payload.photo_path)?;
    let source_album_dir = albums_root(&source.root_path).join(&source.album_name);
    if !source_file.starts_with(&source_album_dir) {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.invalid_shared_path",
        )));
    }
    if !source_file.is_file() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", payload.photo_path.clone())],
        )));
    }

    // Destination: target album dir + same filename (handle conflicts)
    let target_album_dir = albums_root(&target.root_path).join(&target.album_name);
    tokio::fs::create_dir_all(&target_album_dir)
        .await
        .map_err(|e| AppError::Unknown(format!("create target album dir failed: {e}")))?;

    let file_name = source_file
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| AppError::BadRequest("invalid file name".to_string()))?;
    let stem = std::path::Path::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(file_name);
    let ext = std::path::Path::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let dest = {
        let candidate = target_album_dir.join(file_name);
        if !candidate.exists() {
            candidate
        } else {
            let mut i = 1u32;
            loop {
                let name = if ext.is_empty() {
                    format!("{}_{}", stem, i)
                } else {
                    format!("{}_{}.{}", stem, i, ext)
                };
                let candidate = target_album_dir.join(&name);
                if !candidate.exists() {
                    break candidate;
                }
                i += 1;
            }
        }
    };

    tokio::fs::copy(&source_file, &dest)
        .await
        .map_err(|e| AppError::Unknown(format!("copy photo failed: {e}")))?;

    queue_rebuild_album_index(
        cfg.clone(),
        username.clone(),
        target.root_path.clone(),
        target.owner.clone(),
        target.album_name.clone(),
        locale.as_str(),
    )
    .await;

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(true, &message, 0, None))
}

/// Remove empty directories recursively inside `dir`.
#[async_recursion]
pub(crate) async fn cleanup_empty_dirs(dir: &FsPath) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.metadata().await.map(|m| m.is_dir()).unwrap_or(false) {
            cleanup_empty_dirs(&entry.path()).await;
        }
    }
    // After cleaning children, remove this dir if it's now empty
    // (but only sub-dirs, not the album root — we check by re-reading)
    if let Ok(mut re) = tokio::fs::read_dir(dir).await {
        if re.next_entry().await.ok().flatten().is_none() {
            let _ = tokio::fs::remove_dir(dir).await;
        }
    }
}

// ── Sharing ──────────────────────────────────────────────────────────────────

/// Verify that `username` has read access to a photo path owned by `owner`.
/// The path should be of form "albums/{album_name}/..." relative to owner's root.
pub async fn verify_shared_photo_access(
    cfg: &AppState,
    username: &str,
    owner: &str,
    path: &str,
    locale: &str,
) -> Result<(), AppError> {
    // Callers pass the decoded path from `Path`/JSON; decoding again would
    // misread a literal `%` in a filename.
    let path = path.trim_start_matches('/');
    let Some(album_name) = path
        .strip_prefix("albums/")
        .and_then(|rest| rest.split('/').next())
        .and_then(sanitize_album_name)
    else {
        return Err(AppError::Forbidden(tr(locale, "error.invalid_shared_path")));
    };
    let owner_root = cfg.user_root_path(owner);
    let album_dir = albums_root(&owner_root).join(&album_name);
    if !album_dir.exists() {
        return Err(AppError::NotFound(trf(
            locale,
            "error.resource_not_found",
            &[("value", path.to_string())],
        )));
    }
    let meta = read_album_meta(&album_dir).await;
    if !meta.shares.iter().any(|s| s.username == username) {
        return Err(AppError::Forbidden(tr(locale, "error.no_album_access")));
    }
    let source = into_host_path_safely(owner_root.as_path(), path)?;
    if !source.starts_with(&album_dir) {
        return Err(AppError::Forbidden(tr(locale, "error.invalid_shared_path")));
    }
    Ok(())
}

/// Resolved album access info for sharing-aware endpoints.
pub(crate) struct ResolvedAlbum {
    pub(crate) owner: String,
    pub(crate) root_path: PathBuf,
    pub(crate) album_name: String,
    pub(crate) can_write: bool,
    pub(crate) can_delete: bool,
    pub(crate) is_owner: bool,
}

impl ResolvedAlbum {
    /// The album as a validated value. `resolve_album_access` already checked
    /// both components, so a rejected value means a caller built this struct by
    /// hand (tests do).
    pub(crate) fn album_ref(&self) -> Option<AlbumRef> {
        let owner = Owner::new(&self.owner)?;
        let name = AlbumName::new(&self.album_name)?;
        Some(AlbumRef::new(owner, name))
    }
}

pub(crate) async fn resolve_album_access(
    cfg: &AppState,
    username: &str,
    user_root: &FsPath,
    album_input: &str,
    locale: &str,
) -> Result<ResolvedAlbum, AppError> {
    // Try "owner/album_name" format for shared albums
    if let Some((owner, album_name)) = album_input.split_once('/') {
        if owner != username {
            let Some(album_name) = sanitize_album_name(album_name) else {
                return Err(AppError::BadRequest(tr(locale, "error.invalid_album_name")));
            };
            let owner_root = cfg.user_root_path(owner);
            let album_dir = albums_root(&owner_root).join(&album_name);
            if !album_dir.exists() {
                return Err(AppError::NotFound(trf(
                    locale,
                    "error.resource_not_found",
                    &[("value", album_input.to_string())],
                )));
            }
            let meta = read_album_meta(&album_dir).await;
            let share = meta.shares.iter().find(|s| s.username == username);
            let Some(share) = share else {
                return Err(AppError::Forbidden(tr(locale, "error.album_access_denied")));
            };
            return Ok(ResolvedAlbum {
                owner: owner.to_string(),
                root_path: owner_root,
                album_name,
                can_write: share.can_write,
                can_delete: share.can_delete,
                is_owner: false,
            });
        }
    }
    // Own album
    let Some(album_name) = sanitize_album_name(album_input) else {
        return Err(AppError::BadRequest(tr(locale, "error.invalid_album_name")));
    };
    Ok(ResolvedAlbum {
        owner: username.to_string(),
        root_path: user_root.to_path_buf(),
        album_name,
        can_write: true,
        can_delete: true,
        is_owner: true,
    })
}

// ── Dashboard helpers ────────────────────────────────────────────────────────

#[derive(Clone)]
pub(crate) struct AlbumSource {
    dir_name: String,
    owner: String,
    owner_root: PathBuf,
    display_name: String,
    cover: String,
    description: String,
}

impl AlbumSource {
    /// Owner of the album. Storage stats use it to separate shared albums from
    /// the caller's own data root.
    pub(crate) fn owner(&self) -> &str {
        &self.owner
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AiAlbumScanTarget {
    pub owner: String,
    pub album_name: String,
    pub album_dir: PathBuf,
    /// Stable `AlbumMeta.cache` used as AI embedding subdirectory.
    pub cache_id: String,
}

async fn push_visible_album_source(
    sources: &mut Vec<AlbumSource>,
    owner: &str,
    owner_root: PathBuf,
    album_dir: &FsPath,
    dir_name: String,
    require_share_for: Option<&str>,
) {
    if !is_user_visible_album_dir(&dir_name) {
        return;
    }
    let meta = read_album_meta(album_dir).await;
    if let Some(viewer) = require_share_for {
        if !meta.shares.iter().any(|share| share.username == viewer) {
            return;
        }
    }
    sources.push(AlbumSource {
        dir_name: dir_name.clone(),
        owner: owner.to_string(),
        owner_root,
        display_name: album_display_name(&meta, &dir_name),
        cover: meta.cover,
        description: meta.description,
    });
}

/// Gather every album accessible by `username`: own albums + albums shared to them.
/// Hidden system dirs (`__*`, e.g. `__uploading`) are excluded — same口径 as album list.
pub(crate) async fn collect_accessible_album_sources(
    cfg: &AppState,
    username: &str,
) -> Vec<AlbumSource> {
    let mut sources = Vec::new();
    let user_root = cfg.user_root_path(username);

    let albums_dir = albums_root(&user_root);
    if let Ok(mut entries) = tokio::fs::read_dir(&albums_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.metadata().await.map(|m| m.is_dir()).unwrap_or(false) {
                let dir_name = entry.file_name().to_string_lossy().to_string();
                push_visible_album_source(
                    &mut sources,
                    username,
                    user_root.clone(),
                    &entry.path(),
                    dir_name,
                    None,
                )
                .await;
            }
        }
    }

    let all_users = cfg.user_manager.list_usernames().await;
    for owner in &all_users {
        if owner == username {
            continue;
        }
        let owner_root = cfg.user_root_path(owner);
        let albums_dir = albums_root(&owner_root);
        if let Ok(mut entries) = tokio::fs::read_dir(&albums_dir).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                if entry.metadata().await.map(|m| m.is_dir()).unwrap_or(false) {
                    let dir_name = entry.file_name().to_string_lossy().to_string();
                    push_visible_album_source(
                        &mut sources,
                        owner,
                        owner_root.clone(),
                        &entry.path(),
                        dir_name,
                        Some(username),
                    )
                    .await;
                }
            }
        }
    }

    sources
}

fn enrich_photo_value(entry: &AlbumPhotoItem, viewer: &str, source: &AlbumSource) -> Value {
    let album_key = album_access_key(viewer, &source.owner, &source.dir_name);
    let mut photo = serde_json::to_value(entry).unwrap_or_default();
    if let Some(object) = photo.as_object_mut() {
        object.insert("album".to_string(), json!(&album_key));
        object.insert("album_name".to_string(), json!(&source.display_name));
        object.insert("owner".to_string(), json!(&source.owner));
    }
    photo
}

async fn load_accessible_album_indexes(
    cfg: &AppState,
    viewer: &str,
    sources: &[AlbumSource],
    locale: &str,
) -> Result<Vec<(AlbumSource, AlbumIndex)>, AppError> {
    let mut out = Vec::with_capacity(sources.len());
    for source in sources {
        let index = load_or_queue_album_index(
            cfg,
            viewer,
            &source.owner_root,
            &source.owner,
            &source.dir_name,
            locale,
        )
        .await?
        .unwrap_or(AlbumIndex {
            version: 2,
            entries: Vec::new(),
        });
        out.push((source.clone(), index));
    }
    Ok(out)
}

pub(crate) async fn collect_accessible_ai_scan_targets(
    cfg: &AppState,
    username: &str,
    album: Option<&str>,
    locale: &str,
) -> Result<Vec<AiAlbumScanTarget>, AppError> {
    if let Some(album_input) = album {
        let user_root = cfg.user_root_path(username);
        let resolved = resolve_album_access(cfg, username, &user_root, album_input, locale).await?;
        let album_dir = albums_root(resolved.root_path.as_path()).join(&resolved.album_name);
        if !album_dir.exists() {
            return Err(AppError::NotFound(trf(
                locale,
                "error.resource_not_found",
                &[("value", album_input.to_string())],
            )));
        }
        let cache_id = ensure_album_cache_id(album_dir.as_path()).await?;
        return Ok(vec![AiAlbumScanTarget {
            owner: resolved.owner,
            album_name: resolved.album_name,
            album_dir,
            cache_id,
        }]);
    }

    let mut targets = Vec::new();
    for source in collect_accessible_album_sources(cfg, username).await {
        let album_dir = albums_root(&source.owner_root).join(&source.dir_name);
        let cache_id = ensure_album_cache_id(album_dir.as_path())
            .await
            .unwrap_or_else(|_| source.dir_name.clone());
        targets.push(AiAlbumScanTarget {
            owner: source.owner,
            album_name: source.dir_name,
            album_dir,
            cache_id,
        });
    }
    Ok(targets)
}

pub async fn api_dashboard(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let is_admin = session.is_admin();
    let username = session.username;
    let sources = collect_accessible_album_sources(&cfg, &username).await;
    let own_album_count = sources
        .iter()
        .filter(|source| source.owner == username)
        .count();
    let shared_album_count = sources.len().saturating_sub(own_album_count);

    let now = chrono::Local::now();
    let today = now.format("%Y-%m-%d").to_string();
    let today_md = now.format("%m-%d").to_string();
    let mut accessible_photo_count = 0usize;
    let mut total_bytes = 0u64;
    let mut dated_photo_count = 0usize;
    let mut photos = Vec::<Value>::new();
    let mut albums = Vec::<(String, Value)>::new();

    let indexed =
        load_accessible_album_indexes(cfg.as_ref(), &username, &sources, locale.as_str()).await?;

    for (source, index) in indexed {
        let album_key = album_access_key(&username, &source.owner, &source.dir_name);
        accessible_photo_count += index.entries.len();
        total_bytes += index.entries.iter().map(|entry| entry.size).sum::<u64>();
        dated_photo_count += index
            .entries
            .iter()
            .filter(|entry| !entry.date.is_empty() && entry.date != "1970-01-01")
            .count();

        let latest_date = index
            .entries
            .iter()
            .map(|entry| entry.date.as_str())
            .max()
            .unwrap_or("")
            .to_string();
        albums.push((
            latest_date.clone(),
            json!({
                "album": album_key,
                "dir_name": album_key,
                "name": source.display_name,
                "owner": source.owner,
                "cover": source.cover,
                "description": source.description,
                "photo_count": index.entries.len(),
                "latest_date": latest_date,
            }),
        ));

        for entry in &index.entries {
            photos.push(enrich_photo_value(entry, &username, &source));
        }
    }

    photos.sort_by(|left, right| {
        let left_date = left
            .get("datetime")
            .and_then(Value::as_str)
            .filter(|date| !date.is_empty())
            .or_else(|| left.get("date").and_then(Value::as_str))
            .unwrap_or("");
        let right_date = right
            .get("datetime")
            .and_then(Value::as_str)
            .filter(|date| !date.is_empty())
            .or_else(|| right.get("date").and_then(Value::as_str))
            .unwrap_or("");
        right_date.cmp(left_date).then_with(|| {
            left.get("path")
                .and_then(Value::as_str)
                .unwrap_or("")
                .cmp(right.get("path").and_then(Value::as_str).unwrap_or(""))
        })
    });
    let recent_photos = photos.iter().take(12).cloned().collect::<Vec<_>>();

    albums.sort_by(|left, right| right.0.cmp(&left.0));
    let recent_albums = albums
        .into_iter()
        .take(6)
        .map(|(_, album)| album)
        .collect::<Vec<_>>();

    let on_this_day_photos = photos
        .iter()
        .filter(|photo| {
            photo
                .get("date")
                .and_then(Value::as_str)
                .and_then(|date| date.get(5..))
                == Some(today_md.as_str())
        })
        .take(8)
        .cloned()
        .collect::<Vec<_>>();
    let on_this_day_count = on_this_day_photos.len();

    let seed = format!("{username}|{today}");
    let mut daily_picks = photos.clone();
    daily_picks.sort_by_cached_key(|photo| {
        let path = photo.get("path").and_then(Value::as_str).unwrap_or("");
        let album = photo.get("album").and_then(Value::as_str).unwrap_or("");
        let mut hasher = Sha256::new();
        hasher.update(seed.as_bytes());
        hasher.update(b"|");
        hasher.update(album.as_bytes());
        hasher.update(b"|");
        hasher.update(path.as_bytes());
        let result = hasher.finalize();
        u64::from_le_bytes(result[..8].try_into().unwrap_or([0u8; 8]))
    });
    daily_picks.truncate(8);

    let task_map = cfg.task_mgr.get_task_status_map_for_user(&username).await;
    let mut active_tasks = task_map
        .into_iter()
        .filter_map(|(id, mut task)| {
            let status = TaskStatus::from_raw(task.get("status").and_then(Value::as_u64)?)?;
            if !status.is_active() {
                return None;
            }
            task.as_object_mut()?.insert("id".to_string(), json!(id));
            Some(task)
        })
        .collect::<Vec<_>>();
    let active_task_count = active_tasks.len();
    active_tasks.sort_by(|left, right| {
        left.get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .cmp(right.get("name").and_then(Value::as_str).unwrap_or(""))
    });
    active_tasks.truncate(6);

    let admin = if is_admin {
        let settings = cfg.settings_manager.get_all().await;
        Some(json!({
            "user_count": cfg.user_manager.list_usernames().await.len(),
            "webdav_share_count": crate::backend::settings::webdav_share_count(&cfg.data_path).await,
            "settings": {
                "allow_registration": settings.get("allow_registration").cloned().unwrap_or(json!(false)),
                "thumbnail_generation_mode": settings.get("thumbnail_generation_mode").cloned().unwrap_or(json!("on_demand")),
                "thumbnail_concurrency": settings.get("thumbnail_concurrency").cloned().unwrap_or(json!(3)),
                "darktable_concurrency": settings.get("darktable_concurrency").cloned().unwrap_or(json!(1)),
            }
        }))
    } else {
        None
    };

    let data = json!({
        "summary": {
            "own_album_count": own_album_count,
            "shared_album_count": shared_album_count,
            "accessible_photo_count": accessible_photo_count,
            "total_bytes": total_bytes,
            "dated_photo_count": dated_photo_count,
            "active_task_count": active_task_count,
            "uptime_seconds": cfg.started_at.elapsed().as_secs(),
        },
        "recent_photos": recent_photos,
        "recent_albums": recent_albums,
        "on_this_day": {
            "date": today_md,
            "count": on_this_day_count,
            "photos": on_this_day_photos,
        },
        "daily_picks": {
            "date": today,
            "photos": daily_picks,
        },
        "active_tasks": active_tasks,
        "admin": admin,
    });
    Ok(create_response(
        true,
        &tr(locale.as_str(), "response.success"),
        0,
        Some(data),
    ))
}

// ── On This Day ──────────────────────────────────────────────────────────────

pub async fn api_on_this_day(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some((username, _)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    use chrono::Local;
    use std::collections::BTreeMap;

    let now = Local::now();
    let today_md = format!("{:02}-{:02}", now.month(), now.day());

    let sources = collect_accessible_album_sources(&cfg, &username).await;
    let indexed =
        load_accessible_album_indexes(cfg.as_ref(), &username, &sources, locale.as_str()).await?;
    let mut by_year: BTreeMap<String, Vec<Value>> = BTreeMap::new();

    for (src, index) in indexed {
        for entry in &index.entries {
            let date_str = entry.date.as_str();
            if date_str.len() != 10 {
                continue;
            }
            let entry_md = &date_str[5..];
            if entry_md != today_md {
                continue;
            }
            let year = date_str[..4].to_string();
            by_year
                .entry(year)
                .or_default()
                .push(enrich_photo_value(entry, &username, &src));
        }
    }

    let groups: Vec<Value> = by_year
        .into_iter()
        .rev()
        .map(|(year, photos)| json!({ "year": year, "photos": photos }))
        .collect();

    Ok(Json(json!({
        "ok": true,
        "date": format!("{:02}-{:02}", now.month(), now.day()),
        "groups": groups,
    })))
}

// ── Daily Picks ──────────────────────────────────────────────────────────────

pub async fn api_daily_picks(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some((username, _)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    use chrono::Local;

    let today = Local::now().format("%Y-%m-%d").to_string();
    let sources = collect_accessible_album_sources(&cfg, &username).await;
    let indexed =
        load_accessible_album_indexes(cfg.as_ref(), &username, &sources, locale.as_str()).await?;

    let mut all_photos: Vec<Value> = Vec::new();
    for (src, index) in indexed {
        for entry in &index.entries {
            all_photos.push(enrich_photo_value(entry, &username, &src));
        }
    }

    // Stable daily shuffle: sort by SHA-256(seed | album | path), take top 24.
    let seed = format!("{}|{}", username, today);
    all_photos.sort_by_cached_key(|p| {
        let path = p.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let album = p.get("album").and_then(|v| v.as_str()).unwrap_or("");
        let mut h = Sha256::new();
        h.update(seed.as_bytes());
        h.update(b"|");
        h.update(album.as_bytes());
        h.update(b"|");
        h.update(path.as_bytes());
        let result = h.finalize();
        u64::from_le_bytes(result[..8].try_into().unwrap_or([0u8; 8]))
    });
    let photos: Vec<Value> = all_photos.into_iter().take(24).collect();

    Ok(Json(json!({
        "ok": true,
        "date": today,
        "photos": photos,
    })))
}

/// Collect albums shared with `username` by scanning all other users' album dirs.
pub(crate) async fn collect_shared_albums(
    cfg: &AppState,
    username: &str,
    locale: &str,
) -> Vec<Value> {
    struct Candidate {
        owner: String,
        owner_root: PathBuf,
        dir_name: String,
        display_name: String,
        cover: String,
        description: String,
        can_write: bool,
        can_delete: bool,
    }

    // Phase 1 (sequential, cheap metadata reads): collect albums where username has access.
    let all_users = cfg.user_manager.list_usernames().await;
    let mut candidates: Vec<Candidate> = Vec::new();
    for owner in &all_users {
        if owner == username {
            continue;
        }
        let owner_root = cfg.user_root_path(owner);
        let albums_dir = albums_root(&owner_root);
        let Ok(mut entries) = tokio::fs::read_dir(&albums_dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(ft) = entry.metadata().await else {
                continue;
            };
            if !ft.is_dir() {
                continue;
            }
            let dir_name = entry.file_name().to_string_lossy().to_string();
            if !is_user_visible_album_dir(&dir_name) {
                continue;
            }
            let meta = read_album_meta(&entry.path()).await;
            let Some(share) = meta.shares.iter().find(|s| s.username == username) else {
                continue;
            };
            candidates.push(Candidate {
                owner: owner.clone(),
                owner_root: owner_root.clone(),
                dir_name: dir_name.clone(),
                display_name: album_display_name(&meta, &dir_name),
                cover: meta.cover,
                description: meta.description,
                can_write: share.can_write,
                can_delete: share.can_delete,
            });
        }
    }

    // Indexes are queued in the background when absent, so listing shares
    // remains responsive instead of launching synchronous EXIF scans.
    let mut shared_albums = Vec::with_capacity(candidates.len());
    for c in candidates {
        let index =
            load_or_queue_album_index(cfg, username, &c.owner_root, &c.owner, &c.dir_name, locale)
                .await
                .ok()
                .flatten();
        let photo_count = index.map(|i| i.entries.len()).unwrap_or(0);
        shared_albums.push(json!({
            "dir_name": album_access_key(username, &c.owner, &c.dir_name),
            "name": c.display_name,
            "cover": c.cover,
            "description": c.description,
            "photo_count": photo_count,
            "owner": c.owner,
            "can_write": c.can_write,
            "can_delete": c.can_delete,
        }));
    }
    shared_albums
}

#[derive(Debug, Deserialize)]
pub struct ShareAlbumPayload {
    dir_name: String,
    username: String,
    #[serde(default)]
    can_write: bool,
    #[serde(default)]
    can_delete: bool,
}

#[named]
pub async fn api_albums_share(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<ShareAlbumPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(dir_name) = sanitize_album_name(&payload.dir_name) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_album_identifier",
        )));
    };
    let target_user = payload.username.trim();
    if target_user == username {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.share_self",
        )));
    }
    if !cfg.user_manager.user_exists(target_user).await {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.target_user_not_found",
        )));
    }

    let album_dir = albums_root(root_path.as_path()).join(&dir_name);
    if !album_dir.exists() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", dir_name.clone())],
        )));
    }

    let mut meta = read_album_meta(album_dir.as_path()).await;
    if let Some(existing) = meta.shares.iter_mut().find(|s| s.username == target_user) {
        existing.can_write = payload.can_write;
        existing.can_delete = payload.can_delete;
    } else {
        meta.shares.push(AlbumShare {
            username: target_user.to_string(),
            can_write: payload.can_write,
            can_delete: payload.can_delete,
        });
    }
    write_album_meta(album_dir.as_path(), &meta).await?;

    let message = tr(locale.as_str(), "response.album_shared");
    Ok(create_response(true, &message, 0, None))
}

#[derive(Debug, Deserialize)]
pub struct UnshareAlbumPayload {
    dir_name: String,
    username: String,
}

#[named]
pub async fn api_albums_unshare(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<UnshareAlbumPayload>,
) -> Result<Json<Value>, AppError> {
    let Some(root_path) = cfg.get_root_path(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(dir_name) = sanitize_album_name(&payload.dir_name) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_album_identifier",
        )));
    };

    let album_dir = albums_root(root_path.as_path()).join(&dir_name);
    if !album_dir.exists() {
        return Err(AppError::NotFound(trf(
            locale.as_str(),
            "error.resource_not_found",
            &[("value", dir_name.clone())],
        )));
    }

    let mut meta = read_album_meta(album_dir.as_path()).await;
    let target_user = payload.username.trim();
    meta.shares.retain(|s| s.username != target_user);
    write_album_meta(album_dir.as_path(), &meta).await?;

    let message = tr(locale.as_str(), "response.album_unshared");
    Ok(create_response(true, &message, 0, None))
}

// ── Album local-upload ────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct AlbumUploadFileMeta {
    pub bytes: u64,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct AlbumUploadPayload {
    album: String,
    files: Vec<AlbumUploadFileMeta>,
    #[serde(default)]
    conflict: crate::backend::files::ConflictAction,
}

#[named]
pub async fn api_albums_upload(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<AlbumUploadPayload>,
) -> Result<Json<Value>, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let Some(album_name) = sanitize_album_name(&payload.album) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.invalid_album_identifier",
        )));
    };
    if payload.files.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.no_photos_selected",
        )));
    }
    let conflict = payload.conflict;

    let staging_uuid = Uuid::new_v4();
    let staging_dir = root_path
        .join("albums")
        .join("__uploading")
        .join(staging_uuid.to_string());

    // Build UploadFileMeta list — absolute host paths inside the staging dir.
    let metas: Vec<crate::backend::files::UploadFileMeta> = payload
        .files
        .iter()
        .filter(|f| !f.name.is_empty() && !f.name.contains('/') && !f.name.contains('\\'))
        .map(|f| crate::backend::files::UploadFileMeta {
            bytes: f.bytes,
            is_dir: false,
            path: staging_dir.join(&f.name).to_string_lossy().to_string(),
            request_path: format!("albums/__uploading/{}/{}", staging_uuid, f.name),
        })
        .collect();

    if metas.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.no_photos_selected",
        )));
    }

    let task_name = trf(
        locale.as_str(),
        "task.upload_album",
        &[("album", album_name.clone())],
    );
    let staging_rel = format!("albums/__uploading/{}", staging_uuid);
    let upload = Arc::new(crate::backend::files::UploadFiles::new(
        task_name,
        username.clone(),
        root_path.clone(),
        metas,
        conflict,
    ));
    let (task_id, created) = super::workflow::queue_upload_workflow(
        cfg.clone(),
        username.clone(),
        root_path,
        username,
        album_name,
        upload,
        staging_rel,
        conflict,
        locale.as_str().to_string(),
    )
    .await;
    if !created {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.upload_task_running",
        )));
    }

    let message = tr(locale.as_str(), "response.upload_started");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "task_id": task_id.to_string(),
            "staging_prefix": format!("albums/__uploading/{}", staging_uuid)
        })),
    ))
}
