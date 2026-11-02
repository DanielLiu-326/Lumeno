//! Album-scoped face management APIs (`/api/ai/albums/{album}/faces…`).

use axum::{
    Extension, Json,
    body::Body,
    extract::{Path as AxumPath, Query},
    http::{HeaderMap, HeaderValue},
    response::Response,
};
use function_name::named;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use super::super::album::api::{ResolvedAlbum, resolve_album_access};
use super::super::album::ensure_cached_face_crop;
use super::super::album::helpers::{albums_root, ensure_album_cache_id};
use super::super::album::media::is_readable_face_crop;
use super::super::common::{AppError, AppState, Token, create_response};
use super::super::constants::{FACE_CROP_SIZE, clamp_cluster_similarity};
use super::super::i18n::{RequestLocale, tr};
use super::album_cache::{
    AlbumAiCache, ClustersCacheFile, DEFAULT_PERSON_MATCH_MIN_SCORE, FaceHintsFile, FaceIndexEntry,
    MAX_PERSON_MATCH_SUGGESTIONS, appearance_identity_key, apply_person_match_suggestions,
    decode_f32_vector_b64, encode_f32_vector_b64, face_id, hints_from_clusters,
    sort_suggestion_groups,
};
use super::faces_config::{
    FacesConfig, PersonAppearance, PersonCenter, collect_named_person_centers,
    load_faces_config_async, normalize_person_name, save_faces_config_async,
};

#[derive(Debug, Clone, Deserialize)]
pub struct AlbumFacePatchRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    hidden: Option<bool>,
    /// `null` clears cover; object sets cover; omitted leaves unchanged.
    #[serde(default)]
    cover: Option<Option<CoverInput>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CoverInput {
    source: String,
    box_xywh: [f32; 4],
}

