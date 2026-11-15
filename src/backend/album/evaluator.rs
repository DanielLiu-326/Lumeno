//! Explainable quality ranking for duplicate and burst groups.
//!
//! The evaluator only reads metadata, XMP sidecars, and cached previews. It
//! never mutates an original or an XMP sidecar. Ranking always uses automatic
//! quality; there is deliberately no "earliest/newest/manual" policy here.

use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;
use image::imageops::FilterType;
use image::{DynamicImage, GrayImage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::backend::ai::AlbumAiCache;
use crate::backend::ai::build_photo_content_signature;
use crate::backend::album::decoder::{
    DecodeOptions, DecoderCreateOptions, create_decoder, decode_bytes_with_path, find_xmp_sidecar,
    is_raw_path,
};
use crate::backend::album::helpers::{albums_root, ensure_album_cache_id};
use crate::backend::album::media::preview_thumbnail_cache_path;
use crate::backend::album::types::{AlbumPhotoItem, ColorLabel};
use crate::backend::common::{AppState, into_host_path_safely};

const QUALITY_CACHE_VERSION: u32 = 1;
const QUALITY_SAMPLE_EDGE: u32 = 512;
const QUALITY_PREVIEW_EDGE: u32 = 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct QualityWeights {
    pub(crate) sharpness: f32,
    pub(crate) resolution: f32,
    pub(crate) exposure: f32,
    pub(crate) compression: f32,
    pub(crate) format: f32,
}

impl Default for QualityWeights {
    fn default() -> Self {
        Self {
            sharpness: 0.35,
            resolution: 0.25,
            exposure: 0.15,
            compression: 0.15,
            format: 0.10,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct EvalParams {
    /// Prefer a version that already has an XMP sidecar.
    pub(crate) protect_edits: bool,
    /// Prefer a version with rating, tags, or color labels.
    pub(crate) protect_metadata: bool,
    pub(crate) weights: QualityWeights,
}

impl Default for EvalParams {
    fn default() -> Self {
        Self {
            protect_edits: true,
            protect_metadata: true,
            weights: QualityWeights::default(),
        }
    }
}

impl EvalParams {
    pub(crate) fn normalized(mut self) -> Result<Self, EvalError> {
        let weights = [
            self.weights.sharpness,
            self.weights.resolution,
            self.weights.exposure,
            self.weights.compression,
            self.weights.format,
        ];
        if weights
            .iter()
            .any(|weight| !weight.is_finite() || *weight < 0.0)
        {
            return Err(EvalError::InvalidWeights);
        }
        let sum: f32 = weights.iter().sum();
        if sum <= f32::EPSILON {
            return Err(EvalError::InvalidWeights);
        }
        self.weights.sharpness /= sum;
        self.weights.resolution /= sum;
        self.weights.exposure /= sum;
        self.weights.compression /= sum;
        self.weights.format /= sum;
        Ok(self)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct QualityMetrics {
    pub(crate) sharpness: f32,
    pub(crate) highlight_clip: f32,
    pub(crate) shadow_clip: f32,
    pub(crate) contrast: f32,
    pub(crate) bits_per_pixel: f32,
}

#[derive(Clone, Debug)]
pub(crate) struct PhotoSignals {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) size_bytes: u64,
    pub(crate) datetime: Option<NaiveDateTime>,
    pub(crate) has_xmp: bool,
    pub(crate) is_raw: bool,
    pub(crate) rating: u8,
    pub(crate) has_tags: bool,
    pub(crate) has_color_labels: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct EvaluatedPhoto {
    pub(crate) path: String,
    pub(crate) score: f32,
    pub(crate) rank: usize,
    pub(crate) recommended: bool,
    pub(crate) reasons: Vec<EvalReason>,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Evaluation {
    pub(crate) ranked: Vec<EvaluatedPhoto>,
    pub(crate) recommended_path: String,
    pub(crate) confidence: f32,
    pub(crate) exact_duplicates: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub(crate) enum EvalReasonCode {
    #[serde(rename = "has_edits")]
    HasEdits,
    #[serde(rename = "has_metadata")]
    HasMetadata,
    #[serde(rename = "higher_resolution")]
    HigherResolution,
    #[serde(rename = "sharper")]
    Sharper,
    #[serde(rename = "better_exposure")]
    BetterExposure,
    #[serde(rename = "less_compression")]
    LessCompression,
    #[serde(rename = "raw_original")]
    RawOriginal,
    #[serde(rename = "earlier_capture")]
    EarlierCapture,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct EvalReason {
    pub(crate) code: EvalReasonCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) value: Option<f32>,
}

#[derive(Clone, Debug)]
pub(crate) enum EvalError {
    InvalidWeights,
    NoCandidates,
    InvalidInput,
    Io(String),
    Decode(String),
}

impl std::fmt::Display for EvalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidWeights => write!(f, "invalid quality weights"),
            Self::NoCandidates => write!(f, "no candidates to evaluate"),
            Self::InvalidInput => write!(f, "candidate inputs do not match"),
            Self::Io(error) => write!(f, "io error: {error}"),
            Self::Decode(error) => write!(f, "decode error: {error}"),
        }
    }
}

impl std::error::Error for EvalError {}

/// Automatic quality evaluator. It is intentionally stateless between calls:
/// parameters live in this object, while per-photo metrics are cached on disk.
#[derive(Clone, Debug)]
pub(crate) struct Evaluator {
    params: EvalParams,
}

impl Evaluator {
    pub(crate) fn new(params: EvalParams) -> Result<Self, EvalError> {
        Ok(Self {
            params: params.normalized()?,
        })
    }

    pub(crate) fn params(&self) -> &EvalParams {
        &self.params
    }

    /// Rank photos from one album. `exact_duplicates` is supplied by the
    /// caller because only the duplicate index knows whether every byte is
    /// identical; exact groups skip quality scoring and use a stable tie-break.
    pub(crate) async fn rank(
        &self,
        cfg: &AppState,
        owner: &str,
        album: &str,
        photos: &[AlbumPhotoItem],
        exact_duplicates: bool,
    ) -> Result<Evaluation, EvalError> {
        if photos.is_empty() {
            return Err(EvalError::NoCandidates);
        }
        let album_dir = albums_root(&cfg.user_root_path(owner)).join(album);
        let cache_id = ensure_album_cache_id(&album_dir)
            .await
            .map_err(|error| EvalError::Io(error.to_string()))?;
        let cache_dir = AlbumAiCache::new(cfg.ai_service.store().root(), owner, &cache_id)
            .root
            .join("quality");

        let mut signals = Vec::with_capacity(photos.len());
        let mut metrics = Vec::with_capacity(photos.len());
        for photo in photos {
            let signal = load_signals(&album_dir, album, photo).await?;
            let metric = load_quality_metrics(cfg, owner, &cache_dir, album, photo, &signal).await;
            signals.push(signal);
            metrics.push(metric);
        }

        rank_candidates(photos, &signals, &metrics, &self.params, exact_duplicates)
    }
}

fn parse_photo_time(value: &str) -> Option<NaiveDateTime> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y:%m:%d %H:%M:%S"))
        .or_else(|_| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S"))
        .ok()
}

fn album_relative_path(album: &str, path: &str) -> Option<String> {
    path.strip_prefix(&format!("albums/{album}/"))
        .map(|rel| rel.replace('\\', "/"))
        .filter(|rel| !rel.is_empty())
}

async fn load_signals(
    album_dir: &Path,
    album: &str,
    photo: &AlbumPhotoItem,
) -> Result<PhotoSignals, EvalError> {
    let rel = album_relative_path(album, &photo.path).ok_or(EvalError::InvalidInput)?;
    let source =
        into_host_path_safely(album_dir, &rel).map_err(|error| EvalError::Io(error.to_string()))?;
    Ok(PhotoSignals {
        width: photo.width,
        height: photo.height,
        size_bytes: photo.size,
        datetime: parse_photo_time(&photo.datetime),
        has_xmp: find_xmp_sidecar(&source).is_some(),
        is_raw: is_raw_path(&source),
        rating: photo.rating,
        has_tags: !photo.tags.is_empty(),
        has_color_labels: photo
            .color_label
            .iter()
            .any(|label| *label != ColorLabel::None),
    })
}

async fn load_quality_metrics(
    cfg: &AppState,
    owner: &str,
    cache_dir: &Path,
    album: &str,
    photo: &AlbumPhotoItem,
    signal: &PhotoSignals,
) -> Option<QualityMetrics> {
    let signature = build_photo_content_signature(photo.size, photo.modified);
    let key = quality_cache_key(owner, album, &photo.path, &signature);
    let cache_path = cache_dir.join(format!("{key}.json"));
    if let Ok(raw) = tokio::fs::read_to_string(&cache_path).await
        && let Ok(cached) = serde_json::from_str::<QualityCacheFile>(&raw)
        && cached.schema_version == QUALITY_CACHE_VERSION
    {
        return Some(cached.metrics);
    }

    let rel = album_relative_path(album, &photo.path)?;
    let album_dir = albums_root(&cfg.user_root_path(owner)).join(album);
    let source = into_host_path_safely(&album_dir, &rel).ok()?;
    let metadata = tokio::fs::metadata(&source).await.ok()?;
    let preview = preview_thumbnail_cache_path(&cfg.data_path, owner, &source, &metadata);
    let image_path = match preview {
        Some(path) if path.is_file() => path,
        _ if !signal.is_raw => source,
        _ => return None,
    };
    let bytes = tokio::fs::read(&image_path).await.ok()?;
    let mut metrics = tokio::task::spawn_blocking(move || metrics_from_bytes(&image_path, &bytes))
        .await
        .ok()??;
    metrics.bits_per_pixel = (photo.size as f32 * 8.0)
        / (u64::from(signal.width.max(1)) * u64::from(signal.height.max(1))) as f32;
    if let Some(parent) = cache_path.parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    if let Ok(raw) = serde_json::to_vec_pretty(&QualityCacheFile {
        schema_version: QUALITY_CACHE_VERSION,
        metrics: metrics.clone(),
    }) {
        let _ = tokio::fs::write(&cache_path, raw).await;
    }
    Some(metrics)
}

fn quality_cache_key(owner: &str, album: &str, path: &str, signature: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"quality-v1|");
    hasher.update(owner.as_bytes());
    hasher.update(b"|");
    hasher.update(album.as_bytes());
    hasher.update(b"|");
    hasher.update(path.as_bytes());
    hasher.update(b"|");
    hasher.update(signature.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct QualityCacheFile {
    schema_version: u32,
    metrics: QualityMetrics,
}

fn metrics_from_bytes(path: &Path, bytes: &[u8]) -> Option<QualityMetrics> {
    let decoder = create_decoder(DecoderCreateOptions::cpu()).ok()?;
    let image = decode_bytes_with_path(
        decoder.as_ref(),
        path,
        bytes,
        DecodeOptions::new().with_max_edge(QUALITY_PREVIEW_EDGE),
    )
    .ok()?;
    Some(metrics_from_image(&image))
}

fn metrics_from_image(image: &DynamicImage) -> QualityMetrics {
    let width = image.width().max(1);
    let height = image.height().max(1);
    let luma = image.to_luma8();
    let total = (width as u64 * height as u64).max(1) as f32;
    let mut sum = 0.0f32;
    let mut sum_sq = 0.0f32;
    let mut highlight = 0u64;
    let mut shadow = 0u64;
    for pixel in luma.pixels() {
        let value = f32::from(pixel[0]);
        sum += value;
        sum_sq += value * value;
        if value >= 250.0 {
            highlight += 1;
        }
        if value <= 5.0 {
            shadow += 1;
        }
    }
    let mean = sum / total;
    let variance = (sum_sq / total - mean * mean).max(0.0);
    QualityMetrics {
        sharpness: laplacian_variance(&luma),
        highlight_clip: highlight as f32 / total,
        shadow_clip: shadow as f32 / total,
        contrast: variance.sqrt() / 255.0,
        bits_per_pixel: 8.0,
    }
}

fn laplacian_variance(source: &GrayImage) -> f32 {
    let width = source.width().max(1);
    let height = source.height().max(1);
    let sample = DynamicImage::ImageLuma8(source.clone())
        .resize_exact(
            QUALITY_SAMPLE_EDGE,
            QUALITY_SAMPLE_EDGE,
            FilterType::Triangle,
        )
        .to_luma8();
    let blurred = image::imageops::blur(&sample, 1.0);
    let mut response = Vec::with_capacity((sample.width() * sample.height()) as usize);
    for y in 1..blurred.height().saturating_sub(1) {
        for x in 1..blurred.width().saturating_sub(1) {
            let center = f32::from(blurred.get_pixel(x, y)[0]);
            let above = f32::from(blurred.get_pixel(x, y - 1)[0]);
            let below = f32::from(blurred.get_pixel(x, y + 1)[0]);
            let left = f32::from(blurred.get_pixel(x - 1, y)[0]);
            let right = f32::from(blurred.get_pixel(x + 1, y)[0]);
            response.push((above + below + left + right - 4.0 * center).abs());
        }
    }
    if response.is_empty() {
        return 0.0;
    }
    let mean = response.iter().sum::<f32>() / response.len() as f32;
    let variance = response
        .iter()
        .map(|value| {
            let delta = value - mean;
            delta * delta
        })
        .sum::<f32>()
        / response.len() as f32;
    variance
}

pub(crate) fn rank_candidates(
    photos: &[AlbumPhotoItem],
    signals: &[PhotoSignals],
    metrics: &[Option<QualityMetrics>],
    params: &EvalParams,
    exact_duplicates: bool,
) -> Result<Evaluation, EvalError> {
    if photos.is_empty() {
        return Err(EvalError::NoCandidates);
    }
    if photos.len() != signals.len() || photos.len() != metrics.len() {
        return Err(EvalError::InvalidInput);
    }
    let sharpness = normalize_metric(
        metrics
            .iter()
            .map(|metric| metric.as_ref().map(|m| m.sharpness)),
    );
    let resolution = normalize_metric(
        signals
            .iter()
            .map(|signal| Some((u64::from(signal.width) * u64::from(signal.height)) as f32)),
    );
    let compression = normalize_metric(
        metrics
            .iter()
            .map(|metric| metric.as_ref().map(|m| m.bits_per_pixel)),
    );

    let mut candidates: Vec<RankedCandidate> = photos
        .iter()
        .enumerate()
        .map(|(index, photo)| {
            let signal = &signals[index];
            let metric = metrics[index].as_ref();
            let protection = protection_rank(signal, params);
            let exposure = metric
                .map(|m| {
                    (1.0 - (m.highlight_clip * 3.0 + m.shadow_clip * 3.0).clamp(0.0, 1.0))
                        .clamp(0.0, 1.0)
                })
                .unwrap_or(0.5);
            let format = format_score(&photo.name);
            let quality = if exact_duplicates {
                0.0
            } else {
                100.0
                    * (params.weights.sharpness * sharpness[index]
                        + params.weights.resolution * resolution[index]
                        + params.weights.exposure * exposure
                        + params.weights.compression * compression[index]
                        + params.weights.format * format)
            };
            let reasons = reasons_for(
                signal,
                metric,
                params,
                resolution[index],
                sharpness[index],
                exposure,
            );
            RankedCandidate {
                path: photo.path.clone(),
                name: photo.name.clone(),
                datetime: signal.datetime,
                protection,
                score: quality,
                reasons,
            }
        })
        .collect();

    candidates.sort_by(|left, right| {
        right
            .protection
            .cmp(&left.protection)
            .then_with(|| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| match (left.datetime, right.datetime) {
                (Some(left), Some(right)) => left.cmp(&right),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
            .then_with(|| left.path.cmp(&right.path))
    });

    let confidence = if candidates.len() < 2 {
        1.0
    } else {
        let gap = (candidates[0].score - candidates[1].score).abs();
        (gap / 10.0).clamp(0.0, 1.0)
    };
    let recommended_path = candidates[0].path.clone();
    let ranked = candidates
        .into_iter()
        .enumerate()
        .map(|(index, candidate)| EvaluatedPhoto {
            path: candidate.path,
            score: candidate.score,
            rank: index + 1,
            recommended: index == 0,
            reasons: candidate.reasons,
        })
        .collect();

    Ok(Evaluation {
        ranked,
        recommended_path,
        confidence,
        exact_duplicates,
    })
}

struct RankedCandidate {
    path: String,
    name: String,
    datetime: Option<NaiveDateTime>,
    protection: u8,
    score: f32,
    reasons: Vec<EvalReason>,
}

fn protection_rank(signal: &PhotoSignals, params: &EvalParams) -> u8 {
    let edits = if params.protect_edits && signal.has_xmp {
        2
    } else {
        0
    };
    let metadata = if params.protect_metadata
        && (signal.rating > 0 || signal.has_tags || signal.has_color_labels)
    {
        1
    } else {
        0
    };
    edits + metadata
}

fn format_score(name: &str) -> f32 {
    let ext = Path::new(name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "dng" | "nef" | "nrw" | "cr2" | "cr3" | "arw" | "raf" | "rw2" | "orf" | "pef" | "srw"
        | "x3f" => 1.0,
        "tif" | "tiff" | "png" => 0.95,
        "jpg" | "jpeg" => 0.75,
        "webp" => 0.70,
        _ => 0.60,
    }
}

fn normalize_metric<I>(values: I) -> Vec<f32>
where
    I: Iterator<Item = Option<f32>>,
{
    let values: Vec<Option<f32>> = values.collect();
    let present: Vec<f32> = values.iter().filter_map(|value| *value).collect();
    if present.is_empty() {
        return vec![0.5; values.len()];
    }
    let min = present.iter().copied().fold(f32::INFINITY, f32::min);
    let max = present.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if (max - min).abs() <= f32::EPSILON {
        return values
            .iter()
            .map(|value| if value.is_some() { 0.5 } else { 0.0 })
            .collect();
    }
    values
        .iter()
        .map(|value| {
            value
                .map(|value| ((value - min) / (max - min)).clamp(0.0, 1.0))
                .unwrap_or(0.0)
        })
        .collect()
}

fn reasons_for(
    signal: &PhotoSignals,
    metric: Option<&QualityMetrics>,
    params: &EvalParams,
    resolution: f32,
    sharpness: f32,
    exposure: f32,
) -> Vec<EvalReason> {
    let mut reasons = Vec::new();
    if params.protect_edits && signal.has_xmp {
        reasons.push(EvalReason {
            code: EvalReasonCode::HasEdits,
            value: None,
        });
    }
    if params.protect_metadata && (signal.rating > 0 || signal.has_tags || signal.has_color_labels)
    {
        reasons.push(EvalReason {
            code: EvalReasonCode::HasMetadata,
            value: None,
        });
    }
    if signal.is_raw {
        reasons.push(EvalReason {
            code: EvalReasonCode::RawOriginal,
            value: None,
        });
    }
    if resolution >= 0.999 {
        reasons.push(EvalReason {
            code: EvalReasonCode::HigherResolution,
            value: None,
        });
    }
    if sharpness > 0.5 {
        reasons.push(EvalReason {
            code: EvalReasonCode::Sharper,
            value: Some(sharpness),
        });
    }
    if exposure > 0.5 {
        reasons.push(EvalReason {
            code: EvalReasonCode::BetterExposure,
            value: Some(exposure),
        });
    }
    if metric.is_some_and(|metric| metric.bits_per_pixel >= 1.0) {
        reasons.push(EvalReason {
            code: EvalReasonCode::LessCompression,
            value: None,
        });
    }
    reasons
}

#[cfg(test)]
mod tests {
    use super::{
        EvalParams, Evaluator, PhotoSignals, QualityMetrics, QualityWeights, rank_candidates,
    };
    use crate::backend::album::types::{AlbumPhotoItem, ColorLabel};
    use chrono::NaiveDate;

    fn photo(name: &str, width: u32, height: u32, size: u64) -> AlbumPhotoItem {
        AlbumPhotoItem {
            path: format!("albums/test/{name}"),
            name: name.to_string(),
            date: "2026-01-01".to_string(),
            datetime: "2026-01-01 10:00:00".to_string(),
            size,
            modified: None,
            camera_make: String::new(),
            camera_model: String::new(),
            lens: String::new(),
            focal_length: String::new(),
            aperture: String::new(),
            shutter_speed: String::new(),
            iso: None,
            width,
            height,
            duration_ms: 0,
            rating: 0,
            color_label: Vec::<ColorLabel>::new(),
            tags: Vec::new(),
        }
    }

    fn signal(has_xmp: bool, rating: u8) -> PhotoSignals {
        PhotoSignals {
            width: 4000,
            height: 3000,
            size_bytes: 1_000_000,
            datetime: Some(
                NaiveDate::from_ymd_opt(2026, 1, 1)
                    .unwrap()
                    .and_hms_opt(10, 0, 0)
                    .unwrap(),
            ),
            has_xmp,
            is_raw: false,
            rating,
            has_tags: false,
            has_color_labels: false,
        }
    }

    fn metrics(sharpness: f32) -> QualityMetrics {
        QualityMetrics {
            sharpness,
            highlight_clip: 0.0,
            shadow_clip: 0.0,
            contrast: 0.3,
            bits_per_pixel: 2.0,
        }
    }

    #[test]
    fn invalid_weights_are_rejected() {
        let params = EvalParams {
            weights: QualityWeights {
                sharpness: 0.0,
                resolution: 0.0,
                exposure: 0.0,
                compression: 0.0,
                format: 0.0,
            },
            ..Default::default()
        };
        assert!(Evaluator::new(params).is_err());
    }

    #[test]
    fn edited_version_wins_before_sharpness() {
        let photos = vec![
            photo("sharp.jpg", 4000, 3000, 1),
            photo("edited.jpg", 4000, 3000, 1),
        ];
        let signals = vec![signal(false, 0), signal(true, 0)];
        let metrics = vec![Some(metrics(100.0)), Some(metrics(1.0))];
        let result =
            rank_candidates(&photos, &signals, &metrics, &EvalParams::default(), false).unwrap();
        assert_eq!(result.recommended_path, "albums/test/edited.jpg");
    }

    #[test]
    fn sharper_photo_wins_without_protection() {
        let photos = vec![photo("a.jpg", 4000, 3000, 1), photo("b.jpg", 4000, 3000, 1)];
        let signals = vec![signal(false, 0), signal(false, 0)];
        let metrics = vec![Some(metrics(10.0)), Some(metrics(100.0))];
        let result =
            rank_candidates(&photos, &signals, &metrics, &EvalParams::default(), false).unwrap();
        assert_eq!(result.recommended_path, "albums/test/b.jpg");
    }

    #[test]
    fn exact_duplicates_are_stable_and_path_ties_break() {
        let photos = vec![photo("b.jpg", 4000, 3000, 1), photo("a.jpg", 4000, 3000, 1)];
        let signals = vec![signal(false, 0), signal(false, 0)];
        let metrics = vec![None, None];
        let result =
            rank_candidates(&photos, &signals, &metrics, &EvalParams::default(), true).unwrap();
        assert_eq!(result.recommended_path, "albums/test/a.jpg");
        assert_eq!(result.ranked[0].rank, 1);
        assert!(result.exact_duplicates);
    }
}
