use async_recursion::async_recursion;
use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{Path, Query};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::{Extension, Json};
use base64::Engine;
use chrono::{DateTime, Datelike, NaiveDate, NaiveDateTime, Utc};
use function_name::named;
use futures::future::BoxFuture;
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256, Sha512};
use std::collections::{HashMap, HashSet};
use std::io::BufReader;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::u8;
use tokio::sync::Notify;
use tokio_util::io::ReaderStream;
use uuid::Uuid;

use crate::backend::common::{
    AppError, AppState, ConcurrencyLimiter, ConcurrencyPermit, Token, create_response,
    into_host_path_safely, move_file_or_dir,
};
use crate::backend::i18n::{RequestLocale, tr, trf};
use crate::backend::tasks::{Task, TaskBase, TaskStatus};

use super::media::{
    collect_album_images, ensure_thumbnail_cached, exif_cache_key, find_xmp_sidecar, is_image_path,
    is_raw_path, DarktableClient,
};
use super::types::*;

// ── Album helpers ────────────────────────────────────────────────────────────

pub(crate) fn album_meta_path(album_dir: &FsPath) -> PathBuf {
    album_dir.join("album_meta.json")
}

pub(crate) async fn read_album_meta(album_dir: &FsPath) -> AlbumMeta {
    let path = album_meta_path(album_dir);
    if let Ok(raw) = tokio::fs::read_to_string(&path).await {
        if let Ok(meta) = serde_json::from_str::<AlbumMeta>(&raw) {
            return meta;
        }
    }
    AlbumMeta {
        name: album_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
        ..Default::default()
    }
}

pub(crate) async fn write_album_meta(album_dir: &FsPath, meta: &AlbumMeta) -> Result<(), AppError> {
    let path = album_meta_path(album_dir);
    let raw = serde_json::to_string_pretty(meta)
        .map_err(|e| AppError::Unknown(format!("serialize album meta failed: {e}")))?;
    tokio::fs::write(&path, raw)
        .await
        .map_err(|e| AppError::Unknown(format!("write album meta failed: {e}")))?;
    Ok(())
}

pub(crate) fn albums_root(root_path: &FsPath) -> PathBuf {
    root_path.join("albums")
}

pub(crate) fn cache_exif_root(data_path: &FsPath) -> PathBuf {
    data_path.join("cache").join("exif")
}

pub(crate) fn album_index_path(data_path: &FsPath, username: &str, album_name: &str) -> PathBuf {
    data_path
        .join("cache")
        .join("index")
        .join(username)
        .join(format!("{album_name}.json"))
}

pub(crate) fn xmp_sidecar_path(photo: &FsPath) -> PathBuf {
    PathBuf::from(format!("{}.xmp", photo.to_string_lossy()))
}

/// Parse `xmp:Rating`, `dc:subject` tags, and `darktable:colorlabels` from XMP content.
pub(crate) fn parse_xmp_meta(content: &str) -> ImageSoftwareMeta {
    let mut meta = ImageSoftwareMeta::default();
    let mut reader = quick_xml::Reader::from_str(content);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::new();
    let mut depth_dc_subject: u32 = 0;
    let mut in_dc_subject_bag = false;
    let mut depth_colorlabels: u32 = 0;
    let mut in_colorlabels_seq = false;
    let mut collect_tag = false;
    let mut collect_colorlabel = false;

    loop {
        buf.clear();
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => match e.name().as_ref() {
                b"rdf:Description" => {
                    for attr in e.attributes().flatten() {
                        if attr.key.as_ref() == b"xmp:Rating" {
                            if let Ok(v) = attr.unescape_value() {
                                meta.rating = v.trim().parse::<u8>().unwrap_or(0).min(5);
                            }
                        }
                    }
                }
                b"dc:subject" => {
                    depth_dc_subject += 1;
                }
                b"rdf:Bag" => {
                    if depth_dc_subject > 0 {
                        in_dc_subject_bag = true;
                    }
                }
                b"darktable:colorlabels" => {
                    depth_colorlabels += 1;
                }
                b"rdf:Seq" => {
                    if depth_colorlabels > 0 {
                        in_colorlabels_seq = true;
                    }
                }
                b"rdf:li" => {
                    if in_dc_subject_bag {
                        collect_tag = true;
                    } else if in_colorlabels_seq {
                        collect_colorlabel = true;
                    }
                }
                _ => {}
            },
            Ok(Event::Text(ref e)) => {
                if collect_tag || collect_colorlabel {
                    if let Ok(text) = e.unescape() {
                        let text = text.trim();
                        if collect_tag && !text.is_empty() {
                            meta.tags.push(text.to_string());
                        } else if collect_colorlabel {
                            if let Ok(n) = text.parse::<u8>() {
                                let cl = ColorLabel::from_darktable(n);
                                if cl != ColorLabel::None {
                                    meta.color_label.push(cl);
                                }
                            }
                        }
                    }
                    collect_tag = false;
                    collect_colorlabel = false;
                }
            }
            Ok(Event::End(ref e)) => match e.name().as_ref() {
                b"dc:subject" => {
                    depth_dc_subject = depth_dc_subject.saturating_sub(1);
                    if depth_dc_subject == 0 {
                        in_dc_subject_bag = false;
                    }
                }
                b"rdf:Bag" => {
                    if depth_dc_subject == 0 {
                        in_dc_subject_bag = false;
                    }
                }
                b"darktable:colorlabels" => {
                    depth_colorlabels = depth_colorlabels.saturating_sub(1);
                    if depth_colorlabels == 0 {
                        in_colorlabels_seq = false;
                    }
                }
                b"rdf:Seq" => {
                    if depth_colorlabels == 0 {
                        in_colorlabels_seq = false;
                    }
                }
                b"rdf:li" => {
                    collect_tag = false;
                    collect_colorlabel = false;
                }
                _ => {}
            },
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    meta
}

/// Write a `<darktable:colorlabels>` block using a quick-xml Writer.
pub(crate) fn write_colorlabels_block<W: std::io::Write>(writer: &mut Writer<W>, labels: &[ColorLabel]) {
    if labels.is_empty() {
        return;
    }
    writer
        .write_event(Event::Start(BytesStart::new("darktable:colorlabels")))
        .unwrap();
    writer
        .write_event(Event::Start(BytesStart::new("rdf:Seq")))
        .unwrap();
    for cl in labels {
        if let Some(n) = cl.to_darktable() {
            writer
                .write_event(Event::Start(BytesStart::new("rdf:li")))
                .unwrap();
            writer
                .write_event(Event::Text(BytesText::new(&n.to_string())))
                .unwrap();
            writer
                .write_event(Event::End(BytesEnd::new("rdf:li")))
                .unwrap();
        }
    }
    writer
        .write_event(Event::End(BytesEnd::new("rdf:Seq")))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new("darktable:colorlabels")))
        .unwrap();
}