#[derive(Debug, Clone, Deserialize)]
pub struct AdoptClusterRequest {
    cluster_id: String,
    name: String,
    #[serde(default)]
    target_person_id: Option<String>,
    #[serde(default)]
    center_label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MergeClustersRequest {
    target_person_id: String,
    cluster_id: String,
    #[serde(default)]
    center_label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SplitPersonRequest {
    source_person_id: String,
    #[serde(default)]
    appearance_keys: Vec<String>,
    #[serde(default)]
    face_ids: Vec<String>,
    #[serde(default)]
    new_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RemoveCenterRequest {
    person_id: String,
    center_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FaceCropQuery {
    /// Preferred: deterministic face id from index.
    #[serde(default)]
    face_id: Option<String>,
    /// Legacy: album-relative source + box.
    #[serde(default)]
    source: Option<String>,
    #[serde(default, rename = "box")]
    box_xywh: Option<String>,
    #[serde(default)]
    size: Option<u32>,
}

async fn resolve_owned_album(
    cfg: &AppState,
    token: &Token,
    album: &str,
    locale: &str,
) -> Result<(String, ResolvedAlbum, PathBuf, String), AppError> {
    let Some(session) = cfg.get_session_info(token).await else {
        return Err(AppError::unauthorized(locale));
    };
    let user_root = cfg.user_root_path(&session.username);
    let resolved = resolve_album_access(cfg, &session.username, &user_root, album, locale).await?;
    if !resolved.is_owner {
        return Err(AppError::Forbidden(tr(locale, "error.ai_index_owner_only")));
    }
    let album_dir = albums_root(&resolved.root_path).join(&resolved.album_name);
    let cache_id = ensure_album_cache_id(&album_dir).await?;
    Ok((session.username, resolved, album_dir, cache_id))
}

fn face_lookup(clusters: &ClustersCacheFile) -> HashMap<String, FaceIndexEntry> {
    clusters
        .faces
        .iter()
        .map(|entry| (entry.face_id.clone(), entry.clone()))
        .collect()
}

fn filter_named_faces(
    mut clusters: ClustersCacheFile,
    assigned: &std::collections::HashSet<String>,
) -> ClustersCacheFile {
    let keep: std::collections::HashSet<String> = clusters
        .faces
        .iter()
        .filter(|e| !assigned.contains(&appearance_identity_key(&e.source, &e.box_xywh)))
        .map(|e| e.face_id.clone())
        .collect();
    clusters.faces.retain(|e| keep.contains(&e.face_id));
    for cluster in &mut clusters.clusters {
        cluster.face_ids.retain(|id| keep.contains(id));
        if let Some(rep) = &cluster.representative_face_id
            && !keep.contains(rep)
        {
            cluster.representative_face_id = cluster.face_ids.first().cloned();
        }
    }
    clusters.clusters.retain(|c| !c.face_ids.is_empty());
    clusters.noise_face_ids.retain(|id| keep.contains(id));
    clusters
}

/// Map face_id → source/box for opening photos from lightweight suggestion groups.
fn face_catalog_payload(clusters: &ClustersCacheFile) -> Value {
    let mut catalog = serde_json::Map::new();
    for entry in &clusters.faces {
        catalog.insert(
            entry.face_id.clone(),
            json!({
                "source": entry.source,
                "box_xywh": entry.box_xywh,
            }),
        );
    }
    Value::Object(catalog)
}

/// Lightweight suggestion groups: only ids (clients load crops by face_id; hints via `/face-hints`).
fn groups_payload(clusters: &ClustersCacheFile) -> Value {
    let mut groups: Vec<Value> = clusters
        .clusters
        .iter()
        .map(|c| {
            json!({
                "group_id": c.cluster_id,
                "cluster_id": c.cluster_id,
                "face_ids": c.face_ids,
                "face_count": c.face_ids.len(),
                "representative_face_id": c.representative_face_id,
                "synthetic": false,
            })
        })
        .collect();

    let in_group: std::collections::HashSet<&str> = clusters
        .clusters
        .iter()
        .flat_map(|c| c.face_ids.iter().map(String::as_str))
        .collect();
    let mut noise = clusters.noise_face_ids.clone();
    for entry in &clusters.faces {
        if !in_group.contains(entry.face_id.as_str()) && !noise.iter().any(|n| n == &entry.face_id)
        {
            noise.push(entry.face_id.clone());
        }
    }
    noise.sort();
    for fid in &noise {
        groups.push(json!({
            "group_id": format!("noise-{fid}"),
            "cluster_id": format!("noise-{fid}"),
            "face_ids": [fid],
            "face_count": 1,
            "representative_face_id": fid,
            "synthetic": true,
        }));
    }

    sort_suggestion_groups(&mut groups);

    json!({
        "group_count": groups.len(),
        "groups": groups,
        // Compat aliases for existing frontend until it migrates.
        "clusters": groups,
        "noise_face_ids": noise,
        "algorithm": clusters.algorithm,
        "params": clusters.params,
        "unassigned_count": noise.len(),
        "face_catalog": face_catalog_payload(clusters),
    })
}

fn people_payload(config: &FacesConfig, album_name: &str, owner: &str) -> Vec<Value> {
    config
        .people
        .iter()
        .map(|person| {
            let mut appearances = Vec::new();
            let centers: Vec<Value> = person
                .centers
                .iter()
                .map(|c| {
                    for a in &c.appearances {
                        let mut row = json!({
                            "source": a.source,
                            "box_xywh": a.box_xywh,
                            "center_id": c.id,
                            "identity_key": a.identity_key(),
                        });
                        if let Some(face_id) = a.face_id.as_deref() {
                            row["face_id"] = json!(face_id);
                        }
                        appearances.push(row);
                    }
                    json!({
                        "id": c.id,
                        "label": c.label,
                        "appearance_count": c.appearances.len(),
                    })
                })
                .collect();
            let mut payload = json!({
                "id": person.id,
                "name": person.name,
                "hidden": person.hidden,
                "center_count": person.centers.len(),
                "appearance_count": person.appearance_count(),
                "visible_photo_count": person.visible_photo_count(),
                "centers": centers,
                "appearances": appearances,
                "editable": true,
            });
            if let Some(cover) = person_cover_json(person, album_name, owner) {
                payload["cover"] = cover;
            }
            payload
        })
        .collect()
}

fn person_cover_json(
    person: &super::faces_config::AlbumPerson,
    album_name: &str,
    owner: &str,
) -> Option<Value> {
    let cover = person.cover.as_ref()?;
    let rel = cover
        .source
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string();
    if rel.is_empty() {
        return None;
    }
    let mut payload = json!({
        "path": format!("albums/{album_name}/{rel}"),
        "source": rel,
        "face_box": cover.box_xywh,
        "owner": owner,
    });
    if let Some(face_id) = cover.face_id.as_deref() {
        payload["face_id"] = json!(face_id);
    }
    Some(payload)
}

async fn recompute_match_suggestions(
    clusters: &mut ClustersCacheFile,
    config: &FacesConfig,
    cache: &AlbumAiCache,
) {
    let catalog = collect_named_person_centers(config);
    if catalog.is_empty() {
        clusters.noise_match_suggestions.clear();
        for cluster in &mut clusters.clusters {
            cluster.match_suggestions.clear();
        }
        return;
    }

    let needed_ids = face_ids_for_match_queries(clusters);
    let embedding_cache = load_match_embeddings_by_photo(cache, clusters, &needed_ids).await;

    apply_person_match_suggestions(
        clusters,
        &catalog,
        |face_id| embedding_cache.get(face_id).cloned(),
        DEFAULT_PERSON_MATCH_MIN_SCORE,
        MAX_PERSON_MATCH_SUGGESTIONS,
    );
}

/// Recompute merge hints in memory and persist to `face_hint.json` (never mutates `face_index.json`).
async fn refresh_face_hints_file(album_dir: &Path, cache: &AlbumAiCache) -> Result<(), String> {
    let Some(mut clusters) = cache.read_face_index_async().await else {
        return Ok(());
    };
    let config = load_faces_config_async(album_dir).await;
    recompute_match_suggestions(&mut clusters, &config, cache).await;
    cache
        .write_face_hints_async(&hints_from_clusters(&clusters))
        .await
}

fn visible_group_ids(clusters: &ClustersCacheFile) -> HashSet<String> {
    let mut ids: HashSet<String> = clusters
        .clusters
        .iter()
        .map(|c| c.cluster_id.clone())
        .collect();
    let in_group: HashSet<&str> = clusters
        .clusters
        .iter()
        .flat_map(|c| c.face_ids.iter().map(String::as_str))
        .collect();
    for entry in &clusters.faces {
        if !in_group.contains(entry.face_id.as_str())
            && !clusters.noise_face_ids.iter().any(|n| n == &entry.face_id)
        {
            ids.insert(format!("noise-{}", entry.face_id));
        }
    }
    for fid in &clusters.noise_face_ids {
        ids.insert(format!("noise-{fid}"));
    }
    ids
}

async fn build_face_hints_payload(album_dir: &Path, cache: &AlbumAiCache) -> Value {
    let config = load_faces_config_async(album_dir).await;
    let assigned = config.assigned_appearance_keys();
    let clusters = cache
        .read_face_index_async()
        .await
        .unwrap_or_else(ClustersCacheFile::empty);
    let clusters = filter_named_faces(clusters, &assigned);
    let visible = visible_group_ids(&clusters);
    let hints = cache
        .read_face_hints_async()
        .await
        .unwrap_or_else(FaceHintsFile::empty);
    let groups: HashMap<String, _> = hints
        .group_hints
        .into_iter()
        .filter(|(group_id, suggestions)| visible.contains(group_id) && !suggestions.is_empty())
        .collect();
    json!({ "groups": groups })
}

fn log_match_refresh_error(context: &str, error: String) {
    eprintln!("face index match refresh failed ({context}): {error}");
}

/// Face ids whose embeddings are consulted by [`apply_person_match_suggestions`].
fn face_ids_for_match_queries(clusters: &ClustersCacheFile) -> HashSet<String> {
    let mut needed: HashSet<String> = clusters.noise_face_ids.iter().cloned().collect();
    for cluster in &clusters.clusters {
        if cluster.center_b64.as_deref().is_some_and(|b64| {
            decode_f32_vector_b64(b64, None)
                .map(|vector| !vector.is_empty())
                .unwrap_or(false)
        }) {
            continue;
        }
        if let Some(rep) = &cluster.representative_face_id {
            needed.insert(rep.clone());
        } else if let Some(first) = cluster.face_ids.first() {
            needed.insert(first.clone());
        }
    }
    needed
}

/// Load embeddings only for suggestion queries, one `faces/{photo_key}.json` read per photo.
async fn load_match_embeddings_by_photo(
    cache: &AlbumAiCache,
    clusters: &ClustersCacheFile,
    face_ids: &HashSet<String>,
) -> HashMap<String, Vec<f32>> {
    if face_ids.is_empty() {
        return HashMap::new();
    }
    let lookup: HashMap<&str, &FaceIndexEntry> = clusters
        .faces
        .iter()
        .map(|entry| (entry.face_id.as_str(), entry))
        .collect();
    let mut by_photo: HashMap<String, Vec<&FaceIndexEntry>> = HashMap::new();
    for face_id in face_ids {
        let Some(entry) = lookup.get(face_id.as_str()) else {
            continue;
        };
        if entry.photo_key.is_empty() {
            continue;
        }
        by_photo
            .entry(entry.photo_key.clone())
            .or_default()
            .push(*entry);
    }

    let mut embedding_cache = HashMap::new();
    for (photo_key, entries) in by_photo {
        let Some(file) = cache.read_face_photo_async(&photo_key).await else {
            continue;
        };
        for entry in entries {
            let Some(face) = file.faces.iter().find(|f| {
                f.face_id == entry.face_id
                    || face_id(&entry.stable_key, &f.box_xywh) == entry.face_id
            }) else {
                continue;
            };
            if let Ok(vector) = decode_f32_vector_b64(&face.embedding_b64, Some(face.dimension)) {
                embedding_cache.insert(entry.face_id.clone(), vector);
            }
        }
    }
    embedding_cache
}

async fn build_faces_get_payload(
    album_dir: &Path,
    cache: &AlbumAiCache,
    album_name: &str,
    owner: &str,
    cache_id: &str,
    editable: bool,
) -> Value {
    let config = load_faces_config_async(album_dir).await;
    let assigned = config.assigned_appearance_keys();
    let clusters = cache
        .read_face_index_async()
        .await
        .unwrap_or_else(ClustersCacheFile::empty);
    let clusters = filter_named_faces(clusters, &assigned);
    json!({
        "album": album_name,
        "owner": owner,
        "cache_id": cache_id,
        "editable": editable,
        "cluster_similarity": clamp_cluster_similarity(config.cluster_similarity),
        "people": people_payload(&config, album_name, owner),
        "suggestions": groups_payload(&clusters),
    })
}

fn parse_box_xywh(raw: &str) -> Result<[f32; 4], String> {
    let parts: Vec<_> = raw.split(',').map(str::trim).collect();
    if parts.len() != 4 {
        return Err("box must be x,y,w,h".to_string());
    }
    let mut out = [0.0_f32; 4];
    for (i, part) in parts.iter().enumerate() {
        out[i] = part
            .parse::<f32>()
            .map_err(|_| format!("invalid box component: {part}"))?;
    }
    Ok(out)
}

async fn load_embedding_b64(cache: &AlbumAiCache, entry: &FaceIndexEntry) -> Option<String> {
    if entry.photo_key.is_empty() {
        return None;
    }
    let file = cache.read_face_photo_async(&entry.photo_key).await?;
    file.faces
        .into_iter()
        .find(|f| {
            f.face_id == entry.face_id || face_id(&entry.stable_key, &f.box_xywh) == entry.face_id
        })
        .map(|f| f.embedding_b64)
}

fn appearances_for_ids(
    face_ids: &[String],
    lookup: &HashMap<String, FaceIndexEntry>,
) -> Vec<PersonAppearance> {
    face_ids
        .iter()
        .filter_map(|fid| {
            let entry = lookup.get(fid)?;
            Some(PersonAppearance {
                source: entry.source.clone(),
                box_xywh: entry.box_xywh,
                face_id: Some(fid.clone()),
            })
        })
        .collect()
}

fn resolve_group_faces(
    clusters: &ClustersCacheFile,
    lookup: &HashMap<String, FaceIndexEntry>,
    cluster_id: &str,
) -> Result<(Vec<String>, String, Option<String>), String> {
    if let Some(face_id) = cluster_id.strip_prefix("noise-") {
        if !lookup.contains_key(face_id) {
            return Err(format!("noise face {face_id} not found"));
        }
        return Ok((vec![face_id.to_string()], face_id.to_string(), None));
    }
    let cluster = clusters
        .clusters
        .iter()
        .find(|c| c.cluster_id == cluster_id)
        .ok_or_else(|| format!("cluster {cluster_id} not found"))?;
    let rep_id = cluster
        .representative_face_id
        .clone()
        .or_else(|| cluster.face_ids.first().cloned())
        .ok_or_else(|| "cluster has no faces".to_string())?;
    Ok((cluster.face_ids.clone(), rep_id, cluster.center_b64.clone()))
}

async fn adopt_or_append(
    config: &mut FacesConfig,
    clusters: &ClustersCacheFile,
    lookup: &HashMap<String, FaceIndexEntry>,
    cache: &AlbumAiCache,
    cluster_id: &str,
    name: Option<&str>,
    target_person_id: Option<&str>,
    center_label: Option<String>,
) -> Result<String, String> {
    let (face_ids, rep_id, center_b64) = resolve_group_faces(clusters, lookup, cluster_id)?;
    let rep = lookup
        .get(&rep_id)
        .ok_or_else(|| format!("representative face {rep_id} missing"))?;
    let center_b64 = match center_b64 {
        Some(v) => Some(v),
        None => load_embedding_b64(cache, rep).await,
    };
    let center = PersonCenter {
        id: Uuid::new_v4().to_string(),
        label: center_label,
        embedding_ref: None,
        center_b64,
        appearances: Vec::new(),
    };
    let appearances = appearances_for_ids(&face_ids, lookup);
    if let Some(person_id) = target_person_id {
        config.append_cluster_as_center(person_id, appearances, center)?;
        Ok(person_id.to_string())
    } else {
        let name = name.ok_or_else(|| "name is required for new person".to_string())?;
        config.adopt_cluster_as_person(name, appearances, center)
    }
}

#[named]
pub async fn api_album_faces_get(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    AxumPath(album): AxumPath<String>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let user_root = cfg.user_root_path(&session.username);
    let resolved = resolve_album_access(
        cfg.as_ref(),
        &session.username,
        &user_root,
        album.trim(),
        locale.as_str(),
    )
    .await?;
    let album_dir = albums_root(&resolved.root_path).join(&resolved.album_name);
    let cache_id = ensure_album_cache_id(&album_dir).await?;
    let cache = AlbumAiCache::new(cfg.ai_service.store().root(), &resolved.owner, &cache_id);
    let data = build_faces_get_payload(
        &album_dir,
        &cache,
        &resolved.album_name,
        &resolved.owner,
        &cache_id,
        resolved.is_owner,
    )
    .await;

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(true, &message, 0, Some(data)))
}

#[derive(Debug, Deserialize)]
pub struct ReclusterRequest {
    similarity: f32,
}

/// Save this album's clustering similarity and rebuild unnamed suggestion groups.
#[named]
pub async fn api_album_faces_recluster(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    AxumPath(album): AxumPath<String>,
    Json(payload): Json<ReclusterRequest>,
) -> Result<Json<Value>, AppError> {
    if !payload.similarity.is_finite() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.cluster_similarity_invalid",
        )));
    }
    let (_user, resolved, album_dir, cache_id) =
        resolve_owned_album(cfg.as_ref(), &token, album.trim(), locale.as_str()).await?;
    let mut config = load_faces_config_async(&album_dir).await;
    config.cluster_similarity = clamp_cluster_similarity(payload.similarity);
    let cache = AlbumAiCache::new(cfg.ai_service.store().root(), &resolved.owner, &cache_id);
    super::pipelines::rebuild_album_face_clusters(&cache, &config)
        .await
        .map_err(AppError::BadRequest)?;
    save_faces_config_async(&album_dir, &config)
        .await
        .map_err(AppError::BadRequest)?;
    let data = build_faces_get_payload(
        &album_dir,
        &cache,
        &resolved.album_name,
        &resolved.owner,
        &cache_id,
        resolved.is_owner,
    )
    .await;
    let message = tr(locale.as_str(), "response.faces_reclustered");
    Ok(create_response(true, &message, 0, Some(data)))
}

