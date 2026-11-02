//! Whole-file video transcodes cached under `cache/video-encode/{owner}/{codec}/`.

use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{Path, Query};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::{Extension, Json};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use function_name::named;
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha512};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};

use crate::backend::common::{AppError, AppState, Token, create_response, into_host_path_safely};
use crate::backend::files::send_media_range;
use crate::backend::i18n::{RequestLocale, tr, trf};
use crate::backend::tasks::{Task, TaskBase, TaskStatus, task_file_name};

use super::media::{cache_relative_path, is_video_path, resolve_photo_owner_root};

const ENCODE_CODECS: &[&str] = &["av1", "hevc", "h264"];
const PROBE_VERSION: u32 = 1;

const PATH_ENCODE: &AsciiSet = &NON_ALPHANUMERIC.remove(b'/');

static ENCODE_SLOT: OnceLock<Arc<Semaphore>> = OnceLock::new();
static ENCODERS: OnceLock<EncoderSet> = OnceLock::new();

fn encode_slot() -> Arc<Semaphore> {
    ENCODE_SLOT
        .get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VideoSourceInfo {
    pub video_codec: String,
    pub profile: String,
    pub bit_depth: u32,
    pub pix_fmt: String,
    pub audio_codec: String,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u64,
    pub label: String,
}

#[derive(Debug, Clone)]
struct EncoderSet {
    h264_vaapi: bool,
    hevc_vaapi: bool,
    libx264: bool,
    libx265: bool,
    libsvtav1: bool,
    libaom: bool,
    vaapi_device: Option<String>,
}

#[derive(Debug, Clone)]
enum EncoderKind {
    Vaapi {
        name: String,
        device: String,
        tag: Option<String>,
    },
    Software {
        name: String,
        extra: Vec<String>,
        tag: Option<String>,
    },
}

#[derive(Debug, Clone)]
struct EncodePlan {
    input: String,
    output: String,
    encoder: EncoderKind,
    has_audio: bool,
}

#[derive(Debug, Deserialize)]
pub struct OwnerQuery {
    #[serde(default)]
    owner: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VideoEncodePayload {
    path: String,
    #[serde(default)]
    owner: Option<String>,
    codec: String,
}

struct ResolvedVideo {
    username: String,
    owner: String,
    relative: String,
    source: PathBuf,
    key: String,
}

pub(crate) fn is_encode_codec(codec: &str) -> bool {
    ENCODE_CODECS.contains(&codec)
}

pub(crate) fn codec_label(codec: &str) -> String {
    match codec {
        "h264" => "H.264".to_owned(),
        "hevc" | "h265" => "HEVC".to_owned(),
        "av1" | "av01" => "AV1".to_owned(),
        "prores" => "ProRes".to_owned(),
        "vp9" => "VP9".to_owned(),
        "mpeg4" => "MPEG-4".to_owned(),
        other => other.to_ascii_uppercase(),
    }
}

fn normalize_video_codec(name: &str) -> String {
    match name {
        "h265" | "hevc" => "hevc".to_owned(),
        "av01" => "av1".to_owned(),
        other => other.to_owned(),
    }
}

pub(crate) fn bit_depth_from_pix_fmt(pix: &str) -> u32 {
    let bytes = pix.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index] != b'p' || index + 1 >= bytes.len() || !bytes[index + 1].is_ascii_digit() {
            continue;
        }
        let digits: String = pix[index + 1..]
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect();
        if let Ok(depth) = digits.parse::<u32>()
            && (8..=16).contains(&depth)
        {
            return depth;
        }
    }
    if pix.starts_with("p010") || pix.starts_with("p016") {
        return 10;
    }
    8
}