/// Write a `<dc:subject>` block using a quick-xml Writer.
pub(crate) fn write_dc_subject_block<W: std::io::Write>(writer: &mut Writer<W>, tags: &[String]) {
    if tags.is_empty() {
        return;
    }
    writer
        .write_event(Event::Start(BytesStart::new("dc:subject")))
        .unwrap();
    writer
        .write_event(Event::Start(BytesStart::new("rdf:Bag")))
        .unwrap();
    for tag in tags {
        writer
            .write_event(Event::Start(BytesStart::new("rdf:li")))
            .unwrap();
        writer
            .write_event(Event::Text(BytesText::new(tag)))
            .unwrap();
        writer
            .write_event(Event::End(BytesEnd::new("rdf:li")))
            .unwrap();
    }
    writer
        .write_event(Event::End(BytesEnd::new("rdf:Bag")))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new("dc:subject")))
        .unwrap();
}

/// Generate a minimal valid XMP sidecar containing only lumeno-managed fields.
pub(crate) fn generate_minimal_xmp(meta: &ImageSoftwareMeta) -> String {
    let mut out = Vec::new();
    let mut writer = Writer::new_with_indent(&mut out, b' ', 1);

    writer
        .write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .unwrap();

    let mut xmpmeta = BytesStart::new("x:xmpmeta");
    xmpmeta.push_attribute(("xmlns:x", "adobe:ns:meta/"));
    xmpmeta.push_attribute(("x:xmptk", "XMP Core 4.4.0-Exiv2"));
    writer.write_event(Event::Start(xmpmeta)).unwrap();

    let mut rdf = BytesStart::new("rdf:RDF");
    rdf.push_attribute(("xmlns:rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"));
    writer.write_event(Event::Start(rdf)).unwrap();

    let mut desc = BytesStart::new("rdf:Description");
    desc.push_attribute(("rdf:about", ""));
    desc.push_attribute(("xmlns:xmp", "http://ns.adobe.com/xap/1.0/"));
    desc.push_attribute(("xmlns:dc", "http://purl.org/dc/elements/1.1/"));
    if !meta.color_label.is_empty() {
        desc.push_attribute(("xmlns:darktable", "http://darktable.sf.net/"));
    }
    let rating_str = meta.rating.to_string();
    desc.push_attribute(("xmp:Rating", rating_str.as_str()));
    writer.write_event(Event::Start(desc)).unwrap();

    write_colorlabels_block(&mut writer, &meta.color_label);
    write_dc_subject_block(&mut writer, &meta.tags);

    writer
        .write_event(Event::End(BytesEnd::new("rdf:Description")))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new("rdf:RDF")))
        .unwrap();
    writer
        .write_event(Event::End(BytesEnd::new("x:xmpmeta")))
        .unwrap();

    String::from_utf8(out).expect("XMP output is valid UTF-8")
}