/// Merge hints for visible suggestion groups (`face_hint.json`, filtered by assigned faces).
#[named]
pub async fn api_album_face_hints_get(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    AxumPath(album): AxumPath<String>,
) -> Result<Json<Value>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let user_root = cfg.user_root_path(&session.username);
    let resolved = resolve_album_access(
        cfg.as_ref(),
        &session.username,
        &user_root,
        album.trim(),
        locale.as_str(),
    )
    .await?;
    let album_dir = albums_root(&resolved.root_path).join(&resolved.album_name);
    let cache_id = ensure_album_cache_id(&album_dir).await?;
    let cache = AlbumAiCache::new(cfg.ai_service.store().root(), &resolved.owner, &cache_id);
    let data = build_face_hints_payload(&album_dir, &cache).await;
    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(true, &message, 0, Some(data)))
}

#[named]
pub async fn api_album_faces_patch(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    AxumPath((album, person_id)): AxumPath<(String, String)>,
    Json(payload): Json<AlbumFacePatchRequest>,
) -> Result<Json<Value>, AppError> {
    let (_user, resolved, album_dir, cache_id) =
        resolve_owned_album(cfg.as_ref(), &token, album.trim(), locale.as_str()).await?;
    if payload.name.is_none() && payload.hidden.is_none() && payload.cover.is_none() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_person_update_empty",
        )));
    }
    let mut config = load_faces_config_async(&album_dir).await;
    if let Some(name) = payload.name.as_deref() {
        config
            .rename_person(person_id.trim(), name)
            .map_err(AppError::BadRequest)?;
    }
    if let Some(hidden) = payload.hidden {
        config
            .set_hidden(person_id.trim(), hidden)
            .map_err(AppError::BadRequest)?;
    }
    if let Some(cover) = payload.cover {
        if let Some(input) = cover {
            config
                .set_cover(
                    person_id.trim(),
                    &PersonAppearance {
                        source: input.source.clone(),
                        box_xywh: input.box_xywh,
                        face_id: config
                            .find_person(person_id.trim())
                            .and_then(|person| {
                                person.appearances_iter().find(|a| {
                                    a.source.replace('\\', "/").trim_start_matches('/')
                                        == input.source.replace('\\', "/").trim_start_matches('/')
                                        && (0..4).all(|i| {
                                            (a.box_xywh[i] - input.box_xywh[i]).abs() < 1e-3
                                        })
                                })
                            })
                            .and_then(|a| a.face_id.clone()),
                    },
                )
                .map_err(|error| {
                    if error.contains("not found on person") {
                        AppError::BadRequest(tr(locale.as_str(), "error.ai_person_cover_not_found"))
                    } else {
                        AppError::BadRequest(error)
                    }
                })?;
        } else {
            config
                .clear_cover(person_id.trim())
                .map_err(AppError::BadRequest)?;
        }
    }
    save_faces_config_async(&album_dir, &config)
        .await
        .map_err(AppError::BadRequest)?;
    let cache = AlbumAiCache::new(cfg.ai_service.store().root(), &resolved.owner, &cache_id);
    if payload.name.is_some() || payload.hidden.is_some() {
        if let Err(error) = refresh_face_hints_file(&album_dir, &cache).await {
            log_match_refresh_error("patch person", error);
        }
    }
    let person = config.find_person(person_id.trim()).cloned();
    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "person_id": person_id.trim(),
            "updated": true,
            "person": person,
        })),
    ))
}

