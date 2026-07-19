use axum::{
    Extension, Json,
    extract::{Path as AxumPath, Query},
};
use function_name::named;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use super::super::album::collect_accessible_ai_scan_targets;
use super::super::auth::Role;
use super::super::common::{AppError, AppState, Token, create_response};
use super::super::i18n::{RequestLocale, tr, trf};
use super::duplicate::{build_exact_duplicate_groups, build_near_duplicate_groups};
use super::models::NormalizedAiIndexRequest;
use super::people::{
    MAX_PERSON_NAME_LEN, merge_people, split_person, update_person,
};
use super::scope::{
    accessible_albums_by_owner, build_person_cover, collect_visible_person_photos,
    count_visible_person_photos, filter_photos_for_scope, normalize_ai_photo_path,
    normalize_ai_text_query, person_is_editable, person_mutation_owner,
};
use super::tasks::AiIndexTask;
use super::AiService;

#[derive(Debug, Clone, Deserialize)]
pub struct AiIndexRequest {
    #[serde(default)]
    album: Option<String>,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    features: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiDuplicatesQuery {
    #[serde(default)]
    album: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    max_distance: Option<u32>,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiSimilarSearchRequest {
    album: String,
    path: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    min_score: Option<f32>,
    /// When set, only score candidates whose path is in this allowlist
    /// (e.g. current filter results).
    #[serde(default)]
    paths: Option<Vec<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiTextSearchRequest {
    text: String,
    #[serde(default)]
    album: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    min_score: Option<f32>,
    /// When set, only score candidates whose path is in this allowlist
    /// (e.g. current filter results).
    #[serde(default)]
    paths: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
struct NormalizedAiSimilarSearchRequest {
    album: String,
    path: String,
    scope: String,
    limit: usize,
    min_score: f32,
    paths: Option<BTreeSet<String>>,
}

#[derive(Debug, Clone)]
struct NormalizedAiTextSearchRequest {
    text: String,
    album: Option<String>,
    scope: String,
    limit: usize,
    min_score: f32,
    paths: Option<BTreeSet<String>>,
}

impl AiIndexRequest {
    fn normalize(self, locale: &str) -> Result<NormalizedAiIndexRequest, AppError> {
        let album = self
            .album
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        let mode = self
            .mode
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("incremental")
            .to_ascii_lowercase();
        if !matches!(mode.as_str(), "incremental" | "rebuild") {
            return Err(AppError::BadRequest(trf(
                locale,
                "error.bad_request",
                &[("value", format!("unsupported ai index mode: {mode}"))],
            )));
        }

        let requested_features = if self.features.is_empty() {
            vec![
                "image".to_string(),
                "duplicates".to_string(),
                "faces".to_string(),
            ]
        } else {
            self.features
        };

        let mut features = BTreeSet::new();
        for feature in requested_features {
            let normalized = feature.trim().to_ascii_lowercase();
            if normalized.is_empty() {
                continue;
            }
            if !matches!(
                normalized.as_str(),
                "image" | "text" | "duplicates" | "faces"
            ) {
                return Err(AppError::BadRequest(trf(
                    locale,
                    "error.bad_request",
                    &[(
                        "value",
                        format!("unsupported ai index feature: {normalized}"),
                    )],
                )));
            }
            features.insert(normalized);
        }

        Ok(NormalizedAiIndexRequest {
            album,
            mode,
            features: features.into_iter().collect(),
        })
    }
}

impl AiDuplicatesQuery {
    fn normalize(self, locale: &str) -> Result<Self, AppError> {
        let limit = self.limit.unwrap_or(20).clamp(1, 100);
        let offset = self.offset.unwrap_or(0);
        let kind = self
            .kind
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("all")
            .to_ascii_lowercase();
        if !matches!(kind.as_str(), "exact" | "near" | "all") {
            return Err(AppError::BadRequest(trf(
                locale,
                "error.bad_request",
                &[("value", format!("unsupported ai duplicates kind: {kind}"))],
            )));
        }
        let max_distance = self.max_distance.unwrap_or(8).clamp(1, 16);
        let album = self
            .album
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        if offset > 10_000 {
            return Err(AppError::BadRequest(trf(
                locale,
                "error.bad_request",
                &[(
                    "value",
                    format!("unsupported ai duplicates offset: {offset}"),
                )],
            )));
        }

        Ok(Self {
            album,
            kind: Some(kind),
            max_distance: Some(max_distance),
            offset: Some(offset),
            limit: Some(limit),
        })
    }
}

impl AiSimilarSearchRequest {
    fn normalize(self, locale: &str) -> Result<NormalizedAiSimilarSearchRequest, AppError> {
        let album = self.album.trim().to_string();
        if album.is_empty() {
            return Err(AppError::BadRequest(trf(
                locale,
                "error.bad_request",
                &[("value", "album is required".to_string())],
            )));
        }
        let path = normalize_ai_photo_path(self.path.as_str()).map_err(|error| {
            AppError::BadRequest(trf(
                locale,
                "error.bad_request",
                &[("value", error)],
            ))
        })?;
        let scope = self
            .scope
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("accessible")
            .to_ascii_lowercase();
        if !matches!(scope.as_str(), "accessible" | "current_album") {
            return Err(AppError::BadRequest(trf(
                locale,
                "error.bad_request",
                &[("value", format!("unsupported ai search scope: {scope}"))],
            )));
        }
        let limit = self.limit.unwrap_or(60).clamp(1, 200);
        let min_score = self.min_score.unwrap_or(0.35).clamp(0.0, 1.0);
        let paths = normalize_search_path_allowlist(self.paths)?;
        Ok(NormalizedAiSimilarSearchRequest {
            album,
            path,
            scope,
            limit,
            min_score,
            paths,
        })
    }
}

impl AiTextSearchRequest {
    fn normalize(self, locale: &str) -> Result<NormalizedAiTextSearchRequest, AppError> {
        let text = normalize_ai_text_query(self.text.as_str()).map_err(|error| {
            AppError::BadRequest(trf(
                locale,
                "error.bad_request",
                &[("value", error)],
            ))
        })?;
        let album = self
            .album
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let scope = self
            .scope
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("accessible")
            .to_ascii_lowercase();
        if !matches!(scope.as_str(), "accessible" | "current_album") {
            return Err(AppError::BadRequest(trf(
                locale,
                "error.bad_request",
                &[("value", format!("unsupported ai search scope: {scope}"))],
            )));
        }
        if scope == "current_album" && album.is_none() {
            return Err(AppError::BadRequest(trf(
                locale,
                "error.bad_request",
                &[("value", "album is required for current_album scope".to_string())],
            )));
        }
        let limit = self.limit.unwrap_or(60).clamp(1, 200);
        // SigLIP2 text↔image scores are calibrated into ~[0,1]; default is top-k
        // (no hard floor) because absolute cosines are far below CLIP-era thresholds.
        let min_score = self.min_score.unwrap_or(0.0).clamp(0.0, 1.0);
        let paths = normalize_search_path_allowlist(self.paths)?;
        Ok(NormalizedAiTextSearchRequest {
            text,
            album,
            scope,
            limit,
            min_score,
            paths,
        })
    }
}

fn normalize_search_path_allowlist(
    paths: Option<Vec<String>>,
) -> Result<Option<BTreeSet<String>>, AppError> {
    let Some(paths) = paths else {
        return Ok(None);
    };
    if paths.is_empty() {
        return Ok(Some(BTreeSet::new()));
    }
    let mut normalized = BTreeSet::new();
    for path in paths {
        let path = normalize_ai_photo_path(path.as_str()).map_err(|error| {
            AppError::BadRequest(trf(
                "en",
                "error.bad_request",
                &[("value", error)],
            ))
        })?;
        normalized.insert(path);
    }
    Ok(Some(normalized))
}


pub async fn api_ai_duplicates(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Query(query): Query<AiDuplicatesQuery>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let query = query.normalize(locale.as_str())?;
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(20);
    let kind = query.kind.as_deref().unwrap_or("all");
    let max_distance = query.max_distance.unwrap_or(8);
    let targets = collect_accessible_ai_scan_targets(
        cfg.as_ref(),
        session.username.as_str(),
        query.album.as_deref(),
        locale.as_str(),
    )
    .await?;

    let mut albums_by_owner = HashMap::<String, BTreeSet<String>>::new();
    for target in targets {
        albums_by_owner
            .entry(target.owner)
            .or_default()
            .insert(target.album_name);
    }

    let mut generations = HashMap::<String, String>::new();
    let mut records = Vec::new();
    for (owner, allowed_albums) in albums_by_owner {
        let Some((generation, owner_records)) =
            cfg.ai_service.store().load_current_photos(owner.as_str())
        else {
            continue;
        };
        generations.insert(owner.clone(), generation);
        let scope = HashMap::from([(owner, allowed_albums)]);
        records.extend(
            filter_photos_for_scope(owner_records.into_values(), &scope)
                .into_iter()
                .filter(|record| record.duplicate_hash_state == "ready"),
        );
    }

    let mut groups = Vec::new();
    if matches!(kind, "exact" | "all") {
        groups.extend(build_exact_duplicate_groups(&records));
    }
    if matches!(kind, "near" | "all") {
        groups.extend(build_near_duplicate_groups(&records, max_distance));
    }
    groups.sort_by(|left, right| {
        right
            .photo_count
            .cmp(&left.photo_count)
            .then(left.kind.cmp(&right.kind))
            .then(left.group_key.cmp(&right.group_key))
    });

    let total = groups.len();
    let groups = groups
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let message = tr(locale.as_str(), "response.success");

    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "generations": generations,
            "offset": offset,
            "limit": limit,
            "kind": kind,
            "max_distance": max_distance,
            "total": total,
            "groups": groups,
        })),
    ))
}