/// Merge lumeno metadata (rating, tags, color labels) into existing XMP by streaming
/// with quick-xml Reader/Writer. Only our managed fields are replaced; all other
/// content (darktable history, dc:creator, etc.) is passed through unchanged.
pub fn merge_xmp_meta(existing: Option<&str>, meta: &ImageSoftwareMeta) -> String {
    let source = match existing {
        Some(s) if !s.trim().is_empty() => s,
        _ => return generate_minimal_xmp(meta),
    };

    let mut out: Vec<u8> = Vec::with_capacity(source.len() + 256);
    let mut writer = Writer::new(&mut out);
    let mut reader = quick_xml::Reader::from_str(source);
    // Keep existing whitespace so pass-through content is preserved verbatim.
    reader.config_mut().trim_text(false);

    let mut buf = Vec::new();
    // When > 0 we are inside a block being replaced; track nesting depth.
    let mut skip_depth: u32 = 0;
    let mut skip_is_dc_subject = false;
    let mut found_dc_subject = false;
    let mut found_colorlabels = false;
    let rating_str = meta.rating.to_string();

    loop {
        buf.clear();
        let event = match reader.read_event_into(&mut buf) {
            Ok(e) => e,
            Err(_) => break,
        };

        // ── Inside a skipped block: track nesting, write replacement on close ──
        if skip_depth > 0 {
            match &event {
                Event::Start(_) => skip_depth += 1,
                Event::End(_) => {
                    skip_depth -= 1;
                    if skip_depth == 0 {
                        if skip_is_dc_subject {
                            write_dc_subject_block(&mut writer, &meta.tags);
                        } else {
                            write_colorlabels_block(&mut writer, &meta.color_label);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }

        // ── Normal pass-through with targeted modifications ──
        match event {
            // Rebuild rdf:Description start tag with updated xmp:Rating (and
            // xmlns:darktable if now needed).
            Event::Start(ref e) if e.name().as_ref() == b"rdf:Description" => {
                let mut new_elem = BytesStart::new("rdf:Description");
                let mut found_rating = false;
                let mut found_darktable_ns = false;
                for attr in e.attributes().flatten() {
                    let key = attr.key.as_ref();
                    if key == b"xmp:Rating" {
                        new_elem.push_attribute(("xmp:Rating", rating_str.as_str()));
                        found_rating = true;
                    } else {
                        if key == b"xmlns:darktable" {
                            found_darktable_ns = true;
                        }
                        let k = std::str::from_utf8(attr.key.as_ref()).unwrap_or_default();
                        let v = attr.unescape_value().unwrap_or_default();
                        new_elem.push_attribute((k, v.as_ref()));
                    }
                }
                if !found_rating {
                    new_elem.push_attribute(("xmp:Rating", rating_str.as_str()));
                }
                if !found_darktable_ns && !meta.color_label.is_empty() {
                    new_elem.push_attribute(("xmlns:darktable", "http://darktable.sf.net/"));
                }
                writer.write_event(Event::Start(new_elem)).unwrap();
            }

            // Skip existing dc:subject; write our replacement when block closes.
            Event::Start(ref e) if e.name().as_ref() == b"dc:subject" => {
                skip_depth = 1;
                skip_is_dc_subject = true;
                found_dc_subject = true;
            }

            // Skip existing darktable:colorlabels; write our replacement when block closes.
            Event::Start(ref e) if e.name().as_ref() == b"darktable:colorlabels" => {
                skip_depth = 1;
                skip_is_dc_subject = false;
                found_colorlabels = true;
            }

            // Before closing rdf:Description, insert any blocks that were absent.
            Event::End(ref e) if e.name().as_ref() == b"rdf:Description" => {
                if !found_dc_subject {
                    write_dc_subject_block(&mut writer, &meta.tags);
                }
                if !found_colorlabels {
                    write_colorlabels_block(&mut writer, &meta.color_label);
                }
                writer
                    .write_event(Event::End(BytesEnd::new("rdf:Description")))
                    .unwrap();
            }

            Event::Eof => break,

            // Everything else (darktable:history, dc:creator, whitespace, etc.).
            other => {
                writer.write_event(other).unwrap();
            }
        }
    }

    String::from_utf8(out).unwrap_or_else(|_| generate_minimal_xmp(meta))
}

pub(crate) fn sanitize_album_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.contains('\0')
        || trimmed.starts_with('.')
    {
        return None;
    }
    Some(trimmed.to_string())
}

pub(crate) fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut set = HashSet::new();
    let mut out = Vec::new();
    for tag in tags {
        let t = tag.trim().to_lowercase();
        if t.is_empty() {
            continue;
        }
        if set.insert(t.clone()) {
            out.push(t);
        }
    }
    out
}

pub(crate) fn relative_display_path(root_path: &FsPath, full_path: &FsPath) -> String {
    full_path
        .strip_prefix(root_path)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| full_path.to_string_lossy().replace('\\', "/"))
}

pub(crate) fn parse_exif_datetime_to_date(input: &str) -> Option<String> {
    let trimmed = input.trim_matches('"').trim();
    let base = trimmed.get(0..19).unwrap_or(trimmed);
    // EXIF raw uses colons "2024:01:15 10:30:00", but display_value() may
    // format with dashes "2024-01-15 10:30:00". Try both.
    NaiveDateTime::parse_from_str(base, "%Y:%m:%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(base, "%Y-%m-%d %H:%M:%S"))
        .ok()
        .map(|d| d.date().format("%Y-%m-%d").to_string())
}

pub(crate) fn parse_date_from_parent_dirs(path: &FsPath) -> Option<String> {
    for component in path.components() {
        let name = component.as_os_str().to_string_lossy();
        if NaiveDate::parse_from_str(&name, "%Y-%m-%d").is_ok() {
            return Some(name.to_string());
        }
    }
    None
}

pub(crate) fn fallback_date_from_metadata(metadata: &std::fs::Metadata) -> String {
    metadata
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "1970-01-01".to_string())
}

/// Extract EXIF from RAW files via exiftool CLI (the `exif` crate doesn't support most RAW formats).
pub(crate) async fn ensure_exif_via_exiftool(source: &FsPath) -> ExifCacheEntry {
    let output = tokio::process::Command::new("exiftool")
        .arg("-json")
        .arg("-DateTimeOriginal")
        .arg("-Make")
        .arg("-Model")
        .arg("-LensModel")
        .arg("-FocalLength")
        .arg("-FNumber")
        .arg("-ExposureTime")
        .arg("-ISO")
        .arg("-ImageWidth")
        .arg("-ImageHeight")
        .arg(source)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .await;
    let Ok(output) = output else {
        return ExifCacheEntry::default();
    };
    if !output.status.success() {
        return ExifCacheEntry::default();
    }
    let Ok(text) = String::from_utf8(output.stdout) else {
        return ExifCacheEntry::default();
    };
    let Ok(parsed) = serde_json::from_str::<Vec<Value>>(&text) else {
        return ExifCacheEntry::default();
    };
    let Some(obj) = parsed.first() else {
        return ExifCacheEntry::default();
    };

    let get = |key: &str| -> String {
        obj.get(key)
            .and_then(|v| match v {
                Value::String(s) => Some(s.trim().to_string()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .unwrap_or_default()
    };

    let datetime_raw = get("DateTimeOriginal");
    let date = parse_exif_datetime_to_date(&datetime_raw).unwrap_or_default();
    // Normalize "2026:04:04 17:27:57" to "2026-04-04 17:27:57" for display consistency
    let datetime_display = if datetime_raw.len() >= 10 && &datetime_raw[4..5] == ":" {
        format!(
            "{}{}",
            datetime_raw[..10].replace(':', "-"),
            &datetime_raw[10..]
        )
    } else {
        datetime_raw
    };

    let aperture_raw = get("FNumber");
    let aperture = if aperture_raw.is_empty() {
        String::new()
    } else {
        format!("f/{aperture_raw}")
    };

    let shutter = get("ExposureTime");
    let shutter_speed = if shutter.is_empty() {
        String::new()
    } else if shutter.contains('/') {
        format!("{shutter} s")
    } else if let Ok(val) = shutter.parse::<f64>() {
        if val < 1.0 && val > 0.0 {
            format!("1/{} s", (1.0 / val).round() as u64)
        } else {
            format!("{shutter} s")
        }
    } else {
        format!("{shutter} s")
    };

    let focal = get("FocalLength");
    let focal_length = if focal.is_empty() {
        String::new()
    } else if focal.contains("mm") {
        focal
    } else {
        format!("{focal} mm")
    };

    let iso = get("ISO").parse::<u32>().ok();

    ExifCacheEntry {
        date,
        datetime: datetime_display,
        camera_make: get("Make"),
        camera_model: get("Model"),
        lens: get("LensModel").trim().to_string(),
        focal_length,
        aperture,
        shutter_speed,
        iso,
        width: get("ImageWidth").parse().unwrap_or(0),
        height: get("ImageHeight").parse().unwrap_or(0),
    }
}

pub(crate) async fn ensure_exif_cached(
    data_path: &FsPath,
    source: &FsPath,
) -> Result<ExifCacheEntry, AppError> {
    let metadata = tokio::fs::metadata(source)
        .await
        .map_err(|_| AppError::NotFound(source.display().to_string()))?;

    let cache_key = exif_cache_key(source, &metadata);
    let cache_path = cache_exif_root(data_path).join(format!("{cache_key}.json"));

    if cache_path.exists() {
        let cached_raw = tokio::fs::read_to_string(&cache_path)
            .await
            .map_err(|e| AppError::Unknown(format!("read exif cache failed: {e}")))?;
        if let Ok(cached) = serde_json::from_str::<ExifCacheEntry>(&cached_raw)
            && !cached.date.is_empty()
        {
            return Ok(cached);
        }
    }

    let source_for_block = source.to_path_buf();
    let is_raw = is_raw_path(source);
    let exif_info = if is_raw {
        // Use exiftool CLI for RAW files (the exif crate doesn't support most RAW formats)
        ensure_exif_via_exiftool(source).await
    } else {
        tokio::task::spawn_blocking(move || -> ExifCacheEntry {
            let (width, height) = image::image_dimensions(&source_for_block).unwrap_or((0, 0));
            let file = match std::fs::File::open(&source_for_block) {
                Ok(f) => f,
                Err(_) => {
                    return ExifCacheEntry {
                        width,
                        height,
                        ..Default::default()
                    };
                }
            };
            let mut reader = BufReader::new(file);
            let exif = match exif::Reader::new().read_from_container(&mut reader) {
                Ok(e) => e,
                Err(_) => {
                    return ExifCacheEntry {
                        width,
                        height,
                        ..Default::default()
                    };
                }
            };

            let get_str = |tag: exif::Tag| -> String {
                exif.get_field(tag, exif::In::PRIMARY)
                    .map(|f| f.display_value().with_unit(&exif).to_string())
                    .unwrap_or_default()
            };

            let datetime_raw = get_str(exif::Tag::DateTimeOriginal);
            let datetime_raw = if datetime_raw.is_empty() {
                get_str(exif::Tag::DateTime)
            } else {
                datetime_raw
            };
            let datetime_raw = datetime_raw.trim_matches('"').trim().to_string();
            let date = parse_exif_datetime_to_date(&datetime_raw).unwrap_or_default();

            let iso = exif
                .get_field(exif::Tag::PhotographicSensitivity, exif::In::PRIMARY)
                .and_then(|f| match f.value {
                    exif::Value::Short(ref v) => v.first().map(|&x| x as u32),
                    exif::Value::Long(ref v) => v.first().copied(),
                    _ => f.display_value().to_string().parse::<u32>().ok(),
                });

            ExifCacheEntry {
                date,
                datetime: datetime_raw.clone(),
                camera_make: get_str(exif::Tag::Make)
                    .trim_matches('"')
                    .trim()
                    .to_string(),
                camera_model: get_str(exif::Tag::Model)
                    .trim_matches('"')
                    .trim()
                    .to_string(),
                lens: get_str(exif::Tag::LensModel)
                    .trim_matches('"')
                    .trim()
                    .to_string(),
                focal_length: get_str(exif::Tag::FocalLength),
                aperture: get_str(exif::Tag::FNumber),
                shutter_speed: get_str(exif::Tag::ExposureTime),
                iso,
                width,
                height,
            }
        })
        .await
        .map_err(|e| AppError::Unknown(format!("read exif join failed: {e}")))?
    };

    let mut entry = exif_info;
    if entry.date.is_empty() {
        entry.date = fallback_date_from_metadata(&metadata);
    }

    if let Some(parent) = cache_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Unknown(format!("create exif cache dir failed: {e}")))?;
    }
    let cache_data = serde_json::to_string(&entry)
        .map_err(|e| AppError::Unknown(format!("serialize exif cache failed: {e}")))?;
    tokio::fs::write(&cache_path, cache_data)
        .await
        .map_err(|e| AppError::Unknown(format!("write exif cache failed: {e}")))?;

    Ok(entry)
}

pub(crate) async fn read_image_software_meta(image_path: &FsPath) -> ImageSoftwareMeta {
    let xmp_path = find_xmp_sidecar(image_path);
    match xmp_path {
        Some(ref p) => match tokio::fs::read_to_string(p).await {
            Ok(raw) => parse_xmp_meta(&raw),
            Err(_) => ImageSoftwareMeta::default(),
        },
        None => ImageSoftwareMeta::default(),
    }
}

pub(crate) async fn write_image_software_meta_if_needed(
    image_path: &FsPath,
    meta: &ImageSoftwareMeta,
) -> Result<(), AppError> {
    let xmp_path = xmp_sidecar_path(image_path);
    if meta.tags.is_empty() && meta.rating == 0 && meta.color_label.is_empty() {
        if xmp_path.exists() {
            let _ = tokio::fs::remove_file(&xmp_path).await;
        }
        return Ok(());
    }

    let existing = tokio::fs::read_to_string(&xmp_path).await.ok();
    let new_xmp = merge_xmp_meta(existing.as_deref(), meta);
    tokio::fs::write(&xmp_path, new_xmp)
        .await
        .map_err(|e| AppError::Unknown(format!("write XMP sidecar failed: {e}")))?;
    Ok(())
}


// ── Migration: JSON sidecars → XMP ───────────────────────────────────────────

#[async_recursion]
pub(crate) async fn scan_json_sidecars_recursive(dir: &FsPath, excluded_dir: &FsPath, out: &mut Vec<PathBuf>) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        if path == excluded_dir {
            continue;
        }
        if path.is_dir() {
            scan_json_sidecars_recursive(&path, excluded_dir, out).await;
        } else if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                let image_path = path.parent().unwrap_or_else(|| FsPath::new("")).join(stem);
                if image_path.exists() && is_image_path(&image_path) {
                    out.push(path);
                }
            }
        }
    }
}

