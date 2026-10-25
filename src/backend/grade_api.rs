//! Grade documents live in memory, one per open. A login can hold several
//! opens at once. Each open has its own `rev`. Node edits and the full-size
//! preview name that `rev`, and the `rev` must belong to this login.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use axum::Extension;
use axum::Json;
use axum::extract::Multipart;
use base64::Engine;
use percent_encoding::percent_decode;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::backend::album::helpers::{albums_root, read_album_meta, sanitize_album_name};

use crate::backend::album::decoder::{
    CpuImageDecoder, DecodeOptions, ImageDecoder, RawGradePixels, decode_raw_for_grade,
    format_from_path,
};
use crate::backend::album::media::{is_image_path, resolve_photo_owner_root};
use crate::backend::common::{AppError, AppState, Token, into_host_path_safely};
use crate::backend::i18n::{RequestLocale, tr};
use crate::grade::{
    CameraColor, CancelFlag, Document, FieldKind, FieldSpec, Frame, GradeNode, NodePatch, Rgba,
    affected, apply_node_params, default_params, document_with_camera, fields,
    apply_saved_document, document_json, ensure_grade_lut, frame_from_camera_rgb, frame_from_image,
    params_json, parse_camera_profile, render_cancellable, title, user_root_from_photo,
};

const SMALL_EDGE: u32 = 640;
const SESSION_CAP: usize = 16;

static REVS: AtomicU64 = AtomicU64::new(1);
static SESSIONS: LazyLock<Mutex<HashMap<u64, Arc<Mutex<GradeSession>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct GradeSession {
    token: String,
    rev: u64,
    /// Bumped on every parameter edit so an older full-size render is discarded.
    edit_gen: u64,
    doc: Document,
    path: std::path::PathBuf,
    /// Own photos and writable shares may write `{filename}.lumeno.json`. Read-only shares may not.
    writable: bool,
    raw: bool,
    small: Frame,
    small_cache: HashMap<String, std::sync::Arc<Frame>>,
    /// Original working pixels. Filled by the full-size pass, not by the fast open.
    full: Arc<Mutex<Option<Frame>>>,
    full_cache: HashMap<String, std::sync::Arc<Frame>>,
    job_seq: u64,
    /// Renders still inside the pipeline. A newer request cancels these between nodes.
    inflight: Vec<Inflight>,
}

/// A full-size render leaves a running small preview alone. It stops a node edit.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RenderKind {
    /// The fast preview of the document as opened, before a parameter edit.
    Preview,
    /// A node edit on the small frame.
    Incremental,
    Full,
}

#[derive(Clone)]
struct Inflight {
    id: u64,
    kind: RenderKind,
    cancel: Arc<CancelFlag>,
}

/// A small preview and a full-size render leave each other running. Every other pair stops the older one.
fn interrupts(running: RenderKind, incoming: RenderKind) -> bool {
    !matches!(
        (running, incoming),
        (RenderKind::Preview, RenderKind::Full) | (RenderKind::Full, RenderKind::Preview)
    )
}

impl GradeSession {
    fn claim(&mut self, kind: RenderKind) -> Inflight {
        for job in &self.inflight {
            if interrupts(job.kind, kind) {
                job.cancel.cancel();
            }
        }
        self.job_seq = self.job_seq.wrapping_add(1);
        let job = Inflight {
            id: self.job_seq,
            kind,
            cancel: Arc::new(CancelFlag::new()),
        };
        self.inflight.push(job.clone());
        job
    }

    fn finish_job(&mut self, id: u64) {
        self.inflight.retain(|job| job.id != id);
    }
}

