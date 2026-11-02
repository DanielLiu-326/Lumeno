//! One-shot batch download of album originals as a stored (method 0) zip.
//!
//! Local headers set the data-descriptor flag so each file can be copied
//! through in chunks before its CRC is known. The central directory is kept
//! in memory and appended after the last file. No `zip` crate: that writer
//! wants `Seek`, which an HTTP body does not have.

use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tokio::io::AsyncReadExt;

use axum::Json;
use axum::body::{Body, Bytes};
use axum::extract::{Extension, Path as AxumPath};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use chrono::{Datelike, Timelike};
use futures::Stream;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

use crate::backend::common::{
    AppError, AppState, ConcurrencyLimiter, Token, create_response, into_host_path_safely,
};
use crate::backend::grade_api::render_grade_export;
use crate::backend::i18n::{RequestLocale, tr, trf};
use crate::backend::identity::{AlbumName, AlbumRef, Owner, PhotoRef, PhotoRel};

use super::api::resolve_album_access;
use super::decoder::{darktable_export, find_xmp_sidecar};
use super::helpers::albums_root;
use super::media::is_image_path;

/// Downloads stream files that already exist, so the cap only bounds the request.
const MAX_PHOTOS: usize = 2000;
/// Export renders every file before it can be zipped; the cap keeps one ticket
/// from turning into an hours-long CPU job.
const MAX_EXPORT_PHOTOS: usize = 100;
const MAX_TICKETS: usize = 128;
const TICKET_TTL: Duration = Duration::from_secs(5 * 60);
const CHUNK: usize = 64 * 1024;
/// XMP grading is opt-in: a request that omits the flag exports the pixels as
/// they are. Single-photo and batch export share this default.
pub(crate) const DEFAULT_APPLY_XMP: bool = false;
/// General-purpose bits: data descriptor (bit 3) and UTF-8 names (bit 11).
const GP_FLAGS: u16 = 0x0808;

static CRC_TABLE: LazyLock<[u32; 256]> = LazyLock::new(|| {
    let mut table = [0u32; 256];
    for i in 0..256 {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 == 1 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
        table[i] = c;
    }
    table
});

static TICKETS: LazyLock<Mutex<HashMap<String, Ticket>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct Ticket {
    viewer: String,
    owner: String,
    album_name: String,
    filename: String,
    files: Vec<ZipPart>,
    zip64: bool,
    content_length: u64,
    expires: Instant,
}

#[derive(Clone)]
struct ZipPart {
    name: String,
    path: PathBuf,
    size: u64,
    dos_time: u16,
    dos_date: u16,
}

struct CentralRec {
    name: String,
    crc: u32,
    size: u64,
    offset: u64,
    dos_time: u16,
    dos_date: u16,
}

#[derive(Debug, Deserialize)]
pub struct DownloadRequest {
    owner: String,
    albumname: String,
    photos: Vec<String>,
}

enum ZipPhase {
    File,
    Central,
    Done,
}

/// One archive entry. Export entries are produced lazily: the bytes only exist
/// after the photo was rendered or transcoded.
struct ZipEntry {
    name: String,
    path: PathBuf,
    size: u64,
    dos_time: u16,
    dos_date: u16,
    /// Owned temp file; removed once its bytes are in the archive.
    temp: bool,
}

/// Where the entries come from. Download streams files that already exist;
/// export renders each photo first. Both feed the same encoder so the archive
/// bytes cannot drift between the two flows.
enum ZipPlan {
    Files {
        parts: Vec<ZipPart>,
        index: usize,
        zip64: bool,
    },
    Export {
        ticket: ExportTicket,
        index: usize,
        used_names: HashSet<String>,
    },
}

impl ZipPlan {
    fn zip64(&self) -> bool {
        match self {
            ZipPlan::Files { zip64, .. } => *zip64,
            // Rendered sizes are unknown when the ticket is created, so every
            // export entry uses the ZIP64 form regardless of its real size.
            ZipPlan::Export { .. } => true,
        }
    }