#[named]
pub async fn api_album_faces_delete(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    AxumPath((album, person_id)): AxumPath<(String, String)>,
) -> Result<Json<Value>, AppError> {
    let (_user, resolved, album_dir, cache_id) =
        resolve_owned_album(cfg.as_ref(), &token, album.trim(), locale.as_str()).await?;
    let mut config = load_faces_config_async(&album_dir).await;
    config
        .remove_person(person_id.trim())
        .map_err(AppError::BadRequest)?;
    save_faces_config_async(&album_dir, &config)
        .await
        .map_err(AppError::BadRequest)?;
    let cache = AlbumAiCache::new(cfg.ai_service.store().root(), &resolved.owner, &cache_id);
    if let Err(error) = refresh_face_hints_file(&album_dir, &cache).await {
        log_match_refresh_error("delete person", error);
    }
    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "person_id": person_id.trim(),
            "deleted": true,
        })),
    ))
}

#[named]
pub async fn api_album_faces_adopt_cluster(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    AxumPath(album): AxumPath<String>,
    Json(payload): Json<AdoptClusterRequest>,
) -> Result<Json<Value>, AppError> {
    let (_user, resolved, album_dir, cache_id) =
        resolve_owned_album(cfg.as_ref(), &token, album.trim(), locale.as_str()).await?;
    let cache = AlbumAiCache::new(cfg.ai_service.store().root(), &resolved.owner, &cache_id);
    let clusters = cache.read_face_index_async().await.ok_or_else(|| {
        AppError::BadRequest("no face_index.json; run AI index first".to_string())
    })?;
    let lookup = face_lookup(&clusters);
    let mut config = load_faces_config_async(&album_dir).await;
    let person_id = adopt_or_append(
        &mut config,
        &clusters,
        &lookup,
        &cache,
        payload.cluster_id.trim(),
        Some(payload.name.trim()),
        payload.target_person_id.as_deref().map(str::trim),
        payload.center_label.clone(),
    )
    .await
    .map_err(AppError::BadRequest)?;
    save_faces_config_async(&album_dir, &config)
        .await
        .map_err(AppError::BadRequest)?;
    if let Err(error) = refresh_face_hints_file(&album_dir, &cache).await {
        log_match_refresh_error("adopt cluster", error);
    }

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "person_id": person_id,
            "cluster_id": payload.cluster_id.trim(),
        })),
    ))
}