#[derive(Debug, Deserialize)]
pub struct OpenBody {
    path: String,
    #[serde(default)]
    owner: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NodeBody {
    id: String,
    enabled: bool,
    params: Value,
    rev: u64,
}

#[derive(Debug, Deserialize)]
pub struct FullBody {
    rev: u64,
}

pub async fn api_grade_open(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(body): Json<OpenBody>,
) -> Result<Json<Value>, AppError> {
    let source = resolve_source(&locale, &token, &cfg, &body.path, body.owner.as_deref()).await?;
    let raw = format_from_path(source.as_path())
        .ok()
        .is_some_and(|format| format.is_camera_raw());
    let locale_code = locale.as_str().to_string();
    let token_key = token.0.clone();
    let writable = grade_writable(
        &cfg,
        &token,
        body.owner.as_deref(),
        &body.path,
        locale.as_str(),
    )
    .await?;
    let _permit = cfg.darktable_limiter.acquire().await;
    let opened = tokio::task::spawn_blocking(move || {
        let (frame, baseline_ev, camera) = load_frame(source.as_path(), raw, SMALL_EDGE)?;
        Ok(open_session(
            token_key,
            locale_code,
            raw,
            source,
            writable,
            frame,
            baseline_ev,
            camera,
        ))
    })
    .await
    .map_err(|error| AppError::Unknown(error.to_string()))?
    .map_err(AppError::BadRequest)?;
    Ok(Json(opened))
}

pub async fn api_grade_preview(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(body): Json<FullBody>,
) -> Result<Json<Value>, AppError> {
    let Some(_) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let token_key = token.0.clone();
    let locale_code = locale.as_str().to_string();
    let prepared = tokio::task::spawn_blocking(move || prepare_preview(token_key, body.rev))
        .await
        .map_err(|error| AppError::Unknown(error.to_string()))?;
    render_claimed(cfg, &locale_code, prepared).await
}

pub async fn api_grade_node(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(body): Json<NodeBody>,
) -> Result<Json<Value>, AppError> {
    let Some(_) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let locale_code = locale.as_str().to_string();
    let token_key = token.0.clone();
    let locale_for_job = locale_code.clone();
    let prepared =
        tokio::task::spawn_blocking(move || prepare_patch(token_key, locale_for_job, body))
            .await
            .map_err(|error| AppError::Unknown(error.to_string()))?;
    render_claimed(cfg, &locale_code, prepared).await
}

pub async fn api_grade_full(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(body): Json<FullBody>,
) -> Result<Json<Value>, AppError> {
    let Some(_) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let token_key = token.0.clone();
    let locale_code = locale.as_str().to_string();
    let prepared = tokio::task::spawn_blocking(move || prepare_full(token_key, body.rev))
        .await
        .map_err(|error| AppError::Unknown(error.to_string()))?;
    render_claimed(cfg, &locale_code, prepared).await
}

pub async fn api_grade_profile(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<Value>, AppError> {
    let Some(_) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let mut rev = None;
    let mut file_name = String::new();
    let mut bytes = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| AppError::BadRequest(error.to_string()))?
    {
        match field.name() {
            Some("rev") => {
                let text = field
                    .text()
                    .await
                    .map_err(|error| AppError::BadRequest(error.to_string()))?;
                rev = text.parse().ok();
            }
            Some("file") => {
                file_name = field.file_name().unwrap_or("profile").to_string();
                let data = field
                    .bytes()
                    .await
                    .map_err(|error| AppError::BadRequest(error.to_string()))?;
                if data.len() > 1024 * 1024 {
                    return Err(grade_error(locale.as_str(), "profile-invalid".into()));
                }
                bytes = Some(data);
            }
            _ => {}
        }
    }
    let Some(rev) = rev else {
        return Err(grade_error(locale.as_str(), "stale".into()));
    };
    let Some(bytes) = bytes else {
        return Err(grade_error(locale.as_str(), "profile-invalid".into()));
    };
    let locale_code = locale.as_str().to_string();
    let token_key = token.0.clone();
    let locale_for_job = locale_code.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        prepare_profile(&token_key, &locale_for_job, rev, &bytes, &file_name)
    })
    .await
    .map_err(|error| AppError::Unknown(error.to_string()))?;
    render_claimed(cfg, &locale_code, prepared).await
}

fn prepare_profile(
    token: &str,
    locale: &str,
    rev: u64,
    bytes: &[u8],
    file_name: &str,
) -> Result<Prepared, String> {
    let parsed =
        parse_camera_profile(bytes, file_name).map_err(|_| "profile-invalid".to_string())?;
    let (enabled, mut params) = {
        let arc = lock_session(token, rev)?;
        let guard = arc.lock().unwrap_or_else(|err| err.into_inner());
        let node = guard
            .doc
            .nodes
            .iter()
            .find(|node| node.id == "input-color-profile")
            .ok_or_else(|| "profile-not-camera".to_string())?;
        let params = params_json(node);
        if params.get("camera").and_then(Value::as_bool) != Some(true) {
            return Err("profile-not-camera".into());
        }
        (node.enabled, params)
    };
    params["source"] = json!("imported");
    params["imported_name"] = json!(parsed.name);
    params["imported"] = json!(parsed.to_working);
    prepare_patch(
        token.to_string(),
        locale.to_string(),
        NodeBody {
            id: "input-color-profile".into(),
            enabled,
            params,
            rev,
        },
    )
}