pub(crate) fn source_cache_key(relative: &str, len: u64, mtime: u64) -> String {
    let mut hasher = Sha512::new();
    hasher.update(format!("video-encode-v1|{relative}|{len}|{mtime}").as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

pub(crate) fn encode_root(data_path: &FsPath, owner: &str) -> Result<PathBuf, AppError> {
    if !super::helpers::is_safe_cache_owner(owner) {
        return Err(AppError::BadRequest("invalid cache owner".to_owned()));
    }
    Ok(crate::backend::cache_areas::area("video-encode")
        .expect("registered cache area")
        .owner_dir(data_path, owner))
}

pub(crate) fn encode_file(
    data_path: &FsPath,
    owner: &str,
    codec: &str,
    key: &str,
) -> Result<PathBuf, AppError> {
    if !is_encode_codec(codec) || key.contains(['/', '\\']) {
        return Err(AppError::BadRequest("invalid encode path".to_owned()));
    }
    Ok(encode_root(data_path, owner)?
        .join(codec)
        .join(format!("{key}.mp4")))
}

pub(crate) fn probe_file(data_path: &FsPath, owner: &str, key: &str) -> Result<PathBuf, AppError> {
    if key.contains(['/', '\\']) {
        return Err(AppError::BadRequest("invalid probe path".to_owned()));
    }
    Ok(encode_root(data_path, owner)?
        .join("probe")
        .join(format!("{key}.json")))
}

fn playback_url(codec: &str, relative: &str, owner: &str, requester: &str) -> String {
    let path = utf8_percent_encode(relative, PATH_ENCODE);
    if owner != requester {
        let owner = utf8_percent_encode(owner, PATH_ENCODE);
        format!("/api/albums/video_file/{codec}/{path}?owner={owner}")
    } else {
        format!("/api/albums/video_file/{codec}/{path}")
    }
}

async fn resolve_video(
    cfg: &AppState,
    token: &Token,
    path: &str,
    owner: Option<&str>,
    locale: &str,
) -> Result<ResolvedVideo, AppError> {
    let Some((username, root_path)) = cfg.get_session_and_root(token).await else {
        return Err(AppError::unauthorized(locale));
    };
    let effective_root =
        resolve_photo_owner_root(cfg, &username, root_path, owner, path, locale).await?;
    let source = into_host_path_safely(effective_root.as_path(), path)?;
    if !source.is_file() {
        return Err(AppError::NotFound(trf(
            locale,
            "error.resource_not_found",
            &[("value", path.to_owned())],
        )));
    }
    if !is_video_path(source.as_path()) {
        return Err(AppError::BadRequest(tr(locale, "error.not_a_video")));
    }
    let metadata = tokio::fs::metadata(&source)
        .await
        .map_err(|_| AppError::NotFound(path.to_owned()))?;
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|time| time.as_secs())
        .unwrap_or(0);
    let file_owner = owner
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| username.clone());
    let relative = cache_relative_path(cfg.data_path.as_path(), source.as_path());
    let key = source_cache_key(&relative, metadata.len(), mtime);
    Ok(ResolvedVideo {
        username,
        owner: file_owner,
        relative: path.trim_start_matches('/').to_owned(),
        source,
        key,
    })
}