pub async fn api_ai_status(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let enabled = cfg.settings_manager.ai_enabled().await;
    let inference = cfg.ai_service.provider().await;
    let provider_health = inference.health().await;
    let owner_status = cfg.ai_service.store().owner_status(&session.username);
    let message = tr(locale.as_str(), "response.success");
    let provider = if session.role == Role::Admin {
        json!({
            "kind": provider_health.kind,
            "backend": provider_health.backend,
            "healthy": provider_health.healthy,
            "model_manifest_present": provider_health.model_manifest_present,
            "model_root": provider_health.model_root,
            "revision": provider_health.revision,
            "reason": provider_health.reason,
            "store_root": cfg.ai_service.store().root().display().to_string(),
            "remote_token_configured": std::env::var("AI_REMOTE_TOKEN")
                .ok()
                .map(|value| !value.trim().is_empty())
                .unwrap_or(false),
        })
    } else {
        json!({
            "kind": provider_health.kind,
            "healthy": provider_health.healthy,
            "revision": provider_health.revision,
        })
    };

    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "enabled": enabled,
            "capabilities": inference.capabilities(),
            "provider": provider,
            "owner_index": owner_status,
            "limits": {
                "concurrency": cfg.ai_limiter.limit(),
            }
        })),
    ))
}

#[named]
pub async fn api_ai_index(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(payload): Json<AiIndexRequest>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let request = payload.normalize(locale.as_str())?;
    if request.album.is_some() {
        let targets = collect_accessible_ai_scan_targets(
            cfg.as_ref(),
            session.username.as_str(),
            request.album.as_deref(),
            locale.as_str(),
        )
        .await?;
        if targets
            .iter()
            .any(|target| target.owner != session.username)
        {
            return Err(AppError::Forbidden(tr(
                locale.as_str(),
                "error.ai_index_owner_only",
            )));
        }
    }
    let target = request
        .album
        .clone()
        .unwrap_or_else(|| "all owned albums".to_string());
    let task = Arc::new(AiIndexTask::new(
        trf(locale.as_str(), "task.ai_index", &[("target", target)]),
        session.username,
        cfg.clone(),
        locale.as_str().to_string(),
        request,
    ));
    let (task_id, created) = cfg.task_mgr.add_task_if_absent(task).await;
    let message = tr(locale.as_str(), "response.ai_index_task_created");

    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "task_id": task_id.to_string(),
            "created": created,
        })),
    ))
}