fn open_session(
    token: String,
    locale: String,
    raw: bool,
    path: std::path::PathBuf,
    writable: bool,
    source: Frame,
    baseline_ev: f32,
    camera: Option<CameraColor>,
) -> Value {
    let mut doc = document_with_camera(raw, baseline_ev, camera);
    if let Some(saved) = read_grade_sidecar(&path) {
        let _ = apply_saved_document(&mut doc, &saved);
    }
    let rev = REVS.fetch_add(1, Ordering::Relaxed);
    let payload = json!({
        "rev": rev,
        "nodes": nodes_json(&locale, &doc),
    });
    let session = GradeSession {
        token,
        rev,
        edit_gen: 1,
        doc,
        path,
        writable,
        raw,
        small: source.duplicate(),
        small_cache: HashMap::new(),
        full: Arc::new(Mutex::new(None)),
        full_cache: HashMap::new(),
        job_seq: 0,
        inflight: Vec::new(),
    };
    let mut map = SESSIONS.lock().unwrap_or_else(|err| err.into_inner());
    map.insert(rev, Arc::new(Mutex::new(session)));
    while map.len() > SESSION_CAP {
        let Some(oldest) = map.keys().copied().filter(|item| *item != rev).min() else {
            break;
        };
        map.remove(&oldest);
    }
    payload
}

fn lock_session(token: &str, rev: u64) -> Result<Arc<Mutex<GradeSession>>, String> {
    let map = SESSIONS.lock().unwrap_or_else(|err| err.into_inner());
    let arc = map
        .get(&rev)
        .cloned()
        .ok_or_else(|| "not-open".to_string())?;
    drop(map);
    let guard = arc.lock().unwrap_or_else(|err| err.into_inner());
    if guard.token != token {
        return Err("not-open".into());
    }
    drop(guard);
    Ok(arc)
}

/// Claimed before the darktable permit, so a newer request can stop the one that still holds it.
struct Prepared {
    arc: Arc<Mutex<GradeSession>>,
    job: Inflight,
    rev: u64,
    generation: u64,
    doc: Document,
    source: Option<Frame>,
    cache: HashMap<String, std::sync::Arc<Frame>>,
    dirty: HashSet<String>,
    kind: RenderKind,
    locale: String,
    node_id: Option<String>,
    full_path: Option<std::path::PathBuf>,
    full_raw: bool,
    full_slot: Option<Arc<Mutex<Option<Frame>>>>,
}

impl Drop for Prepared {
    fn drop(&mut self) {
        let mut session = self.arc.lock().unwrap_or_else(|err| err.into_inner());
        session.finish_job(self.job.id);
    }
}

fn prepare_preview(token: String, rev: u64) -> Result<Prepared, String> {
    let arc = lock_session(&token, rev)?;
    let mut session = arc.lock().unwrap_or_else(|err| err.into_inner());
    let job = session.claim(RenderKind::Preview);
    let doc = session.doc.clone();
    let prepared = Prepared {
        rev: session.rev,
        generation: session.edit_gen,
        source: Some(session.small.duplicate()),
        cache: HashMap::new(),
        dirty: doc.nodes.iter().map(|node| node.id.clone()).collect(),
        kind: RenderKind::Preview,
        locale: String::new(),
        node_id: None,
        full_path: None,
        full_raw: false,
        full_slot: None,
        doc,
        arc: Arc::clone(&arc),
        job,
    };
    drop(session);
    Ok(prepared)
}

fn prepare_patch(token: String, locale: String, body: NodeBody) -> Result<Prepared, String> {
    let arc = lock_session(&token, body.rev)?;
    let mut session = arc.lock().unwrap_or_else(|err| err.into_inner());
    apply_node_params(
        &mut session.doc,
        &NodePatch {
            id: body.id.clone(),
            enabled: body.enabled,
            params: body.params,
        },
    )?;
    session.edit_gen = session.edit_gen.wrapping_add(1);
    let dirty = affected(&session.doc, &body.id);
    for id in &dirty {
        session.full_cache.remove(id);
    }
    let job = session.claim(RenderKind::Incremental);
    let prepared = Prepared {
        rev: session.rev,
        generation: session.edit_gen,
        doc: session.doc.clone(),
        source: Some(session.small.duplicate()),
        cache: session.small_cache.clone(),
        dirty,
        kind: RenderKind::Incremental,
        locale,
        node_id: Some(body.id),
        full_path: None,
        full_raw: false,
        full_slot: None,
        arc: Arc::clone(&arc),
        job,
    };
    drop(session);
    Ok(prepared)
}