    async fn next_entry(&mut self) -> io::Result<Option<ZipEntry>> {
        match self {
            ZipPlan::Files { parts, index, .. } => {
                let Some(part) = parts.get(*index) else {
                    return Ok(None);
                };
                *index += 1;
                Ok(Some(ZipEntry {
                    name: part.name.clone(),
                    path: part.path.clone(),
                    size: part.size,
                    dos_time: part.dos_time,
                    dos_date: part.dos_date,
                    temp: false,
                }))
            }
            ZipPlan::Export {
                ticket,
                index,
                used_names,
            } => {
                if *index >= ticket.sources.len() {
                    return Ok(None);
                }
                let source_path = ticket.sources[*index].path.clone();
                let subpath = ticket.sources[*index].subpath.clone();
                let dos_time = ticket.sources[*index].dos_time;
                let dos_date = ticket.sources[*index].dos_date;
                let apply_grade = ticket.apply_grade;
                let xmp = if !apply_grade && ticket.apply_xmp {
                    find_xmp_sidecar(&source_path)
                } else {
                    None
                };
                let edited = apply_grade || xmp.is_some();
                let out_ext = ticket.out_ext.clone();
                let quality = ticket.quality;
                let max_width = ticket.max_width;
                let direct =
                    !edited && max_width == 0 && same_export_format(&source_path, &out_ext);
                let name = next_entry_name(used_names, &subpath, &out_ext, edited);
                let (path, temp) = if direct {
                    (source_path, false)
                } else {
                    let temp_path = std::env::temp_dir().join(format!(
                        "lumeno-export-{}-{}.{}",
                        Uuid::new_v4(),
                        *index,
                        out_ext
                    ));
                    let limiter = ticket.limiter.clone();
                    let rendered = if apply_grade {
                        match render_grade_export(
                            &limiter,
                            &source_path,
                            &out_ext,
                            quality,
                            max_width,
                        )
                        .await
                        {
                            Ok(bytes) => tokio::fs::write(&temp_path, bytes)
                                .await
                                .map_err(|err| io::Error::other(err.to_string())),
                            Err(err) => Err(io::Error::other(err.to_string())),
                        }
                    } else {
                        let _permit = limiter.acquire().await;
                        darktable_export(
                            &source_path,
                            xmp.as_deref(),
                            &temp_path,
                            &out_ext,
                            quality,
                            max_width,
                        )
                        .await
                        .map_err(|err| io::Error::other(err.message))
                    };
                    if let Err(error) = rendered {
                        let _ = tokio::fs::remove_file(&temp_path).await;
                        return Err(error);
                    }
                    (temp_path, true)
                };
                let meta = match tokio::fs::metadata(&path).await {
                    Ok(meta) => meta,
                    Err(error) => {
                        if temp {
                            let _ = tokio::fs::remove_file(&path).await;
                        }
                        return Err(error);
                    }
                };
                if !meta.is_file() {
                    if temp {
                        let _ = tokio::fs::remove_file(&path).await;
                    }
                    return Err(io::Error::other("export did not produce a file"));
                }
                *index += 1;
                Ok(Some(ZipEntry {
                    name,
                    path,
                    size: meta.len(),
                    dos_time,
                    dos_date,
                    temp,
                }))
            }
        }
    }
}

/// Streaming ZIP writer shared by download and export.
struct ZipStream {
    plan: ZipPlan,
    zip64: bool,
    current: Option<ZipEntry>,
    reader: Option<tokio::fs::File>,
    crc: u32,
    read: u64,
    /// Bytes written so far, which is also the offset of the next entry.
    offset: u64,
    records: Vec<CentralRec>,
    pending: Vec<u8>,
    pending_pos: usize,
    phase: ZipPhase,
    central_ready: bool,
    dead: bool,
}

/// One validated photo from a batch request.
struct BatchSource {
    subpath: String,
    path: PathBuf,
    size: u64,
    dos_time: u16,
    dos_date: u16,
}

impl BatchSource {
    fn zip_part(self) -> ZipPart {
        ZipPart {
            name: self.subpath,
            path: self.path,
            size: self.size,
            dos_time: self.dos_time,
            dos_date: self.dos_date,
        }
    }

    fn export_source(self) -> ExportSource {
        ExportSource {
            subpath: self.subpath,
            path: self.path,
            dos_time: self.dos_time,
            dos_date: self.dos_date,
        }
    }
}

/// Validate a batch request and resolve every photo to a confined host path.
///
/// This is the whole authorisation story for both batch endpoints: album access,
/// path confinement, symlink escape, duplicate removal. The caller only decides
/// the cap, the error key for it, and whether non-images may be included —
/// download returns any file, export can only produce images.
#[allow(clippy::too_many_arguments)]
async fn collect_batch_sources(
    cfg: &AppState,
    username: &str,
    user_root: &Path,
    owner: &str,
    albumname: &str,
    photos: &[String],
    limit: usize,
    limit_error: &str,
    require_image: bool,
    lang: &str,
) -> Result<Vec<BatchSource>, AppError> {
    let Some(owner) = Owner::new(owner) else {
        return Err(AppError::BadRequest(tr(
            lang,
            "error.invalid_album_identifier",
        )));
    };
    let Some(album_name) = AlbumName::new(albumname) else {
        return Err(AppError::BadRequest(tr(
            lang,
            "error.invalid_album_identifier",
        )));
    };
    let album = AlbumRef::new(owner, album_name);
    if photos.is_empty() {
        return Err(AppError::BadRequest(tr(
            lang,
            "error.download_photos_empty",
        )));
    }
    if photos.len() > limit {
        return Err(AppError::BadRequest(trf(
            lang,
            limit_error,
            &[("value", limit.to_string())],
        )));
    }

    let resolved = resolve_album_access(
        cfg,
        username,
        user_root,
        &album_access_input(username, album.owner().as_str(), album.name().as_str()),
        lang,
    )
    .await?;
    if resolved.owner != album.owner().as_str() || resolved.album_name != album.name().as_str() {
        return Err(AppError::Forbidden(tr(lang, "error.album_access_denied")));
    }

    let album_dir = albums_root(resolved.root_path.as_path()).join(album.name());
    let album_canon = tokio::fs::canonicalize(&album_dir).await.map_err(|_| {
        AppError::NotFound(trf(
            lang,
            "error.resource_not_found",
            &[("value", albumname.to_string())],
        ))
    })?;

    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    for raw in photos {
        let Some(rel) = PhotoRel::new(raw) else {
            return Err(AppError::BadRequest(trf(
                lang,
                "error.download_bad_path",
                &[("value", raw.clone())],
            )));
        };
        if !seen.insert(rel.as_str().to_string()) {
            continue;
        }
        let photo = PhotoRef::new(album.clone(), rel);
        let subpath = photo.rel().as_str().to_string();
        let host = into_host_path_safely(resolved.root_path.as_path(), &photo.api_path())?;
        if !host.starts_with(&album_dir) {
            return Err(AppError::BadRequest(trf(
                lang,
                "error.download_bad_path",
                &[("value", raw.clone())],
            )));
        }
        let canon = tokio::fs::canonicalize(&host).await.map_err(|_| {
            AppError::NotFound(trf(
                lang,
                "error.resource_not_found",
                &[("value", subpath.clone())],
            ))
        })?;
        if !canon.starts_with(&album_canon) {
            return Err(AppError::Forbidden(tr(lang, "error.invalid_shared_path")));
        }
        let meta = tokio::fs::metadata(&canon).await.map_err(|_| {
            AppError::NotFound(trf(
                lang,
                "error.resource_not_found",
                &[("value", subpath.clone())],
            ))
        })?;
        if !meta.is_file() {
            return Err(AppError::BadRequest(trf(
                lang,
                "error.download_not_a_file",
                &[("value", subpath.clone())],
            )));
        }
        if require_image && !is_image_path(&canon) {
            return Err(AppError::BadRequest(trf(
                lang,
                "error.export_not_image",
                &[("value", subpath.clone())],
            )));
        }
        let (dos_time, dos_date) =
            dos_timestamp(meta.modified().unwrap_or_else(|_| SystemTime::now()));
        sources.push(BatchSource {
            subpath,
            path: canon,
            size: meta.len(),
            dos_time,
            dos_date,
        });
    }
    if sources.is_empty() {
        return Err(AppError::BadRequest(tr(
            lang,
            "error.download_photos_empty",
        )));
    }
    Ok(sources)
}