#[named]
pub async fn api_album_faces_merge_clusters(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    AxumPath(album): AxumPath<String>,
    Json(payload): Json<MergeClustersRequest>,
) -> Result<Json<Value>, AppError> {
    let (_user, resolved, album_dir, cache_id) =
        resolve_owned_album(cfg.as_ref(), &token, album.trim(), locale.as_str()).await?;
    let cache = AlbumAiCache::new(cfg.ai_service.store().root(), &resolved.owner, &cache_id);
    let clusters = cache.read_face_index_async().await.ok_or_else(|| {
        AppError::BadRequest("no face_index.json; run AI index first".to_string())
    })?;
    let lookup = face_lookup(&clusters);
    let mut config = load_faces_config_async(&album_dir).await;
    let person_id = adopt_or_append(
        &mut config,
        &clusters,
        &lookup,
        &cache,
        payload.cluster_id.trim(),
        None,
        Some(payload.target_person_id.trim()),
        payload
            .center_label
            .clone()
            .or_else(|| Some("variant".to_string())),
    )
    .await
    .map_err(AppError::BadRequest)?;
    save_faces_config_async(&album_dir, &config)
        .await
        .map_err(AppError::BadRequest)?;
    if let Err(error) = refresh_face_hints_file(&album_dir, &cache).await {
        log_match_refresh_error("merge cluster", error);
    }

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "person_id": person_id,
            "cluster_id": payload.cluster_id.trim(),
            "merged_as_center": true,
        })),
    ))
}