pub(crate) fn parse_probe_json(value: &Value) -> Option<VideoSourceInfo> {
    let streams = value.get("streams")?.as_array()?;
    let video = streams
        .iter()
        .find(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("video"))?;
    let audio_codec = streams
        .iter()
        .find(|stream| stream.get("codec_type").and_then(Value::as_str) == Some("audio"))
        .and_then(|stream| stream.get("codec_name").and_then(Value::as_str))
        .unwrap_or("")
        .to_owned();
    let video_codec = normalize_video_codec(
        video
            .get("codec_name")
            .and_then(Value::as_str)
            .unwrap_or(""),
    );
    if video_codec.is_empty() {
        return None;
    }
    let pix_fmt = video
        .get("pix_fmt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let reported = json_u32(video.get("bits_per_raw_sample").unwrap_or(&Value::Null));
    let bit_depth = if reported >= 8 {
        reported
    } else {
        bit_depth_from_pix_fmt(&pix_fmt)
    };
    let duration_ms = duration_ms_from_probe(video, value.get("format"));
    Some(VideoSourceInfo {
        label: codec_label(&video_codec),
        video_codec,
        profile: video
            .get("profile")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
        bit_depth,
        pix_fmt,
        audio_codec,
        width: json_u32(video.get("width").unwrap_or(&Value::Null)),
        height: json_u32(video.get("height").unwrap_or(&Value::Null)),
        duration_ms,
    })
}

fn json_u32(value: &Value) -> u32 {
    match value {
        Value::Number(number) => number.as_u64().unwrap_or(0) as u32,
        Value::String(text) => text.parse().unwrap_or(0),
        _ => 0,
    }
}

fn duration_ms_from_probe(video: &Value, format: Option<&Value>) -> u64 {
    let seconds = video
        .get("duration")
        .and_then(Value::as_str)
        .or_else(|| {
            format
                .and_then(|item| item.get("duration"))
                .and_then(Value::as_str)
        })
        .and_then(|text| text.parse::<f64>().ok())
        .unwrap_or(0.0);
    if seconds <= 0.0 {
        0
    } else {
        (seconds * 1000.0) as u64
    }
}

async fn load_or_probe(
    cfg: &AppState,
    video: &ResolvedVideo,
    locale: &str,
) -> Result<VideoSourceInfo, AppError> {
    let path = probe_file(cfg.data_path.as_path(), &video.owner, &video.key)?;
    if let Ok(bytes) = tokio::fs::read(&path).await
        && let Ok(cached) = serde_json::from_slice::<Value>(&bytes)
        && cached.get("version").and_then(Value::as_u64) == Some(PROBE_VERSION as u64)
        && let Some(info) = cached.get("source").and_then(parse_probe_json_stored)
    {
        return Ok(info);
    }
    let info = run_ffprobe(&video.source, locale).await?;
    if let Some(parent) = path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let stored = json!({ "version": PROBE_VERSION, "source": info });
    if let Ok(bytes) = serde_json::to_vec_pretty(&stored) {
        let _ = tokio::fs::write(&path, bytes).await;
    }
    Ok(info)
}

fn parse_probe_json_stored(value: &Value) -> Option<VideoSourceInfo> {
    serde_json::from_value(value.clone()).ok()
}

async fn run_ffprobe(source: &FsPath, locale: &str) -> Result<VideoSourceInfo, AppError> {
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new("ffprobe")
            .args([
                "-v", "error",
                "-show_entries",
                "stream=codec_type,codec_name,profile,pix_fmt,bits_per_raw_sample,width,height,duration:format=duration",
                "-of", "json",
            ])
            .arg(source)
            .output(),
    )
    .await
    .map_err(|_| AppError::BadRequest(tr(locale, "error.ffmpeg_missing")))?
    .map_err(|_| AppError::BadRequest(tr(locale, "error.ffmpeg_missing")))?;
    if !output.status.success() {
        return Err(AppError::BadRequest(tr(locale, "error.video_probe_failed")));
    }
    let value: Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| AppError::BadRequest(tr(locale, "error.video_probe_failed")))?;
    parse_probe_json(&value)
        .ok_or_else(|| AppError::BadRequest(tr(locale, "error.video_probe_failed")))
}

fn encode_attempts(codec: &str, encoders: &EncoderSet) -> Vec<EncoderKind> {
    let mut attempts = Vec::new();
    match codec {
        "h264" => {
            if encoders.h264_vaapi
                && let Some(device) = &encoders.vaapi_device
            {
                attempts.push(EncoderKind::Vaapi {
                    name: "h264_vaapi".to_owned(),
                    device: device.clone(),
                    tag: None,
                });
            }
            if encoders.libx264 {
                attempts.push(EncoderKind::Software {
                    name: "libx264".to_owned(),
                    extra: vec![
                        "-preset".into(),
                        "veryfast".into(),
                        "-crf".into(),
                        "20".into(),
                    ],
                    tag: None,
                });
            }
        }
        "hevc" => {
            if encoders.hevc_vaapi
                && let Some(device) = &encoders.vaapi_device
            {
                attempts.push(EncoderKind::Vaapi {
                    name: "hevc_vaapi".to_owned(),
                    device: device.clone(),
                    tag: Some("hvc1".into()),
                });
            }
            if encoders.libx265 {
                attempts.push(EncoderKind::Software {
                    name: "libx265".to_owned(),
                    extra: vec![
                        "-preset".into(),
                        "medium".into(),
                        "-crf".into(),
                        "22".into(),
                    ],
                    tag: Some("hvc1".into()),
                });
            }
        }
        "av1" => {
            if encoders.libsvtav1 {
                attempts.push(EncoderKind::Software {
                    name: "libsvtav1".to_owned(),
                    extra: vec!["-preset".into(), "8".into(), "-crf".into(), "32".into()],
                    tag: None,
                });
            }
            if encoders.libaom {
                attempts.push(EncoderKind::Software {
                    name: "libaom-av1".to_owned(),
                    extra: vec![
                        "-cpu-used".into(),
                        "6".into(),
                        "-crf".into(),
                        "32".into(),
                        "-b:v".into(),
                        "0".into(),
                    ],
                    tag: None,
                });
            }
        }
        _ => {}
    }
    attempts
}

fn partial_output(output: &FsPath) -> PathBuf {
    let stem = output
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("video");
    output.with_file_name(format!("{stem}.part.mp4"))
}