pub async fn api_ai_search_similar(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(payload): Json<AiSimilarSearchRequest>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let inference = cfg.ai_service.provider().await;
    if !inference.capabilities().image_search {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_image_search_unavailable",
        )));
    }

    let request = payload.normalize(locale.as_str())?;
    let source_targets = collect_accessible_ai_scan_targets(
        cfg.as_ref(),
        session.username.as_str(),
        Some(request.album.as_str()),
        locale.as_str(),
    )
    .await?;
    let Some(source_target) = source_targets.into_iter().next() else {
        return Err(AppError::Forbidden(tr(
            locale.as_str(),
            "error.album_access_denied",
        )));
    };

    let Some(source_index) = cfg
        .ai_service
        .store()
        .load_current_index(source_target.owner.as_str())
    else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_not_indexed",
        )));
    };

    let source_record = source_index
        .photos
        .values()
        .find(|record| record.album == source_target.album_name && record.path == request.path)
        .cloned();
    let Some(source_record) = source_record else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_not_indexed",
        )));
    };
    let Some(query_vector) = source_index
        .embeddings
        .get(&source_record.stable_key)
        .cloned()
    else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_not_indexed",
        )));
    };

    let search_album = if request.scope == "current_album" {
        Some(request.album.as_str())
    } else {
        None
    };
    let search_targets = collect_accessible_ai_scan_targets(
        cfg.as_ref(),
        session.username.as_str(),
        search_album,
        locale.as_str(),
    )
    .await?;

    let mut albums_by_owner = HashMap::<String, BTreeSet<String>>::new();
    for target in search_targets {
        albums_by_owner
            .entry(target.owner)
            .or_default()
            .insert(target.album_name);
    }

    let mut candidates = Vec::new();
    let mut generations = HashMap::<String, String>::new();
    for (owner, allowed_albums) in albums_by_owner {
        let Some(loaded) = cfg.ai_service.store().load_current_index(owner.as_str()) else {
            continue;
        };
        generations.insert(owner.clone(), loaded.generation.clone());
        for (stable_key, record) in loaded.photos {
            if record.owner != owner || !allowed_albums.contains(&record.album) {
                continue;
            }
            if let Some(ref allowed_paths) = request.paths {
                if !allowed_paths.contains(&record.path) {
                    continue;
                }
            }
            if record.image_embedding_state != "ready" {
                continue;
            }
            let Some(vector) = loaded.embeddings.get(&stable_key).cloned() else {
                continue;
            };
            candidates.push((record, vector));
        }
    }

    let results = AiService::search_similar_in_records(
        &query_vector,
        &candidates,
        source_record.stable_id.as_str(),
        request.min_score,
        request.limit,
    );
    let message = tr(locale.as_str(), "response.success");

    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "query": {
                "kind": "image",
                "owner": source_record.owner,
                "album": source_record.album,
                "path": source_record.path,
                "scope": request.scope,
                "min_score": request.min_score,
            },
            "generations": generations,
            "total": results.len(),
            "results": results,
        })),
    ))
}