pub async fn api_albums_download(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<DownloadRequest>,
) -> Result<Json<Value>, AppError> {
    let lang = locale.as_str();
    let Some((username, user_root)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(lang));
    };
    let owner = payload.owner.trim();
    let albumname = payload.albumname.trim();
    let sources = collect_batch_sources(
        &cfg,
        &username,
        user_root.as_path(),
        owner,
        albumname,
        &payload.photos,
        MAX_PHOTOS,
        "error.download_too_many",
        false,
        lang,
    )
    .await?;
    let files: Vec<ZipPart> = sources.into_iter().map(BatchSource::zip_part).collect();

    let zip64 = needs_zip64(&files);
    let content_length = zip_content_length(&files, zip64);
    let filename = zip_download_name(albumname);
    let id = Uuid::new_v4().to_string();
    {
        let mut tickets = TICKETS.lock().unwrap_or_else(|err| err.into_inner());
        purge_tickets(&mut tickets);
        tickets.insert(
            id.clone(),
            Ticket {
                viewer: username,
                owner: owner.to_string(),
                album_name: albumname.to_string(),
                filename,
                files,
                zip64,
                content_length,
                expires: Instant::now() + TICKET_TTL,
            },
        );
    }

    Ok(create_response(
        true,
        &tr(lang, "response.success"),
        0,
        Some(json!({ "id": id })),
    ))
}

pub async fn api_albums_download_file(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Response, AppError> {
    let lang = locale.as_str();
    let Some((username, user_root)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(lang));
    };
    let Ok(parsed) = Uuid::parse_str(id.trim()) else {
        return Err(AppError::NotFound(tr(
            lang,
            "error.download_ticket_invalid",
        )));
    };
    let id = parsed.to_string();
    let ticket = {
        let mut tickets = TICKETS.lock().unwrap_or_else(|err| err.into_inner());
        purge_tickets(&mut tickets);
        let Some(ticket) = tickets.get(&id) else {
            return Err(AppError::NotFound(tr(
                lang,
                "error.download_ticket_invalid",
            )));
        };
        if ticket.viewer != username {
            return Err(AppError::Forbidden(tr(lang, "error.album_access_denied")));
        }
        tickets.remove(&id).expect("ticket was just checked")
    };

    let resolved = resolve_album_access(
        &cfg,
        &username,
        &user_root,
        &album_access_input(&username, &ticket.owner, &ticket.album_name),
        lang,
    )
    .await?;
    if resolved.owner != ticket.owner || resolved.album_name != ticket.album_name {
        return Err(AppError::Forbidden(tr(lang, "error.album_access_denied")));
    }

    let filename = ticket.filename.clone();
    let length = ticket.content_length;
    let zip64 = ticket.zip64;
    let files = ticket.files.clone();
    let body = Body::from_stream(zip_body(ZipPlan::Files {
        parts: files,
        index: 0,
        zip64,
    }));

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );
    headers.insert(CONTENT_DISPOSITION, attachment_value(&filename));

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .body(body)
        .map_err(|err| AppError::Unknown(format!("zip response failed: {err}")))?;
    *response.headers_mut() = headers;
    Ok(response)
}

fn album_access_input(viewer: &str, owner: &str, albumname: &str) -> String {
    if owner == viewer {
        albumname.to_string()
    } else {
        format!("{owner}/{albumname}")
    }
}

fn zip_download_name(album: &str) -> String {
    let mut name: String = album
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .collect();
    if name.is_empty() {
        name = "album".to_string();
    }
    name.push_str(".zip");
    name
}