fn encode_args(plan: &EncodePlan) -> Vec<String> {
    let mut args = vec![
        "-y".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
    ];
    match &plan.encoder {
        EncoderKind::Vaapi { name, device, tag } => {
            args.extend([
                "-init_hw_device".into(),
                format!("vaapi=va:{device}"),
                "-filter_hw_device".into(),
                "va".into(),
                "-i".into(),
                plan.input.clone(),
                "-vf".into(),
                "format=nv12,hwupload".into(),
                "-c:v".into(),
                name.clone(),
            ]);
            if let Some(tag) = tag {
                args.extend(["-tag:v".into(), tag.clone()]);
            }
        }
        EncoderKind::Software { name, extra, tag } => {
            args.extend(["-i".into(), plan.input.clone(), "-c:v".into(), name.clone()]);
            args.extend(["-pix_fmt".into(), "yuv420p".into()]);
            args.extend(extra.iter().cloned());
            if let Some(tag) = tag {
                args.extend(["-tag:v".into(), tag.clone()]);
            }
        }
    }
    args.extend(["-map".into(), "0:v:0".into()]);
    if plan.has_audio {
        args.extend([
            "-map".into(),
            "0:a:0".into(),
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            "160k".into(),
        ]);
    } else {
        args.push("-an".into());
    }
    args.extend([
        "-sn".into(),
        "-movflags".into(),
        "+faststart".into(),
        "-f".into(),
        "mp4".into(),
        plan.output.clone(),
    ]);
    args
}

fn render_devices() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/dev/dri") else {
        return Vec::new();
    };
    let mut devices: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("renderD")
                .then(|| entry.path().display().to_string())
        })
        .collect();
    devices.sort();
    devices
}

async fn working_vaapi_device() -> Option<String> {
    for device in render_devices() {
        let Ok(output) = Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-init_hw_device",
                &format!("vaapi=va:{device}"),
                "-filter_hw_device",
                "va",
                "-f",
                "lavfi",
                "-i",
                "color=c=black:s=64x64:d=0.04",
                "-vf",
                "format=nv12,hwupload",
                "-c:v",
                "h264_vaapi",
                "-frames:v",
                "1",
                "-f",
                "null",
                "-",
            ])
            .output()
            .await
        else {
            continue;
        };
        if output.status.success() {
            return Some(device);
        }
    }
    None
}

async fn encoder_set() -> EncoderSet {
    if let Some(cached) = ENCODERS.get() {
        return cached.clone();
    }
    let listed = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .await
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();
    let has = |name: &str| {
        listed
            .lines()
            .any(|line| line.split_whitespace().any(|part| part == name))
    };
    let device = working_vaapi_device().await;
    let set = EncoderSet {
        h264_vaapi: device.is_some() && has("h264_vaapi"),
        hevc_vaapi: device.is_some() && has("hevc_vaapi"),
        libx264: has("libx264"),
        libx265: has("libx265"),
        libsvtav1: has("libsvtav1"),
        libaom: has("libaom-av1"),
        vaapi_device: device,
    };
    let _ = ENCODERS.set(set.clone());
    set
}

struct VideoEncodeTask {
    base: TaskBase,
    source: PathBuf,
    output: PathBuf,
    codec: String,
    file_name: String,
    locale: String,
    has_audio: bool,
    duration_ms: u64,
    cancel: AtomicBool,
    child: Mutex<Option<Child>>,
}

impl VideoEncodeTask {
    fn partial_path(&self) -> PathBuf {
        partial_output(self.output.as_path())
    }

    fn line(&self, key: &str) -> String {
        tr(self.locale.as_str(), key)
    }

    fn running_line(&self) -> String {
        trf(
            self.locale.as_str(),
            "task.video_encode_running",
            &[("codec", codec_label(&self.codec))],
        )
    }

    fn public_error(&self, error: &str) -> String {
        error
            .replace(&self.source.display().to_string(), &self.file_name)
            .replace(&self.output.display().to_string(), &self.file_name)
            .replace(&self.partial_path().display().to_string(), &self.file_name)
    }