fn prepare_full(token: String, rev: u64) -> Result<Prepared, String> {
    let arc = lock_session(&token, rev)?;
    let mut session = arc.lock().unwrap_or_else(|err| err.into_inner());
    let job = session.claim(RenderKind::Full);
    let prepared = Prepared {
        rev: session.rev,
        generation: session.edit_gen,
        doc: session.doc.clone(),
        source: None,
        cache: session.full_cache.clone(),
        dirty: HashSet::new(),
        kind: RenderKind::Full,
        locale: String::new(),
        node_id: None,
        full_path: Some(session.path.clone()),
        full_raw: session.raw,
        full_slot: Some(Arc::clone(&session.full)),
        arc: Arc::clone(&arc),
        job,
    };
    drop(session);
    Ok(prepared)
}

async fn render_claimed(
    cfg: Arc<AppState>,
    locale: &str,
    prepared: Result<Prepared, String>,
) -> Result<Json<Value>, AppError> {
    let prepared = prepared.map_err(|error| grade_error(locale, error))?;
    let _permit = cfg.darktable_limiter.acquire().await;
    let rendered = tokio::task::spawn_blocking(move || execute_prepared(prepared))
        .await
        .map_err(|error| AppError::Unknown(error.to_string()))?
        .map_err(|error| grade_error(locale, error))?;
    Ok(Json(rendered))
}

async fn grade_writable(
    cfg: &AppState,
    token: &Token,
    owner: Option<&str>,
    path: &str,
    locale: &str,
) -> Result<bool, AppError> {
    let Some((username, _)) = cfg.get_session_and_root(token).await else {
        return Err(AppError::unauthorized(locale));
    };
    let Some(owner) = owner.filter(|name| *name != username) else {
        return Ok(true);
    };
    let decoded = percent_decode(path.as_bytes())
        .decode_utf8()
        .unwrap_or(std::borrow::Cow::Borrowed(path));
    let Some(album) = decoded
        .trim_start_matches('/')
        .strip_prefix("albums/")
        .and_then(|rest| rest.split('/').next())
        .and_then(sanitize_album_name)
    else {
        return Ok(false);
    };
    let album_dir = albums_root(&cfg.user_root_path(owner)).join(album);
    let meta = read_album_meta(&album_dir).await;
    Ok(meta
        .shares
        .iter()
        .any(|share| share.username == username && share.can_write))
}

/// `{photo.ext}.lumeno.json` beside the original. The original and any XMP stay untouched.
fn grade_sidecar(photo: &std::path::Path) -> Option<std::path::PathBuf> {
    let name = photo.file_name()?.to_str()?;
    if name.is_empty() || name.contains('\0') {
        return None;
    }
    let sidecar = photo.with_file_name(format!("{name}.lumeno.json"));
    if sidecar.parent() != photo.parent() {
        return None;
    }
    Some(sidecar)
}

fn read_grade_sidecar(photo: &std::path::Path) -> Option<Value> {
    let path = grade_sidecar(photo)?;
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if meta.file_type().is_symlink() {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn write_grade_sidecar(photo: &std::path::Path, doc: &Document) -> Result<(), ()> {
    let path = grade_sidecar(photo).ok_or(())?;
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if meta.file_type().is_symlink() {
            return Err(());
        }
    }
    let bytes = serde_json::to_vec_pretty(&document_json(doc)).map_err(|_| ())?;
    let file_name = path.file_name().and_then(|name| name.to_str()).ok_or(())?;
    let tmp = path.with_file_name(format!(".{file_name}.tmp"));
    std::fs::write(&tmp, &bytes).map_err(|_| ())?;
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Err(());
    }
    Ok(())
}