#[named]
pub async fn api_ai_search_text(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(payload): Json<AiTextSearchRequest>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let inference = cfg.ai_service.provider().await;
    if !inference.capabilities().text_search {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_text_search_unavailable",
        )));
    }

    let request = payload.normalize(locale.as_str())?;
    if let Some(album) = request.album.as_deref() {
        let targets = collect_accessible_ai_scan_targets(
            cfg.as_ref(),
            session.username.as_str(),
            Some(album),
            locale.as_str(),
        )
        .await?;
        if targets.is_empty() {
            return Err(AppError::Forbidden(tr(
                locale.as_str(),
                "error.album_access_denied",
            )));
        }
    }

    let query_results = inference
        .embed_texts(&[request.text.clone()])
        .await
        .map_err(|error| {
            AppError::BadRequest(trf(
                locale.as_str(),
                "error.bad_request",
                &[("value", error)],
            ))
        })?;
    let query_vector = match query_results.into_iter().next() {
        Some(Ok(vector)) => vector,
        Some(Err(error)) => {
            return Err(AppError::BadRequest(trf(
                locale.as_str(),
                "error.bad_request",
                &[("value", error.to_string())],
            )));
        }
        None => {
            return Err(AppError::BadRequest(tr(
                locale.as_str(),
                "error.ai_text_search_unavailable",
            )));
        }
    };

    let search_album = if request.scope == "current_album" {
        request.album.as_deref()
    } else {
        None
    };
    let search_targets = collect_accessible_ai_scan_targets(
        cfg.as_ref(),
        session.username.as_str(),
        search_album,
        locale.as_str(),
    )
    .await?;

    let mut albums_by_owner = HashMap::<String, BTreeSet<String>>::new();
    for target in search_targets {
        albums_by_owner
            .entry(target.owner)
            .or_default()
            .insert(target.album_name);
    }

    let mut candidates = Vec::new();
    let mut generations = HashMap::<String, String>::new();
    let mut any_index = false;
    for (owner, allowed_albums) in albums_by_owner {
        let Some(loaded) = cfg.ai_service.store().load_current_index(owner.as_str()) else {
            continue;
        };
        any_index = true;
        generations.insert(owner.clone(), loaded.generation.clone());
        for (stable_key, record) in loaded.photos {
            if record.owner != owner || !allowed_albums.contains(&record.album) {
                continue;
            }
            if let Some(ref allowed_paths) = request.paths {
                if !allowed_paths.contains(&record.path) {
                    continue;
                }
            }
            let embedding = if record.image_embedding_state == "ready" {
                loaded.embeddings.get(&stable_key).cloned()
            } else {
                None
            };
            candidates.push((record, embedding));
        }
    }

    if !any_index {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_not_indexed",
        )));
    }

    let results = AiService::search_text_in_records(
        request.text.as_str(),
        &query_vector,
        &candidates,
        request.min_score,
        request.limit,
    );
    let message = tr(locale.as_str(), "response.success");

    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "query": {
                "kind": "text",
                "text": request.text,
                "album": request.album,
                "scope": request.scope,
                "min_score": request.min_score,
            },
            "generations": generations,
            "total": results.len(),
            "results": results,
        })),
    ))
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiPeopleQuery {
    #[serde(default)]
    include_hidden: Option<bool>,
    #[serde(default)]
    person_id: Option<String>,
    /// When set, only return people who have visible faces in this album.
    #[serde(default)]
    album: Option<String>,
    #[serde(default)]
    offset: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiPeoplePatchRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    hidden: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiPeopleMergeRequest {
    target_person_id: String,
    source_person_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiPeopleSplitRequest {
    source_person_id: String,
    face_ids: Vec<String>,
    #[serde(default)]
    new_name: Option<String>,
}


#[named]
pub async fn api_ai_people(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Query(query): Query<AiPeopleQuery>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let inference = cfg.ai_service.provider().await;
    if !inference.capabilities().faces {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_faces_unavailable",
        )));
    }

    let include_hidden = query.include_hidden.unwrap_or(false);
    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(100).clamp(1, 200);

    if let Some(person_id) = query
        .person_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return api_ai_people_photos_inner(
            locale,
            &session.username,
            &cfg,
            person_id,
            offset,
            limit,
        )
        .await;
    }

    let targets = collect_accessible_ai_scan_targets(
        cfg.as_ref(),
        session.username.as_str(),
        query
            .album
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
        locale.as_str(),
    )
    .await?;
    let albums_by_owner = accessible_albums_by_owner(&targets);

    let mut people = Vec::new();
    for (owner, allowed_albums) in albums_by_owner {
        let Some(index) = cfg.ai_service.store().load_current_index(owner.as_str()) else {
            continue;
        };
        for person in index.people.values() {
            if person.hidden && !include_hidden {
                continue;
            }
            let visible_photo_count =
                count_visible_person_photos(person.id.as_str(), &index, &allowed_albums);
            if visible_photo_count == 0 {
                continue;
            }
            people.push(json!({
                "id": person.id,
                "name": person.name,
                "hidden": person.hidden,
                "visible_photo_count": visible_photo_count,
                "cover": build_person_cover(person, &index, &allowed_albums),
                "editable": person_is_editable(owner.as_str(), session.username.as_str()),
                "owner": owner,
            }));
        }
    }

    people.sort_by(|left, right| {
        left.get("name")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .cmp(right.get("name").and_then(|value| value.as_str()).unwrap_or_default())
            .then_with(|| {
                left.get("id")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .cmp(right.get("id").and_then(|value| value.as_str()).unwrap_or_default())
            })
    });

    let total_count = people.len();
    let people = people.into_iter().skip(offset).take(limit).collect::<Vec<_>>();
    let message = tr(locale.as_str(), "response.success");

    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "offset": offset,
            "limit": limit,
            "total_count": total_count,
            "people": people,
        })),
    ))
}