    async fn acquire_slot(&self) -> Option<OwnedSemaphorePermit> {
        let acquire = encode_slot().acquire_owned();
        tokio::pin!(acquire);
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return None;
            }
            tokio::select! {
                result = &mut acquire => {
                    let Ok(permit) = result else { return None };
                    if self.cancel.load(Ordering::Relaxed) {
                        return None;
                    }
                    return Some(permit);
                }
                _ = tokio::time::sleep(Duration::from_millis(200)) => {}
            }
        }
    }

    async fn run_ffmpeg(&self, encoder: EncoderKind, has_audio: bool) -> Result<(), String> {
        let partial = self.partial_path();
        if let Some(parent) = partial.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| error.to_string())?;
        }
        let _ = tokio::fs::remove_file(&partial).await;
        let plan = EncodePlan {
            input: self.source.display().to_string(),
            output: partial.display().to_string(),
            encoder,
            has_audio,
        };
        let mut child = Command::new("ffmpeg")
            .args(encode_args(&plan))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| error.to_string())?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        *self.child.lock().await = Some(child);
        let stderr_task = tokio::spawn(async move {
            let Some(stderr) = stderr else {
                return String::new();
            };
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            let mut kept = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) if kept.len() < 4000 => kept.push_str(&line),
                    Ok(_) => {}
                }
            }
            kept
        });
        if let Some(stdout) = stdout {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                if self.cancel.load(Ordering::Relaxed) {
                    self.kill_child().await;
                    break;
                }
                line.clear();
                match tokio::time::timeout(Duration::from_millis(300), reader.read_line(&mut line))
                    .await
                {
                    Ok(Ok(0)) | Ok(Err(_)) => break,
                    Err(_) => continue,
                    Ok(Ok(_)) => {
                        if let Some(ms) = progress_ms(line.trim()) {
                            self.base
                                .set_processed_cost(ms.min(self.duration_ms.max(ms)));
                        }
                    }
                }
            }
        }
        let status = if let Some(mut child) = self.child.lock().await.take() {
            child.wait().await.map_err(|error| error.to_string())?
        } else {
            return Err("encoder exited".to_owned());
        };
        let stderr = stderr_task.await.unwrap_or_default();
        if self.cancel.load(Ordering::Relaxed) {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err("cancelled".to_owned());
        }
        if !status.success() {
            let _ = tokio::fs::remove_file(&partial).await;
            return Err(if stderr.trim().is_empty() {
                format!("ffmpeg exited {status}")
            } else {
                stderr
            });
        }
        if tokio::fs::metadata(&partial)
            .await
            .map(|meta| meta.len() > 0)
            .unwrap_or(false)
        {
            if tokio::fs::metadata(&self.output).await.is_ok() {
                let _ = tokio::fs::remove_file(&self.output).await;
            }
            tokio::fs::rename(&partial, &self.output)
                .await
                .map_err(|error| error.to_string())?;
            return Ok(());
        }
        Err(stderr)
    }

    async fn kill_child(&self) {
        if let Some(child) = self.child.lock().await.as_mut() {
            let _ = child.start_kill();
        }
    }

    async fn remove_partial(&self) {
        let _ = tokio::fs::remove_file(self.partial_path()).await;
    }
}

fn progress_ms(line: &str) -> Option<u64> {
    let (key, value) = line.split_once('=')?;
    let value = value.trim();
    match key {
        "out_time" => parse_ffmpeg_clock_ms(value),
        "out_time_us" => value.parse::<u64>().ok().map(|us| us / 1000),
        _ => None,
    }
}

fn parse_ffmpeg_clock_ms(value: &str) -> Option<u64> {
    let mut parts = value.split(':');
    let hours: u64 = parts.next()?.parse().ok()?;
    let minutes: u64 = parts.next()?.parse().ok()?;
    let seconds: f64 = parts.next()?.parse().ok()?;
    Some(hours * 3_600_000 + minutes * 60_000 + (seconds * 1000.0) as u64)
}

#[async_trait]
impl Task for VideoEncodeTask {
    fn base(&self) -> &TaskBase {
        &self.base
    }

    fn deduplication_key(&self) -> Option<String> {
        Some(format!("video-encode:{}", self.output.display()))
    }

    fn cancellable(&self) -> bool {
        true
    }