fn bind_lut(arc: &Arc<Mutex<GradeSession>>, doc: &Document) -> Result<(), String> {
    let Some(node) = doc.nodes.iter().find(|node| node.id == "lut" && node.enabled) else {
        return Ok(());
    };
    let params = params_json(node);
    let id = params.get("lut").and_then(Value::as_str).unwrap_or("");
    let strength = params.get("strength").and_then(Value::as_f64).unwrap_or(1.0);
    if id.is_empty() || strength <= 0.0 {
        return Ok(());
    }
    let photo = {
        let session = arc.lock().unwrap_or_else(|err| err.into_inner());
        session.path.clone()
    };
    let Some(root) = user_root_from_photo(&photo) else {
        return Ok(());
    };
    ensure_grade_lut(&root, id).map_err(|error| error.to_string())
}

fn execute_prepared(mut prepared: Prepared) -> Result<Value, String> {
    if prepared.kind == RenderKind::Full {
        let slot = prepared
            .full_slot
            .clone()
            .ok_or_else(|| "not-open".to_string())?;
        let path = prepared
            .full_path
            .clone()
            .ok_or_else(|| "not-open".to_string())?;
        {
            let mut guard = slot.lock().unwrap_or_else(|err| err.into_inner());
            if guard.is_none() {
                *guard = Some(load_frame(path.as_path(), prepared.full_raw, 0)?.0);
            }
        }
        if prepared.job.cancel.is_cancelled() {
            return Ok(json!({ "rev": prepared.rev, "superseded": true }));
        }
        prepared.source = slot
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .as_ref()
            .map(Frame::duplicate);
    }
    if prepared.job.cancel.is_cancelled() {
        return Ok(json!({ "rev": prepared.rev, "superseded": true }));
    }
    let source = prepared
        .source
        .take()
        .ok_or_else(|| "not-open".to_string())?;
    bind_lut(&prepared.arc, &prepared.doc)?;
    let rendered = render_cancellable(
        &prepared.doc,
        &source,
        &mut prepared.cache,
        &prepared.dirty,
        &prepared.job.cancel,
    );
    let kind = prepared.kind;
    let generation = prepared.generation;
    let rev = prepared.rev;
    let locale = prepared.locale.clone();
    let node_id = prepared.node_id.clone();
    let cancel = Arc::clone(&prepared.job.cancel);
    let cache = std::mem::take(&mut prepared.cache);
    let mut session = prepared.arc.lock().unwrap_or_else(|err| err.into_inner());
    let cancelled = cancel.is_cancelled();
    let stale = session.edit_gen != generation;
    if cancelled || stale {
        return Ok(json!({ "rev": rev, "superseded": true }));
    }
    if session.writable {
        let _ = write_grade_sidecar(&session.path, &session.doc);
    }
    let quality = if kind == RenderKind::Full { 92 } else { 82 };
    let preview = preview_url(&rendered.preview, quality);
    let view = view_json(rendered.degrees, rendered.roi);
    if kind == RenderKind::Full {
        session.full_cache = cache;
        return Ok(json!({
            "rev": session.rev,
            "preview": preview,
            "view": view,
        }));
    }
    session.small_cache = cache;
    if let Some(id) = node_id.as_deref() {
        let node = session
            .doc
            .nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or_else(|| format!("unknown node {id}"))?;
        let node = node_json(&locale, node);
        return Ok(json!({
            "rev": session.rev,
            "node": node,
            "preview": preview,
            "view": view,
        }));
    }
    Ok(json!({
        "rev": session.rev,
        "preview": preview,
        "view": view,
    }))
}

fn view_json(degrees: f32, roi: [f32; 4]) -> Value {
    json!({
        "degrees": degrees,
        "roi": { "x": roi[0], "y": roi[1], "w": roi[2], "h": roi[3] }
    })
}

fn load_frame(
    path: &std::path::Path,
    raw: bool,
    max_edge: u32,
) -> Result<(Frame, f32, Option<CameraColor>), String> {
    if raw {
        return match decode_raw_for_grade(path).map_err(|error| error.message)? {
            RawGradePixels::Camera {
                width,
                height,
                samples,
                baseline_ev,
                to_working,
                camera_name,
            } => Ok((
                frame_from_camera_rgb(width, height, samples, max_edge),
                baseline_ev,
                Some(CameraColor {
                    name: camera_name,
                    to_working,
                }),
            )),
            RawGradePixels::Srgb(image) => Ok((frame_from_image(&image, max_edge), 0.0, None)),
        };
    }
    let format = format_from_path(path).map_err(|error| error.message)?;
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let image = CpuImageDecoder
        .decode_bytes(
            &bytes,
            DecodeOptions {
                format: Some(format),
                max_edge: 0,
                ..DecodeOptions::new()
            },
        )
        .map_err(|error| error.message)?;
    Ok((frame_from_image(&image, max_edge), 0.0, None))
}