fn attachment_value(file_name: &str) -> HeaderValue {
    let fallback: String = file_name
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_control() && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let fallback = if fallback.trim_matches('_').is_empty() {
        "download.zip"
    } else {
        fallback.as_str()
    };
    let encoded = utf8_percent_encode(file_name, NON_ALPHANUMERIC);
    let value = format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}");
    HeaderValue::from_str(&value)
        .unwrap_or_else(|_| HeaderValue::from_static("attachment; filename=\"download.zip\""))
}

fn purge_tickets(tickets: &mut HashMap<String, Ticket>) {
    let now = Instant::now();
    tickets.retain(|_, ticket| ticket.expires > now);
    if tickets.len() <= MAX_TICKETS {
        return;
    }
    let mut order: Vec<(Instant, String)> = tickets
        .iter()
        .map(|(id, ticket)| (ticket.expires, id.clone()))
        .collect();
    order.sort_by_key(|(expires, _)| *expires);
    let extra = tickets.len() - MAX_TICKETS;
    for (_, id) in order.into_iter().take(extra) {
        tickets.remove(&id);
    }
}

fn needs_zip64(files: &[ZipPart]) -> bool {
    let mut total = 0u64;
    for file in files {
        if file.size >= u32::MAX as u64 {
            return true;
        }
        let overhead = local_header_len(&file.name, false)
            + descriptor_len(false)
            + central_header_len(&file.name, false);
        total = total.saturating_add(file.size).saturating_add(overhead);
        if total >= u32::MAX as u64 {
            return true;
        }
    }
    false
}

fn dos_timestamp(modified: SystemTime) -> (u16, u16) {
    let dt = chrono::DateTime::<chrono::Utc>::from(modified);
    let year = dt.year().clamp(1980, 2107) as u16;
    let month = dt.month().clamp(1, 12) as u16;
    let day = dt.day().clamp(1, 31) as u16;
    let time = ((dt.hour() as u16) << 11) | ((dt.minute() as u16) << 5) | (dt.second() as u16 / 2);
    let date = ((year - 1980) << 9) | (month << 5) | day;
    (time, date)
}

fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    let mut crc = !crc;
    for &byte in data {
        crc = CRC_TABLE[((crc ^ byte as u32) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

fn local_header_len(name: &str, zip64: bool) -> u64 {
    30 + name.len() as u64 + if zip64 { 20 } else { 0 }
}

fn descriptor_len(zip64: bool) -> u64 {
    if zip64 { 24 } else { 16 }
}

fn central_header_len(name: &str, zip64: bool) -> u64 {
    46 + name.len() as u64 + if zip64 { 28 } else { 0 }
}

fn eocd_len(zip64: bool) -> u64 {
    if zip64 { 98 } else { 22 }
}

fn zip_content_length(files: &[ZipPart], zip64: bool) -> u64 {
    let mut len = eocd_len(zip64);
    for file in files {
        len += local_header_len(&file.name, zip64)
            + file.size
            + descriptor_len(zip64)
            + central_header_len(&file.name, zip64);
    }
    len
}

fn zip_body(plan: ZipPlan) -> impl Stream<Item = Result<Bytes, io::Error>> + Send {
    let state = ZipStream::new(plan);
    futures::stream::unfold(state, |mut state| async move {
        if state.dead {
            return None;
        }
        match state.pull().await {
            Ok(Some(bytes)) => Some((Ok(bytes), state)),
            Ok(None) => None,
            Err(err) => {
                state.dead = true;
                Some((Err(err), state))
            }
        }
    })
}

impl ZipStream {
    fn new(plan: ZipPlan) -> Self {
        let zip64 = plan.zip64();
        Self {
            plan,
            zip64,
            current: None,
            reader: None,
            crc: 0,
            read: 0,
            offset: 0,
            records: Vec::new(),
            pending: Vec::new(),
            pending_pos: 0,
            phase: ZipPhase::File,
            central_ready: false,
            dead: false,
        }
    }

    fn queue(&mut self, bytes: Vec<u8>) {
        debug_assert!(self.pending.is_empty());
        self.pending = bytes;
        self.pending_pos = 0;
    }

    fn take_pending(&mut self) -> Option<Bytes> {
        if self.pending_pos >= self.pending.len() {
            return None;
        }
        let end = (self.pending_pos + CHUNK).min(self.pending.len());
        let bytes = Bytes::copy_from_slice(&self.pending[self.pending_pos..end]);
        self.pending_pos = end;
        if self.pending_pos == self.pending.len() {
            self.pending.clear();
            self.pending_pos = 0;
        }
        self.offset += bytes.len() as u64;
        Some(bytes)
    }

    fn finish_entry(&mut self) {
        let crc = self.crc;
        let size = self.read;
        if let Some(record) = self.records.last_mut() {
            record.crc = crc;
            record.size = size;
        }
        let mut descriptor = Vec::new();
        write_descriptor(&mut descriptor, crc, size, self.zip64);
        self.queue(descriptor);
        self.reader = None;
        if let Some(entry) = self.current.take()
            && entry.temp
        {
            let _ = std::fs::remove_file(&entry.path);
        }
    }

    async fn pull(&mut self) -> io::Result<Option<Bytes>> {
        loop {
            if let Some(bytes) = self.take_pending() {
                return Ok(Some(bytes));
            }
            match self.phase {
                ZipPhase::Done => return Ok(None),
                ZipPhase::Central => {
                    if !self.central_ready {
                        self.queue(build_central(&self.records, self.offset, self.zip64));
                        self.central_ready = true;
                        continue;
                    }
                    self.phase = ZipPhase::Done;
                    return Ok(None);
                }
                ZipPhase::File => {
                    if self.reader.is_none() {
                        if self.current.is_some() {
                            self.finish_entry();
                            continue;
                        }
                        let Some(entry) = self.plan.next_entry().await? else {
                            self.phase = ZipPhase::Central;
                            continue;
                        };
                        let mut header = Vec::new();
                        write_local(
                            &mut header,
                            &entry.name,
                            entry.dos_time,
                            entry.dos_date,
                            self.zip64,
                        );
                        self.queue(header);
                        self.records.push(CentralRec {
                            name: entry.name.clone(),
                            crc: 0,
                            size: entry.size,
                            offset: self.offset,
                            dos_time: entry.dos_time,
                            dos_date: entry.dos_date,
                        });
                        self.crc = 0;
                        self.read = 0;
                        // Track the entry before opening it: if the open fails the
                        // stream is dropped and an owned temp file must still go.
                        let path = entry.path.clone();
                        self.current = Some(entry);
                        self.reader = Some(tokio::fs::File::open(&path).await?);
                        continue;
                    }
                    let size = self.current.as_ref().map(|entry| entry.size).unwrap_or(0);
                    if self.read < size {
                        let want = ((size - self.read) as usize).min(CHUNK);
                        let mut buf = vec![0u8; want];
                        let mut got = 0;
                        while got < want {
                            let read = self.reader.as_mut().unwrap().read(&mut buf[got..]).await?;
                            if read == 0 {
                                return Err(io::Error::new(
                                    io::ErrorKind::UnexpectedEof,
                                    "file changed size while zipping",
                                ));
                            }
                            got += read;
                        }
                        self.crc = crc32_update(self.crc, &buf);
                        self.read += got as u64;
                        self.offset += got as u64;
                        return Ok(Some(Bytes::from(buf)));
                    }
                    self.finish_entry();
                }
            }
        }
    }
}

impl Drop for ZipStream {
    fn drop(&mut self) {
        self.reader.take();
        if let Some(entry) = self.current.take()
            && entry.temp
        {
            let _ = std::fs::remove_file(&entry.path);
        }
    }
}

fn push_u16(buf: &mut Vec<u8>, value: u16) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(buf: &mut Vec<u8>, value: u32) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(buf: &mut Vec<u8>, value: u64) {
    buf.extend_from_slice(&value.to_le_bytes());
}

fn write_local(buf: &mut Vec<u8>, name: &str, dos_time: u16, dos_date: u16, zip64: bool) {
    let name = name.as_bytes();
    push_u32(buf, 0x0403_4B50);
    push_u16(buf, if zip64 { 45 } else { 20 });
    push_u16(buf, GP_FLAGS);
    push_u16(buf, 0);
    push_u16(buf, dos_time);
    push_u16(buf, dos_date);
    push_u32(buf, 0);
    if zip64 {
        push_u32(buf, 0xFFFF_FFFF);
        push_u32(buf, 0xFFFF_FFFF);
        push_u16(buf, name.len() as u16);
        push_u16(buf, 20);
        buf.extend_from_slice(name);
        push_u16(buf, 0x0001);
        push_u16(buf, 16);
        push_u64(buf, 0);
        push_u64(buf, 0);
    } else {
        push_u32(buf, 0);
        push_u32(buf, 0);
        push_u16(buf, name.len() as u16);
        push_u16(buf, 0);
        buf.extend_from_slice(name);
    }
}

fn write_descriptor(buf: &mut Vec<u8>, crc: u32, size: u64, zip64: bool) {
    push_u32(buf, 0x0807_4B50);
    push_u32(buf, crc);
    if zip64 {
        push_u64(buf, size);
        push_u64(buf, size);
    } else {
        push_u32(buf, size as u32);
        push_u32(buf, size as u32);
    }
}

fn write_central(buf: &mut Vec<u8>, rec: &CentralRec, zip64: bool) {
    let name = rec.name.as_bytes();
    push_u32(buf, 0x0201_4B50);
    push_u16(buf, if zip64 { 45 } else { 20 });
    push_u16(buf, if zip64 { 45 } else { 20 });
    push_u16(buf, GP_FLAGS);
    push_u16(buf, 0);
    push_u16(buf, rec.dos_time);
    push_u16(buf, rec.dos_date);
    push_u32(buf, rec.crc);
    if zip64 {
        push_u32(buf, 0xFFFF_FFFF);
        push_u32(buf, 0xFFFF_FFFF);
    } else {
        push_u32(buf, rec.size as u32);
        push_u32(buf, rec.size as u32);
    }
    push_u16(buf, name.len() as u16);
    push_u16(buf, if zip64 { 28 } else { 0 });
    push_u16(buf, 0);
    push_u16(buf, 0);
    push_u16(buf, 0);
    push_u32(buf, 0);
    if zip64 {
        push_u32(buf, 0xFFFF_FFFF);
    } else {
        push_u32(buf, rec.offset as u32);
    }
    buf.extend_from_slice(name);
    if zip64 {
        push_u16(buf, 0x0001);
        push_u16(buf, 24);
        push_u64(buf, rec.size);
        push_u64(buf, rec.size);
        push_u64(buf, rec.offset);
    }
}

fn build_central(records: &[CentralRec], cd_offset: u64, zip64: bool) -> Vec<u8> {
    let mut buf = Vec::new();
    for rec in records {
        write_central(&mut buf, rec, zip64);
    }
    let cd_size = buf.len() as u64;
    let count = records.len() as u64;
    if zip64 {
        push_u32(&mut buf, 0x0606_4B50);
        push_u64(&mut buf, 44);
        push_u16(&mut buf, 45);
        push_u16(&mut buf, 45);
        push_u32(&mut buf, 0);
        push_u32(&mut buf, 0);
        push_u64(&mut buf, count);
        push_u64(&mut buf, count);
        push_u64(&mut buf, cd_size);
        push_u64(&mut buf, cd_offset);
        push_u32(&mut buf, 0x0706_4B50);
        push_u32(&mut buf, 0);
        push_u64(&mut buf, cd_offset + cd_size);
        push_u32(&mut buf, 1);
    }
    push_u32(&mut buf, 0x0605_4B50);
    push_u16(&mut buf, 0);
    push_u16(&mut buf, 0);
    if zip64 || count > u16::MAX as u64 {
        push_u16(&mut buf, 0xFFFF);
        push_u16(&mut buf, 0xFFFF);
    } else {
        push_u16(&mut buf, count as u16);
        push_u16(&mut buf, count as u16);
    }
    if zip64 || cd_size > u32::MAX as u64 {
        push_u32(&mut buf, 0xFFFF_FFFF);
    } else {
        push_u32(&mut buf, cd_size as u32);
    }
    if zip64 || cd_offset > u32::MAX as u64 {
        push_u32(&mut buf, 0xFFFF_FFFF);
    } else {
        push_u32(&mut buf, cd_offset as u32);
    }
    push_u16(&mut buf, 0);
    buf
}

const MAX_EXPORT_TICKETS: usize = 64;

static EXPORT_TICKETS: LazyLock<Mutex<HashMap<String, ExportTicket>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

struct ExportTicket {
    viewer: String,
    owner: String,
    album_name: String,
    filename: String,
    sources: Vec<ExportSource>,
    out_ext: String,
    quality: u32,
    max_width: u32,
    apply_xmp: bool,
    apply_grade: bool,
    limiter: Arc<ConcurrencyLimiter>,
    expires: Instant,
}

struct ExportSource {
    subpath: String,
    path: PathBuf,
    dos_time: u16,
    dos_date: u16,
}

#[derive(Debug, Deserialize)]
pub struct ExportBatchRequest {
    owner: String,
    albumname: String,
    photos: Vec<String>,
    format: String,
    #[serde(default)]
    quality: Option<u32>,
    #[serde(default)]
    max_width: Option<u32>,
    #[serde(default)]
    apply_xmp: Option<bool>,
    #[serde(default)]
    apply_grade: Option<bool>,
}

pub async fn api_albums_export_batch(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    payload: Json<ExportBatchRequest>,
) -> Result<Json<Value>, AppError> {
    let lang = locale.as_str();
    let Some((username, user_root)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(lang));
    };
    let owner = payload.owner.trim();
    let albumname = payload.albumname.trim();
    let Some(out_ext) = export_extension(&payload.format) else {
        return Err(AppError::BadRequest(tr(
            lang,
            "error.unsupported_export_format",
        )));
    };
    let quality = payload.quality.unwrap_or(95).clamp(1, 100);
    let max_width = payload.max_width.unwrap_or(0);
    // Same default as the single-photo export: XMP grading is opt-in.
    let apply_xmp = payload.apply_xmp.unwrap_or(DEFAULT_APPLY_XMP);
    let apply_grade = payload.apply_grade.unwrap_or(false);

    let sources = collect_batch_sources(
        &cfg,
        &username,
        user_root.as_path(),
        owner,
        albumname,
        &payload.photos,
        MAX_EXPORT_PHOTOS,
        "error.export_too_many",
        true,
        lang,
    )
    .await?;

    if !apply_grade
        && sources
            .iter()
            .any(|source| photo_needs_darktable(&source.path, out_ext, max_width, apply_xmp))
        && !darktable_installed()
    {
        return Err(AppError::BadRequest(tr(
            lang,
            "error.darktable_not_available",
        )));
    }
    let sources: Vec<ExportSource> = sources
        .into_iter()
        .map(BatchSource::export_source)
        .collect();

    let id = Uuid::new_v4().to_string();
    {
        let mut tickets = EXPORT_TICKETS.lock().unwrap_or_else(|err| err.into_inner());
        purge_export_tickets(&mut tickets);
        tickets.insert(
            id.clone(),
            ExportTicket {
                viewer: username,
                owner: owner.to_string(),
                album_name: albumname.to_string(),
                filename: zip_download_name(albumname),
                sources,
                out_ext: out_ext.to_string(),
                quality,
                max_width,
                apply_xmp,
                apply_grade,
                limiter: cfg.darktable_limiter.clone(),
                expires: Instant::now() + TICKET_TTL,
            },
        );
    }

    Ok(create_response(
        true,
        &tr(lang, "response.success"),
        0,
        Some(json!({ "id": id })),
    ))
}

pub async fn api_albums_export_batch_file(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    AxumPath(id): AxumPath<String>,
) -> Result<Response, AppError> {
    let lang = locale.as_str();
    let Some((username, user_root)) = cfg.get_session_and_root(&token).await else {
        return Err(AppError::unauthorized(lang));
    };
    let Ok(parsed) = Uuid::parse_str(id.trim()) else {
        return Err(AppError::NotFound(tr(lang, "error.export_ticket_invalid")));
    };
    let id = parsed.to_string();
    let ticket = {
        let mut tickets = EXPORT_TICKETS.lock().unwrap_or_else(|err| err.into_inner());
        purge_export_tickets(&mut tickets);
        let Some(ticket) = tickets.get(&id) else {
            return Err(AppError::NotFound(tr(lang, "error.export_ticket_invalid")));
        };
        if ticket.viewer != username {
            return Err(AppError::Forbidden(tr(lang, "error.album_access_denied")));
        }
        tickets.remove(&id).expect("export ticket was just checked")
    };

    let resolved = resolve_album_access(
        &cfg,
        &username,
        &user_root,
        &album_access_input(&username, &ticket.owner, &ticket.album_name),
        lang,
    )
    .await?;
    if resolved.owner != ticket.owner || resolved.album_name != ticket.album_name {
        return Err(AppError::Forbidden(tr(lang, "error.album_access_denied")));
    }

    let filename = ticket.filename.clone();
    let body = Body::from_stream(zip_body(ZipPlan::Export {
        ticket,
        index: 0,
        used_names: HashSet::new(),
    }));
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(CONTENT_DISPOSITION, attachment_value(&filename));

    let mut response = Response::builder()
        .status(StatusCode::OK)
        .body(body)
        .map_err(|err| AppError::Unknown(format!("export response failed: {err}")))?;
    *response.headers_mut() = headers;
    Ok(response)
}

fn export_extension(format: &str) -> Option<&'static str> {
    match format.trim().to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => Some("jpg"),
        "png" => Some("png"),
        "tiff" | "tif" => Some("tiff"),
        "webp" => Some("webp"),
        _ => None,
    }
}