/// Scan `data_path` recursively for legacy `.json` image sidecars.
pub async fn scan_json_sidecars(data_path: &FsPath) -> Vec<PathBuf> {
    let mut result = Vec::new();
    scan_json_sidecars_recursive(data_path, &data_path.join("cache"), &mut result).await;
    result
}

/// Migrate a single `.json` sidecar to XMP, then delete the JSON file.
pub async fn migrate_json_sidecar(json_path: &FsPath) {
    let Ok(raw) = tokio::fs::read_to_string(json_path).await else {
        return;
    };
    let Ok(meta) = serde_json::from_str::<ImageSoftwareMeta>(&raw) else {
        return;
    };
    let path_str = json_path.to_string_lossy();
    let image_str = path_str.strip_suffix(".json").unwrap_or(&path_str);
    let xmp_path = xmp_sidecar_path(FsPath::new(image_str));
    let existing_xmp = tokio::fs::read_to_string(&xmp_path).await.ok();
    let new_xmp = merge_xmp_meta(existing_xmp.as_deref(), &meta);
    if tokio::fs::write(&xmp_path, new_xmp).await.is_ok() {
        let _ = tokio::fs::remove_file(json_path).await;
    }
}

pub(crate) async fn unique_destination_path(dir: &FsPath, file_name: &str) -> PathBuf {
    let mut candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let stem = FsPath::new(file_name)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    let ext = FsPath::new(file_name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let mut idx = 1usize;
    loop {
        let name = if ext.is_empty() {
            format!("{stem}_{idx}")
        } else {
            format!("{stem}_{idx}.{ext}")
        };
        candidate = dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
        idx += 1;
    }
}

pub(crate) async fn rebuild_album_index(
    data_path: &FsPath,
    root_path: &FsPath,
    username: &str,
    album_name: &str,
    task_base: Option<&TaskBase>,
) -> Result<AlbumIndex, AppError> {
    use futures::StreamExt;

    let album_dir = albums_root(root_path).join(album_name);
    if !album_dir.exists() {
        return Ok(AlbumIndex {
            version: 2,
            entries: Vec::new(),
        });
    }

    let mut photos_paths = Vec::new();
    collect_album_images(album_dir.as_path(), &mut photos_paths).await;
    let total = photos_paths.len();

    if let Some(base) = task_base {
        base.set_total_count(total);
        base.set_status(TaskStatus::Running).await;
    }

    let data_path_buf = data_path.to_path_buf();
    let root_path_buf = root_path.to_path_buf();

    // Process EXIF/metadata concurrently — up to 8 photos at a time.
    let mut entries: Vec<AlbumPhotoItem> = futures::stream::iter(photos_paths)
        .map(|photo| {
            let data_path = data_path_buf.clone();
            let root_path = root_path_buf.clone();
            let task_base = task_base;
            async move {
                if let Some(base) = task_base {
                    base.set_cur_task(
                        photo
                            .file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or_default()
                            .to_string(),
                    )
                    .await;
                }
                let Ok(meta) = tokio::fs::metadata(&photo).await else {
                    if let Some(base) = task_base {
                        base.add_processed_count(1);
                    }
                    return None;
                };
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs());

                let mut exif_info = ensure_exif_cached(&data_path, photo.as_path())
                    .await
                    .unwrap_or_default();
                if exif_info.date.is_empty() {
                    exif_info.date = parse_date_from_parent_dirs(photo.as_path())
                        .unwrap_or_else(|| fallback_date_from_metadata(&meta));
                }

                let software_meta = read_image_software_meta(photo.as_path()).await;
                let tags = normalize_tags(&software_meta.tags);

                let item = AlbumPhotoItem {
                    path: relative_display_path(&root_path, photo.as_path()),
                    name: photo
                        .file_name()
                        .and_then(|f| f.to_str())
                        .unwrap_or_default()
                        .to_string(),
                    date: exif_info.date,
                    datetime: exif_info.datetime,
                    size: meta.len(),
                    modified,
                    camera_make: exif_info.camera_make,
                    camera_model: exif_info.camera_model,
                    lens: exif_info.lens,
                    focal_length: exif_info.focal_length,
                    aperture: exif_info.aperture,
                    shutter_speed: exif_info.shutter_speed,
                    iso: exif_info.iso,
                    width: exif_info.width,
                    height: exif_info.height,
                    rating: software_meta.rating,
                    color_label: software_meta.color_label,
                    tags,
                };
                if let Some(base) = task_base {
                    base.add_processed_count(1);
                }
                Some(item)
            }
        })
        .buffer_unordered(8)
        .filter_map(|x| async move { x })
        .collect()
        .await;

    entries.sort_by(|a, b| b.date.cmp(&a.date).then(a.name.cmp(&b.name)));

    let index = AlbumIndex {
        version: 2,
        entries,
    };

    let index_path = album_index_path(data_path, username, album_name);
    if let Some(parent) = index_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| AppError::Unknown(format!("create index dir failed: {e}")))?;
    }
    let index_data = serde_json::to_string(&index)
        .map_err(|e| AppError::Unknown(format!("serialize index failed: {e}")))?;
    tokio::fs::write(&index_path, index_data)
        .await
        .map_err(|e| AppError::Unknown(format!("write index failed: {e}")))?;

    Ok(index)
}