fn nodes_json(locale: &str, doc: &Document) -> Vec<Value> {
    doc.nodes
        .iter()
        .map(|node| node_json(locale, node))
        .collect()
}

fn node_json(locale: &str, node: &GradeNode) -> Value {
    json!({
        "id": node.id,
        "enabled": node.enabled,
        "always_on": node.always_on(),
        "title": title(locale, &node.id),
        "params": params_json(node),
        "defaults": default_params(node),
        "fields": fields(locale, node).iter().map(field_json).collect::<Vec<_>>(),
    })
}

fn field_json(field: &FieldSpec) -> Value {
    let options = if field.kind == FieldKind::Choice {
        let labels: Vec<&str> = field
            .choices
            .split('|')
            .filter(|item| !item.is_empty())
            .collect();
        let wires: Vec<&str> = field
            .wires
            .split('|')
            .filter(|item| !item.is_empty())
            .collect();
        labels
            .into_iter()
            .zip(wires)
            .map(|(label, value)| json!({ "label": label, "value": value }))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    json!({
        "key": field.key,
        "label": field.label,
        "kind": field.kind.as_str(),
        "value": field.value,
        "min": field.min,
        "max": field.max,
        "display": field.display,
        "options": options,
    })
}

fn preview_url(rgba: &Rgba, quality: u8) -> String {
    let mut rgb = Vec::with_capacity((rgba.width * rgba.height * 3) as usize);
    for pixel in rgba.pixels.chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
    }
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, quality);
    if encoder
        .encode(
            &rgb,
            rgba.width,
            rgba.height,
            image::ExtendedColorType::Rgb8,
        )
        .is_err()
    {
        return String::new();
    }
    format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

async fn resolve_source(
    locale: &RequestLocale,
    token: &Token,
    cfg: &AppState,
    path: &str,
    owner: Option<&str>,
) -> Result<std::path::PathBuf, AppError> {
    let Some((username, root)) = cfg.get_session_and_root(token).await else {
        return Err(AppError::unauthorized(locale.as_str()));
    };
    let root = resolve_photo_owner_root(cfg, &username, root, owner, path, locale.as_str()).await?;
    let full = into_host_path_safely(root.as_path(), path)?;
    if !full.is_file() {
        return Err(AppError::NotFound(tr(
            locale.as_str(),
            "error.resource_not_found",
        )));
    }
    if !is_image_path(full.as_path()) {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.not_previewable_image",
        )));
    }
    Ok(full)
}

#[cfg(test)]
mod tests {
    use super::{RenderKind, grade_sidecar, interrupts};

    #[test]
    fn full_render_spares_only_the_small_preview() {
        assert!(!interrupts(RenderKind::Preview, RenderKind::Full));
        assert!(interrupts(RenderKind::Incremental, RenderKind::Full));
        assert!(interrupts(RenderKind::Preview, RenderKind::Incremental));
        assert!(interrupts(RenderKind::Incremental, RenderKind::Incremental));
        assert!(interrupts(RenderKind::Full, RenderKind::Incremental));
        assert!(!interrupts(RenderKind::Full, RenderKind::Preview));
        assert!(interrupts(RenderKind::Preview, RenderKind::Preview));
        assert!(interrupts(RenderKind::Full, RenderKind::Full));
    }

    #[test]
    fn grade_sidecar_keeps_the_original_name() {
        let photo = std::path::Path::new("/library/albums/trips/IMG_1.RAF");
        let sidecar = grade_sidecar(photo).unwrap();
        assert_eq!(
            sidecar.file_name().and_then(|name| name.to_str()),
            Some("IMG_1.RAF.lumeno.json")
        );
        assert_eq!(sidecar.parent(), photo.parent());
    }
}

fn grade_error(locale: &str, error: String) -> AppError {
    let (reason, key) = match error.as_str() {
        "not-open" => ("not-open", "error.grade_not_open"),
        "stale" => ("stale", "error.grade_stale"),
        "profile-invalid" => ("profile-invalid", "error.grade_profile_invalid"),
        "profile-not-camera" => ("profile-not-camera", "error.grade_profile_not_camera"),
        "always-on" => ("always-on", "error.grade_always_on"),
        other => return AppError::BadRequest(other.to_string()),
    };
    AppError::GradeRejected {
        message: tr(locale, key),
        reason,
    }
}