    async fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.kill_child().await;
        self.remove_partial().await;
        self.base
            .set_cur_task(self.line("task.video_encode_cancelled"))
            .await;
        self.base.set_status(TaskStatus::Cancelled).await;
    }

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Pending).await;
        self.base
            .set_cur_task(self.line("task.video_encode_waiting"))
            .await;
        let Some(_permit) = self.acquire_slot().await else {
            self.remove_partial().await;
            self.base
                .set_cur_task(self.line("task.video_encode_cancelled"))
                .await;
            self.base.set_status(TaskStatus::Cancelled).await;
            return;
        };
        self.base.set_status(TaskStatus::Running).await;
        self.base.set_total_cost(self.duration_ms);
        self.base.set_cur_task(self.running_line()).await;
        let encoders = encoder_set().await;
        let attempts = encode_attempts(&self.codec, &encoders);
        if attempts.is_empty() {
            self.base
                .set_error_msg(trf(
                    self.locale.as_str(),
                    "task.video_encode_no_encoder",
                    &[("codec", codec_label(&self.codec))],
                ))
                .await;
            self.base
                .set_cur_task(self.line("task.video_encode_failed"))
                .await;
            self.base.set_status(TaskStatus::Failed).await;
            return;
        }
        let mut last_error = String::new();
        let audio_passes = if self.has_audio {
            vec![true, false]
        } else {
            vec![false]
        };
        for has_audio in audio_passes {
            for encoder in attempts.clone() {
                if self.cancel.load(Ordering::Relaxed) {
                    self.remove_partial().await;
                    self.base
                        .set_cur_task(self.line("task.video_encode_cancelled"))
                        .await;
                    self.base.set_status(TaskStatus::Cancelled).await;
                    return;
                }
                match self.run_ffmpeg(encoder, has_audio).await {
                    Ok(()) => {
                        self.base.set_processed_cost(self.duration_ms);
                        self.base
                            .set_cur_task(self.line("task.video_encode_finished"))
                            .await;
                        self.base.set_status(TaskStatus::Finished).await;
                        return;
                    }
                    Err(error) if error == "cancelled" || self.cancel.load(Ordering::Relaxed) => {
                        self.remove_partial().await;
                        self.base
                            .set_cur_task(self.line("task.video_encode_cancelled"))
                            .await;
                        self.base.set_status(TaskStatus::Cancelled).await;
                        return;
                    }
                    Err(error) => last_error = error,
                }
            }
            if !has_audio {
                break;
            }
            let lower = last_error.to_ascii_lowercase();
            if !lower.contains("matches no streams") && !lower.contains("stream map") {
                break;
            }
        }
        self.remove_partial().await;
        self.base
            .set_error_msg(self.public_error(&last_error))
            .await;
        self.base
            .set_cur_task(self.line("task.video_encode_failed"))
            .await;
        self.base.set_status(TaskStatus::Failed).await;
    }
}

#[named]
pub async fn api_video_info(
    locale: RequestLocale,
    token: Token,
    Path(path): Path<String>,
    query: Query<OwnerQuery>,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Json<Value>, AppError> {
    let _ = function_name!();
    let video = resolve_video(&cfg, &token, &path, query.owner.as_deref(), locale.as_str()).await?;
    let source = load_or_probe(&cfg, &video, locale.as_str()).await?;
    let mut encodes = Vec::new();
    for codec in ENCODE_CODECS {
        let file = encode_file(cfg.data_path.as_path(), &video.owner, codec, &video.key)?;
        let ready = tokio::fs::metadata(&file)
            .await
            .map(|meta| meta.is_file() && meta.len() > 0)
            .unwrap_or(false);
        encodes.push(json!({
            "codec": codec,
            "label": codec_label(codec),
            "ready": ready,
            "url": playback_url(codec, &video.relative, &video.owner, &video.username),
        }));
    }
    let message = tr(locale.as_str(), "response.ok");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({ "source": source, "encodes": encodes })),
    ))
}