#[named]
pub async fn api_album_faces_split(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    AxumPath(album): AxumPath<String>,
    Json(payload): Json<SplitPersonRequest>,
) -> Result<Json<Value>, AppError> {
    let (_user, resolved, album_dir, cache_id) =
        resolve_owned_album(cfg.as_ref(), &token, album.trim(), locale.as_str()).await?;
    let selected: std::collections::HashSet<String> = payload
        .appearance_keys
        .iter()
        .chain(payload.face_ids.iter())
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();
    if selected.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_person_split_empty",
        )));
    }
    let mut config = load_faces_config_async(&album_dir).await;
    let source_id = payload.source_person_id.trim();
    let source = config
        .find_person(source_id)
        .ok_or_else(|| AppError::NotFound(tr(locale.as_str(), "error.ai_person_not_found")))?
        .clone();

    for key in &selected {
        let belongs = source.appearances_iter().any(|a| a.identity_key() == *key);
        if !belongs {
            return Err(AppError::BadRequest(format!(
                "appearance {key} does not belong to source person"
            )));
        }
    }

    let person = config
        .find_person_mut(source_id)
        .ok_or_else(|| AppError::NotFound(tr(locale.as_str(), "error.ai_person_not_found")))?;
    let moved = person.take_appearances_by_keys(&selected);
    if moved.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_person_split_empty",
        )));
    }

    let new_name = payload
        .new_name
        .as_deref()
        .map(normalize_person_name)
        .transpose()
        .map_err(AppError::BadRequest)?
        .unwrap_or_default();

    let new_person_id = Uuid::new_v4().to_string();
    let center = PersonCenter {
        id: Uuid::new_v4().to_string(),
        label: Some("split".to_string()),
        embedding_ref: None,
        center_b64: None,
        appearances: moved,
    };
    config.people.push(super::faces_config::AlbumPerson {
        id: new_person_id.clone(),
        name: new_name,
        hidden: false,
        centers: vec![center],
        cover: None,
    });
    save_faces_config_async(&album_dir, &config)
        .await
        .map_err(AppError::BadRequest)?;
    let cache = AlbumAiCache::new(cfg.ai_service.store().root(), &resolved.owner, &cache_id);
    if let Err(error) = refresh_face_hints_file(&album_dir, &cache).await {
        log_match_refresh_error("split person", error);
    }

    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "source_person_id": source_id,
            "new_person_id": new_person_id,
            "face_count": selected.len(),
        })),
    ))
}