pub(crate) async fn import_album_files(
    cfg: &Arc<AppState>,
    root_path: &FsPath,
    username: &str,
    album_name: &str,
    sources: &[String],
    move_files: bool,
    conflict_action: crate::backend::files::ConflictAction,
    task_base: Option<&TaskBase>,
) -> Result<usize, AppError> {
    let album_base = albums_root(root_path).join(album_name);
    tokio::fs::create_dir_all(&album_base)
        .await
        .map_err(|e| AppError::Unknown(format!("create album dir failed: {e}")))?;

    let mut all_images = Vec::new();
    for source in sources {
        let full = into_host_path_safely(root_path, source)?;
        collect_album_images(full.as_path(), &mut all_images).await;
    }

    if let Some(base) = task_base {
        base.set_total_count(all_images.len());
        base.set_status(TaskStatus::Running).await;
    }

    if all_images.is_empty() {
        return Ok(0);
    }

    let mut imported = 0usize;
    for image in all_images {
        if let Some(base) = task_base {
            base.set_cur_task(
                image
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            )
            .await;
        }

        let exif = ensure_exif_cached(cfg.data_path.as_path(), image.as_path()).await?;
        let date = if exif.date.is_empty() {
            "unknown".to_string()
        } else {
            exif.date
        };
        let year = date.get(0..4).unwrap_or("unknown").to_string();
        let target_dir = album_base.join(&year).join(&date);
        tokio::fs::create_dir_all(&target_dir)
            .await
            .map_err(|e| AppError::Unknown(format!("create target dir failed: {e}")))?;

        let file_name = image
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("image");
        // Apply conflict action
        let dst_candidate = target_dir.join(file_name);
        let dst = if dst_candidate.exists() {
            match conflict_action {
                crate::backend::files::ConflictAction::Rename => {
                    unique_destination_path(target_dir.as_path(), file_name).await
                }
                crate::backend::files::ConflictAction::Overwrite => dst_candidate,
                crate::backend::files::ConflictAction::Skip => {
                    imported += 1;
                    if let Some(base) = task_base {
                        base.add_processed_count(1);
                    }
                    continue;
                }
            }
        } else {
            dst_candidate
        };

        if move_files {
            if move_file_or_dir(image.as_path(), dst.as_path())
                .await
                .is_err()
            {
                tokio::fs::copy(image.as_path(), dst.as_path())
                    .await
                    .map_err(|e| AppError::Unknown(format!("import copy fallback failed: {e}")))?;
                let _ = tokio::fs::remove_file(image.as_path()).await;
            }
        } else {
            tokio::fs::copy(image.as_path(), dst.as_path())
                .await
                .map_err(|e| AppError::Unknown(format!("import copy failed: {e}")))?;
        }

        // Also move/copy any XMP sidecar alongside the image.
        {
            let image_dir = image.parent().unwrap_or_else(|| FsPath::new(""));
            let target_dir = dst.parent().unwrap_or_else(|| FsPath::new(""));
            let stem = image.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let img_name_str = image.file_name().and_then(|s| s.to_str()).unwrap_or("");
            for xmp_filename in [format!("{stem}.xmp"), format!("{img_name_str}.xmp")] {
                let xmp_src = image_dir.join(&xmp_filename);
                if xmp_src.exists() {
                    let xmp_dst = target_dir.join(&xmp_filename);
                    if move_files {
                        let _ = move_file_or_dir(&xmp_src, &xmp_dst).await;
                    } else {
                        let _ = tokio::fs::copy(&xmp_src, &xmp_dst).await;
                    }
                    break;
                }
            }
        }

        imported += 1;

        if let Some(base) = task_base {
            base.add_processed_count(1);
        }
    }

    let _ = rebuild_album_index(
        cfg.data_path.as_path(),
        root_path,
        username,
        album_name,
        None,
    )
    .await;

    let _ = crate::backend::ai::queue_owner_ai_index(
        cfg.clone(),
        username.to_string(),
        Some(album_name.to_string()),
        "en",
    )
    .await;

    Ok(imported)
}