#[named]
pub async fn api_video_encode(
    locale: RequestLocale,
    token: Token,
    Extension(cfg): Extension<Arc<AppState>>,
    Json(payload): Json<VideoEncodePayload>,
) -> Result<Json<Value>, AppError> {
    let _ = function_name!();
    let codec = payload.codec.trim().to_ascii_lowercase();
    if !is_encode_codec(&codec) {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.video_encode_unsupported",
        )));
    }
    let video = resolve_video(
        &cfg,
        &token,
        &payload.path,
        payload.owner.as_deref(),
        locale.as_str(),
    )
    .await?;
    let output = encode_file(cfg.data_path.as_path(), &video.owner, &codec, &video.key)?;
    if tokio::fs::metadata(&output)
        .await
        .map(|meta| meta.is_file() && meta.len() > 0)
        .unwrap_or(false)
    {
        let message = tr(locale.as_str(), "response.ok");
        return Ok(create_response(
            true,
            &message,
            0,
            Some(json!({ "ready": true })),
        ));
    }
    let source = load_or_probe(&cfg, &video, locale.as_str()).await?;
    let file_name = task_file_name(video.source.as_path());
    let task = Arc::new(VideoEncodeTask {
        base: TaskBase::new(
            trf(
                locale.as_str(),
                "task.video_encode",
                &[("file", file_name.clone())],
            ),
            video.username.clone(),
        ),
        source: video.source.clone(),
        output,
        codec: codec.clone(),
        file_name,
        locale: locale.as_str().to_owned(),
        has_audio: !source.audio_codec.is_empty(),
        duration_ms: source.duration_ms,
        cancel: AtomicBool::new(false),
        child: Mutex::new(None),
    });
    let (task_id, created) = cfg.task_mgr.add_task_if_absent(task).await;
    let message = tr(locale.as_str(), "response.video_encode_started");
    Ok(create_response(
        true,
        &message,
        0,
        Some(json!({ "task_id": task_id, "created": created, "codec": codec })),
    ))
}

#[named]
pub async fn api_video_file(
    locale: RequestLocale,
    token: Token,
    Path((codec, path)): Path<(String, String)>,
    query: Query<OwnerQuery>,
    headers: HeaderMap,
    Extension(cfg): Extension<Arc<AppState>>,
) -> Result<Response<Body>, AppError> {
    let _ = function_name!();
    let codec = codec.to_ascii_lowercase();
    if !is_encode_codec(&codec) {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.video_encode_unsupported",
        )));
    }
    let video = resolve_video(&cfg, &token, &path, query.owner.as_deref(), locale.as_str()).await?;
    let file = encode_file(cfg.data_path.as_path(), &video.owner, &codec, &video.key)?;
    let root = encode_root(cfg.data_path.as_path(), &video.owner)?.join(&codec);
    if !file.starts_with(&root) {
        return Err(AppError::BadRequest(tr(
            locale.as_str(),
            "error.video_encode_unsupported",
        )));
    }
    send_media_range(file, &headers).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_bit_and_eight_bit_pixel_formats() {
        assert_eq!(bit_depth_from_pix_fmt("yuv420p10le"), 10);
        assert_eq!(bit_depth_from_pix_fmt("yuv422p"), 8);
        assert_eq!(bit_depth_from_pix_fmt("p010le"), 10);
    }

    #[test]
    fn probe_reads_video_audio_and_duration() {
        let value = serde_json::json!({
            "streams": [
                {"codec_type": "video", "codec_name": "h264", "profile": "High 10", "pix_fmt": "yuv420p10le", "bits_per_raw_sample": "10", "width": 1920, "height": 1080},
                {"codec_type": "audio", "codec_name": "pcm_s16le"}
            ],
            "format": {"duration": "2.5"}
        });
        let info = parse_probe_json(&value).unwrap();
        assert_eq!(info.video_codec, "h264");
        assert_eq!(info.bit_depth, 10);
        assert_eq!(info.audio_codec, "pcm_s16le");
        assert_eq!(info.duration_ms, 2500);
        assert_eq!(info.label, "H.264");
    }

    #[test]
    fn software_h264_keeps_resolution_and_uses_aac() {
        let args = encode_args(&EncodePlan {
            input: "in.mov".into(),
            output: "out.mp4".into(),
            encoder: EncoderKind::Software {
                name: "libx264".into(),
                extra: vec!["-crf".into(), "20".into()],
                tag: None,
            },
            has_audio: true,
        });
        let joined = args.join(" ");
        assert!(joined.contains("libx264"));
        assert!(joined.contains("yuv420p"));
        assert!(joined.contains("aac"));
        assert!(!joined.contains("scale="));
        assert!(joined.contains("+faststart"));
    }

    #[test]
    fn clock_progress_is_milliseconds() {
        assert_eq!(parse_ffmpeg_clock_ms("00:01:02.500"), Some(62_500));
    }

    #[test]
    fn partial_file_keeps_an_mp4_extension() {
        let path = partial_output(FsPath::new("/cache/video-encode/daniel/h264/abc.mp4"));
        assert_eq!(path.extension().and_then(|ext| ext.to_str()), Some("mp4"));
        assert_ne!(path, FsPath::new("/cache/video-encode/daniel/h264/abc.mp4"));
    }
}