async fn api_ai_people_photos_inner(
    locale: RequestLocale,
    username: &str,
    cfg: &Arc<AppState>,
    person_id: &str,
    offset: usize,
    limit: usize,
) -> Result<Json<Value>, AppError> {
    let targets = collect_accessible_ai_scan_targets(cfg.as_ref(), username, None, locale.as_str())
        .await?;
    let albums_by_owner = accessible_albums_by_owner(&targets);

    for (owner, allowed_albums) in albums_by_owner {
        let Some(index) = cfg.ai_service.store().load_current_index(owner.as_str()) else {
            continue;
        };
        if !index.people.contains_key(person_id) {
            continue;
        }
        let photos = collect_visible_person_photos(person_id, &index, &allowed_albums);
        if photos.is_empty() {
            return Err(AppError::NotFound(tr(
                locale.as_str(),
                "error.ai_person_not_visible",
            )));
        }
        let total_count = photos.len();
        let photos = photos.into_iter().skip(offset).take(limit).collect::<Vec<_>>();
        let message = tr(locale.as_str(), "response.success");
        return Ok(create_response(
            true,
            &message,
            0,
            Some(json!({
                "person_id": person_id,
                "owner": owner,
                "offset": offset,
                "limit": limit,
                "total_count": total_count,
                "photos": photos,
            })),
        ));
    }

    Err(AppError::NotFound(tr(
        locale.as_str(),
        "error.ai_person_not_found",
    )))
}