fn same_export_format(path: &Path, out_ext: &str) -> bool {
    let src = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(
        (src.as_str(), out_ext),
        ("jpg" | "jpeg", "jpg") | ("png", "png") | ("tif" | "tiff", "tiff") | ("webp", "webp")
    )
}

fn photo_needs_darktable(path: &Path, out_ext: &str, max_width: u32, apply_xmp: bool) -> bool {
    if apply_xmp && find_xmp_sidecar(path).is_some() {
        return true;
    }
    !(max_width == 0 && same_export_format(path, out_ext))
}

fn darktable_installed() -> bool {
    std::process::Command::new("darktable-cli")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn purge_export_tickets(tickets: &mut HashMap<String, ExportTicket>) {
    let now = Instant::now();
    tickets.retain(|_, ticket| ticket.expires > now);
    if tickets.len() <= MAX_EXPORT_TICKETS {
        return;
    }
    let mut order: Vec<(Instant, String)> = tickets
        .iter()
        .map(|(id, ticket)| (ticket.expires, id.clone()))
        .collect();
    order.sort_by_key(|(expires, _)| *expires);
    let extra = tickets.len() - MAX_EXPORT_TICKETS;
    for (_, id) in order.into_iter().take(extra) {
        tickets.remove(&id);
    }
}

fn zip_join(dir: &str, file: &str) -> String {
    if dir.is_empty() {
        file.to_string()
    } else {
        format!("{dir}/{file}")
    }
}

fn next_entry_name(
    used: &mut HashSet<String>,
    subpath: &str,
    out_ext: &str,
    edited: bool,
) -> String {
    let (dir, file) = match subpath.rsplit_once('/') {
        Some((dir, file)) => (dir, file),
        None => ("", subpath),
    };
    let stem = file.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(file);
    let stem = if stem.is_empty() { "photo" } else { stem };
    let base = if edited {
        format!("{stem}_edited")
    } else {
        stem.to_string()
    };
    let mut candidate = zip_join(dir, &format!("{base}.{out_ext}"));
    let mut n = 2u32;
    while used.contains(&candidate) {
        candidate = zip_join(dir, &format!("{base}-{n}.{out_ext}"));
        n += 1;
    }
    used.insert(candidate.clone());
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    async fn collect_zip(dir: &std::path::Path, zip64: bool) -> Vec<u8> {
        let hello = dir.join("a.jpg");
        let nested = dir.join("sub").join("b.jpg");
        tokio::fs::create_dir_all(dir.join("sub")).await.unwrap();
        tokio::fs::write(&hello, b"hello").await.unwrap();
        tokio::fs::write(&nested, b"world!").await.unwrap();
        let files = vec![
            ZipPart {
                name: "a.jpg".into(),
                path: hello,
                size: 5,
                dos_time: 0,
                dos_date: 33,
            },
            ZipPart {
                name: "sub/b.jpg".into(),
                path: nested,
                size: 6,
                dos_time: 0,
                dos_date: 33,
            },
        ];
        let expected = zip_content_length(&files, zip64);
        let mut stream = Box::pin(zip_body(ZipPlan::Files {
            parts: files,
            index: 0,
            zip64,
        }));
        let mut out = Vec::new();
        while let Some(chunk) = stream.next().await {
            out.extend_from_slice(&chunk.unwrap());
        }
        assert_eq!(out.len() as u64, expected);
        out
    }

    #[tokio::test]
    async fn stored_zip32_contains_payloads_and_directory() {
        let dir = std::env::temp_dir().join(format!("lumeno-zip32-{}", Uuid::new_v4()));
        let out = collect_zip(&dir, false).await;
        assert_eq!(&out[0..4], &0x0403_4B50u32.to_le_bytes());
        assert!(out.windows(5).any(|window| window == b"hello"));
        assert!(out.windows(6).any(|window| window == b"world!"));
        assert_eq!(
            &out[out.len() - 22..out.len() - 18],
            &0x0605_4B50u32.to_le_bytes()
        );
        let hello_crc = crc32_update(0, b"hello");
        assert!(
            out.windows(4)
                .any(|window| window == hello_crc.to_le_bytes())
        );
        let zip_path = dir.join("out.zip");
        std::fs::write(&zip_path, &out).unwrap();
        let python = std::process::Command::new("python3")
            .arg("-c")
            .arg(
                "import sys, zipfile; z=zipfile.ZipFile(sys.argv[1]); \
                 assert z.read('a.jpg')==b'hello'; assert z.read('sub/b.jpg')==b'world!'",
            )
            .arg(&zip_path)
            .status();
        if let Ok(status) = python {
            assert!(
                status.success(),
                "zipfile could not read the stored archive"
            );
        }
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn stored_zip64_length_matches_and_ends_with_records() {
        let dir = std::env::temp_dir().join(format!("lumeno-zip64-{}", Uuid::new_v4()));
        let out = collect_zip(&dir, true).await;
        assert!(
            out.windows(4)
                .any(|window| window == 0x0606_4B50u32.to_le_bytes())
        );
        assert!(
            out.windows(4)
                .any(|window| window == 0x0706_4B50u32.to_le_bytes())
        );
        assert_eq!(
            &out[out.len() - 22..out.len() - 18],
            &0x0605_4B50u32.to_le_bytes()
        );
        let zip_path = dir.join("out.zip");
        std::fs::write(&zip_path, &out).unwrap();
        let python = std::process::Command::new("python3")
            .arg("-c")
            .arg(
                "import sys, zipfile; z=zipfile.ZipFile(sys.argv[1]); \
                 assert z.read('a.jpg')==b'hello'; assert z.read('sub/b.jpg')==b'world!'",
            )
            .arg(&zip_path)
            .status();
        if let Ok(status) = python {
            assert!(status.success(), "zipfile could not read the zip64 archive");
        }
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[test]
    fn export_names_keep_folders_and_avoid_collisions() {
        let mut used = HashSet::new();
        assert_eq!(
            next_entry_name(&mut used, "2024/a.RAF", "jpg", true),
            "2024/a_edited.jpg"
        );
        assert_eq!(
            next_entry_name(&mut used, "2024/a.jpeg", "jpg", false),
            "2024/a.jpg"
        );
        assert_eq!(
            next_entry_name(&mut used, "2024/a.jpg", "jpg", false),
            "2024/a-2.jpg"
        );
    }

    #[tokio::test]
    async fn export_stream_copies_matching_files_without_darktable() {
        let dir = std::env::temp_dir().join(format!("lumeno-export-{}", Uuid::new_v4()));
        tokio::fs::create_dir_all(dir.join("sub")).await.unwrap();
        let hello = dir.join("a.jpg");
        let nested = dir.join("sub").join("b.jpg");
        tokio::fs::write(&hello, b"hello").await.unwrap();
        tokio::fs::write(&nested, b"world!").await.unwrap();
        let settings = std::sync::Arc::new(crate::backend::settings::SettingsManager::new(
            std::env::temp_dir().join(format!("lumeno-limiter-{}.json", Uuid::new_v4())),
        ));
        let limiter = ConcurrencyLimiter::for_thumbnails(&settings).await;
        let ticket = ExportTicket {
            viewer: "alice".into(),
            owner: "alice".into(),
            album_name: "Trip".into(),
            filename: "Trip.zip".into(),
            sources: vec![
                ExportSource {
                    subpath: "a.jpg".into(),
                    path: hello,
                    dos_time: 0,
                    dos_date: 33,
                },
                ExportSource {
                    subpath: "sub/b.jpg".into(),
                    path: nested,
                    dos_time: 0,
                    dos_date: 33,
                },
            ],
            out_ext: "jpg".into(),
            quality: 95,
            max_width: 0,
            // Same format and size: the entries are copied straight through.
            apply_xmp: false,
            apply_grade: false,
            limiter,
            expires: Instant::now() + TICKET_TTL,
        };
        let mut stream = Box::pin(zip_body(ZipPlan::Export {
            ticket,
            index: 0,
            used_names: HashSet::new(),
        }));
        let mut out = Vec::new();
        while let Some(chunk) = stream.next().await {
            out.extend_from_slice(&chunk.unwrap());
        }
        assert!(out.windows(5).any(|window| window == b"hello"));
        assert!(out.windows(6).any(|window| window == b"world!"));
        let zip_path = dir.join("out.zip");
        std::fs::write(&zip_path, &out).unwrap();
        let python = std::process::Command::new("python3")
            .arg("-c")
            .arg(
                "import sys, zipfile; z=zipfile.ZipFile(sys.argv[1]); \
                 assert z.read('a.jpg')==b'hello'; assert z.read('sub/b.jpg')==b'world!'",
            )
            .arg(&zip_path)
            .status()
            .unwrap();
        assert!(python.success(), "zipfile could not read the export stream");
        tokio::fs::remove_dir_all(&dir).await.ok();
    }
}