#[named]
pub async fn api_album_faces_remove_center(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    AxumPath(album): AxumPath<String>,
    Json(payload): Json<RemoveCenterRequest>,
) -> Result<Json<Value>, AppError> {
    let (_user, resolved, album_dir, cache_id) =
        resolve_owned_album(cfg.as_ref(), &token, album.trim(), locale.as_str()).await?;
    let person_id = payload.person_id.trim();
    let center_id = payload.center_id.trim();
    if person_id.is_empty() || center_id.is_empty() {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.ai_person_center_not_found",
        )));
    }
    let mut config = load_faces_config_async(&album_dir).await;
    let had_person = config.find_person(person_id).is_some();
    let removed_count = config
        .remove_center(person_id, center_id)
        .map_err(|error| {
            if error.contains("not found") {
                AppError::BadRequest(tr(locale.as_str(), "error.ai_person_center_not_found"))
            } else {
                AppError::BadRequest(error)
            }
        })?;
    save_faces_config_async(&album_dir, &config)
        .await
        .map_err(AppError::BadRequest)?;
    let cache = AlbumAiCache::new(cfg.ai_service.store().root(), &resolved.owner, &cache_id);
    if let Err(error) = refresh_face_hints_file(&album_dir, &cache).await {
        log_match_refresh_error("remove center", error);
    }
    let person_deleted = had_person && config.find_person(person_id).is_none();
    let message = tr(locale.as_str(), "response.success");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({
            "person_id": person_id,
            "center_id": center_id,
            "removed_count": removed_count,
            "person_deleted": person_deleted,
        })),
    ))
}