#[named]
pub async fn api_ai_people_photos(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    AxumPath(person_id): AxumPath<String>,
    Query(query): Query<AiPeopleQuery>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let inference = cfg.ai_service.provider().await;
    if !inference.capabilities().faces {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_faces_unavailable",
        )));
    }

    api_ai_people_photos_inner(
        locale,
        session.username.as_str(),
        &cfg,
        person_id.trim(),
        query.offset.unwrap_or(0),
        query.limit.unwrap_or(100).clamp(1, 200),
    )
    .await
}

#[named]
pub async fn api_ai_people_patch(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    AxumPath(person_id): AxumPath<String>,
    Json(payload): Json<AiPeoplePatchRequest>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    if payload.name.is_none() && payload.hidden.is_none() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_person_update_empty",
        )));
    }

    if payload
        .name
        .as_deref()
        .map(|value| value.trim().chars().count() > MAX_PERSON_NAME_LEN)
        .unwrap_or(false)
    {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_person_name_invalid",
        )));
    }

    let owner = person_mutation_owner(session.username.as_str());
    let Some(index) = cfg.ai_service.store().load_current_index(owner) else {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_not_indexed",
        )));
    };
    if !index.people.contains_key(person_id.trim()) {
        return Err(AppError::NotFound(tr(
            locale.as_str(),
            "error.ai_person_not_found",
        )));
    }

    let inference = cfg.ai_service.provider().await;
    let provider_health = inference.health().await;
    let name = payload.name.as_deref();
    let hidden = payload.hidden;
    cfg.ai_service
        .store()
        .commit_people_mutation(
            owner,
            inference.capabilities(),
            &provider_health,
            |people, _assignments| {
                update_person(people, person_id.trim(), name, hidden)
                    .map_err(|error| error.to_string())
            },
        )
        .map_err(AppError::BadRequest)?;

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "person_id": person_id.trim(),
            "updated": true,
        })),
    ))
}

#[named]
pub async fn api_ai_people_merge(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(payload): Json<AiPeopleMergeRequest>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let owner = person_mutation_owner(session.username.as_str());
    if payload.source_person_ids.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_person_merge_empty",
        )));
    }

    let inference = cfg.ai_service.provider().await;
    let provider_health = inference.health().await;
    cfg.ai_service
        .store()
        .commit_people_mutation(
            owner,
            inference.capabilities(),
            &provider_health,
            |people, assignments| {
                merge_people(
                    people,
                    assignments,
                    payload.target_person_id.trim(),
                    &payload
                        .source_person_ids
                        .iter()
                        .map(|value| value.trim().to_string())
                        .collect::<Vec<_>>(),
                )
                .map_err(|error| error.to_string())
            },
        )
        .map_err(AppError::BadRequest)?;

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "target_person_id": payload.target_person_id.trim(),
            "merged": payload.source_person_ids.len(),
        })),
    ))
}

#[named]
pub async fn api_ai_people_split(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(payload): Json<AiPeopleSplitRequest>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };

    let owner = person_mutation_owner(session.username.as_str());
    if payload.face_ids.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_person_split_empty",
        )));
    }

    let inference = cfg.ai_service.provider().await;
    let provider_health = inference.health().await;
    let mut new_person_id = String::new();
    cfg
        .ai_service
        .store()
        .commit_people_mutation(
            owner,
            inference.capabilities(),
            &provider_health,
            |people, assignments| {
                new_person_id = split_person(
                    people,
                    assignments,
                    payload.source_person_id.trim(),
                    &payload
                        .face_ids
                        .iter()
                        .map(|value| value.trim().to_string())
                        .collect::<Vec<_>>(),
                    payload.new_name.as_deref(),
                )?;
                Ok(())
            },
        )
        .map_err(AppError::BadRequest)?;

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "source_person_id": payload.source_person_id.trim(),
            "new_person_id": new_person_id,
            "face_count": payload.face_ids.len(),
        })),
    ))
}