// ── Rebuild index task ───────────────────────────────────────────────────────

pub(crate) struct RebuildIndexTask {
    base: TaskBase,
    cfg: Option<Arc<AppState>>,
    data_path: PathBuf,
    root_path: PathBuf,
    owner: String,
    album_name: String,
    locale: String,
}

impl RebuildIndexTask {
    pub(crate) fn new(
        task_name: String,
        username: String,
        cfg: Option<Arc<AppState>>,
        data_path: PathBuf,
        root_path: PathBuf,
        owner: String,
        album_name: String,
        locale: String,
    ) -> Self {
        Self {
            base: TaskBase::new(task_name, username),
            cfg,
            data_path,
            root_path,
            owner,
            album_name,
            locale,
        }
    }
}

#[async_trait]
impl Task for RebuildIndexTask {
    fn base(&self) -> &TaskBase {
        &self.base
    }

    fn deduplication_key(&self) -> Option<String> {
        Some(format!("rebuild-index:{}:{}", self.owner, self.album_name))
    }

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Pending).await;
        match rebuild_album_index(
            &self.data_path,
            &self.root_path,
            &self.owner,
            &self.album_name,
            Some(&self.base),
        )
        .await
        {
            Ok(index) => {
                self.base
                    .set_cur_task(format!("Finished, {} photos indexed", index.entries.len()))
                    .await;
                self.base.set_status(TaskStatus::Finished).await;
                if let Some(cfg) = &self.cfg {
                    let _ = crate::backend::ai::queue_owner_ai_index(
                        cfg.clone(),
                        self.owner.clone(),
                        Some(self.album_name.clone()),
                        self.locale.as_str(),
                    )
                    .await;
                }
            }
            Err(e) => {
                self.base.set_error_msg(format!("{e}")).await;
                self.base.set_status(TaskStatus::Failed).await;
            }
        }
    }

    async fn cancel(&self) {}
}