async fn find_named_appearance_by_face_id(
    album_dir: &Path,
    face_id: &str,
) -> Option<PersonAppearance> {
    let config = load_faces_config_async(album_dir).await;
    for person in &config.people {
        for appearance in person.appearances_iter() {
            if appearance.face_id.as_deref() == Some(face_id) {
                return Some(appearance.clone());
            }
        }
    }
    None
}

/// Serve a face crop by `face_id` (preferred) or legacy `source`+`box`.
#[named]
pub async fn api_album_face_crop(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    AxumPath(album): AxumPath<String>,
    Query(query): Query<FaceCropQuery>,
) -> Result<Response<Body>, AppError> {
    let Some(session) = cfg.get_session_info(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let user_root = cfg.user_root_path(&session.username);
    let resolved = resolve_album_access(
        cfg.as_ref(),
        &session.username,
        &user_root,
        album.trim(),
        locale.as_str(),
    )
    .await?;
    let album_dir = albums_root(&resolved.root_path).join(&resolved.album_name);
    let cache_id = ensure_album_cache_id(&album_dir).await?;
    let cache = AlbumAiCache::new(cfg.ai_service.store().root(), &resolved.owner, &cache_id);
    let _ = query.size;

    let (path, source, box_xywh) = if let Some(fid) = query
        .face_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let Some(path) = cache.face_crop_path_by_id(fid) else {
            return Err(AppError::NotFound(fid.to_string()));
        };
        if let Some(entry) = cache.find_face_in_index_async(fid).await {
            (path, entry.source, entry.box_xywh)
        } else if let Some(app) = find_named_appearance_by_face_id(&album_dir, fid).await {
            (path, app.source, app.box_xywh)
        } else if is_readable_face_crop(&path).await {
            (path, String::new(), [0.0; 4])
        } else {
            return Err(AppError::NotFound(fid.to_string()));
        }
    } else {
        let source = query
            .source
            .as_deref()
            .unwrap_or("")
            .replace('\\', "/")
            .trim_start_matches('/')
            .to_string();
        let box_raw = query.box_xywh.as_deref().unwrap_or("");
        let box_xywh = parse_box_xywh(box_raw).map_err(AppError::BadRequest)?;
        if source.is_empty() || source.contains("..") {
            return Err(AppError::BadRequest(tr(
                locale.as_str(),
                "error.invalid_face_box",
            )));
        }
        // Named people live in faces.json only; they are omitted from face_index.json.
        // Prefer index crop path when present, else generate by source+box.
        let clusters = cache
            .read_face_index_async()
            .await
            .unwrap_or_else(ClustersCacheFile::empty);
        let path = if let Some(entry) = clusters.faces.iter().find(|e| {
            e.source.replace('\\', "/").trim_start_matches('/') == source
                && (0..4).all(|i| (e.box_xywh[i] - box_xywh[i]).abs() < 1e-3)
        }) {
            cache
                .face_crop_path_by_id(&entry.face_id)
                .unwrap_or_else(|| cache.face_crop_path_by_appearance(&source, &box_xywh))
        } else {
            cache.face_crop_path_by_appearance(&source, &box_xywh)
        };
        (path, source, box_xywh)
    };

    ensure_cached_face_crop(
        &album_dir,
        &path,
        &source,
        box_xywh,
        FACE_CROP_SIZE,
        &cfg.thumbnail_limiter,
        &cfg.darktable_limiter,
    )
    .await
    .map_err(AppError::Unknown)?;
    let file = tokio::fs::File::open(&path)
        .await
        .map_err(|e| AppError::Unknown(format!("open face crop: {e}")))?;
    let mut headers = HeaderMap::new();
    headers.insert("Content-Type", HeaderValue::from_static("image/webp"));
    headers.insert(
        "Cache-Control",
        HeaderValue::from_static("private, max-age=86400"),
    );
    let mut response = Response::new(Body::from_stream(ReaderStream::new(file)));
    *response.headers_mut() = headers;
    Ok(response)
}

#[allow(dead_code)]
pub fn decode_center_vector(center: &PersonCenter) -> Option<Vec<f32>> {
    center
        .center_b64
        .as_ref()
        .and_then(|b| decode_f32_vector_b64(b, None).ok())
}

#[allow(dead_code)]
pub fn encode_center_vector(vector: &[f32]) -> String {
    encode_f32_vector_b64(vector)
}