pub(crate) struct ImportAlbumTask {
    base: TaskBase,
    cfg: Arc<AppState>,
    root_path: PathBuf,
    username: String,
    album_name: String,
    sources: Vec<String>,
    move_files: bool,
    conflict_action: crate::backend::files::ConflictAction,
    locale: String,
}

impl ImportAlbumTask {
    pub(crate) fn new(
        task_name: String,
        username: String,
        cfg: Arc<AppState>,
        root_path: PathBuf,
        album_name: String,
        sources: Vec<String>,
        move_files: bool,
        conflict_action: crate::backend::files::ConflictAction,
        locale: String,
    ) -> Self {
        Self {
            base: TaskBase::new(task_name, username.clone()),
            cfg,
            root_path,
            username,
            album_name,
            sources,
            move_files,
            conflict_action,
            locale,
        }
    }
}

#[async_trait]
impl Task for ImportAlbumTask {
    fn base(&self) -> &TaskBase {
        &self.base
    }

    async fn execute(&self) {
        self.base.set_status(TaskStatus::Pending).await;
        match import_album_files(
            &self.cfg,
            &self.root_path,
            &self.username,
            &self.album_name,
            &self.sources,
            self.move_files,
            self.conflict_action,
            Some(&self.base),
        )
        .await
        {
            Ok(imported) => {
                self.base
                    .set_cur_task(format!("Finished, {imported} photos imported"))
                    .await;
                self.base.set_status(TaskStatus::Finished).await;
                let _ = crate::backend::ai::queue_owner_ai_index(
                    self.cfg.clone(),
                    self.username.clone(),
                    Some(self.album_name.clone()),
                    self.locale.as_str(),
                )
                .await;
            }
            Err(e) => {
                self.base.set_error_msg(format!("{e}")).await;
                self.base.set_status(TaskStatus::Failed).await;
            }
        }
    }

    async fn cancel(&self) {}
}

pub(crate) async fn load_existing_album_index(
    data_path: &FsPath,
    username: &str,
    album_name: &str,
) -> Result<Option<AlbumIndex>, AppError> {
    let index_path = album_index_path(data_path, username, album_name);
    if !index_path.exists() {
        return Ok(None);
    }
    let raw = tokio::fs::read_to_string(&index_path)
        .await
        .map_err(|e| AppError::Unknown(format!("read index failed: {e}")))?;
    if let Ok(index) = serde_json::from_str::<AlbumIndex>(&raw)
        && index.version >= 2
    {
        return Ok(Some(index));
    }
    Ok(None)
}

pub(crate) async fn load_or_queue_album_index(
    cfg: &AppState,
    task_owner: &str,
    root_path: &FsPath,
    index_owner: &str,
    album_name: &str,
    locale: &str,
) -> Result<Option<AlbumIndex>, AppError> {
    if let Some(index) =
        load_existing_album_index(cfg.data_path.as_path(), index_owner, album_name).await?
    {
        return Ok(Some(index));
    }

    let task = Arc::new(RebuildIndexTask::new(
        trf(
            locale,
            "task.rebuild_index",
            &[("album", album_name.to_string())],
        ),
        task_owner.to_string(),
        None,
        cfg.data_path.clone(),
        root_path.to_path_buf(),
        index_owner.to_string(),
        album_name.to_string(),
        locale.to_string(),
    ));
    cfg.task_mgr.add_task_if_absent(task).await;
    Ok(None)
}

