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

use super::filters::FilterablePhoto;

// ── Album types ──────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CreateAlbumPayload {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAlbumPayload {
    pub(crate) dir_name: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) cover: Option<String>,
    #[serde(default)]
    pub(crate) description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DeleteAlbumPayload {
    pub(crate) dir_name: String,
}

#[derive(Debug, Deserialize)]
pub struct ImportAlbumPayload {
    pub(crate) album: String,
    pub(crate) sources: Vec<String>,
    pub(crate) move_files: Option<bool>,
    #[serde(default)]
    pub(crate) conflict: crate::backend::files::ConflictAction,
}

#[derive(Debug, Deserialize)]
pub struct AlbumPhotosQuery {
    pub(crate) album: String,
    pub(crate) from: Option<String>,
    pub(crate) to: Option<String>,
    pub(crate) tags: Option<String>,
    pub(crate) ratings: Option<String>,
    pub(crate) color_label: Option<String>,
    /// Legacy single-value camera filter (exact match).
    pub(crate) camera: Option<String>,
    /// Multi-value camera filter: comma-separated list, photo must match any one.
    pub(crate) cameras: Option<String>,
    /// Legacy single-value lens filter (exact match).
    pub(crate) lens: Option<String>,
    /// Multi-value lens filter: comma-separated list, photo must match any one.
    pub(crate) lenses: Option<String>,
    /// Legacy single-value format filter (exact extension match).
    pub(crate) format: Option<String>,
    /// Multi-value format filter: comma-separated list of extensions, photo must match any one.
    pub(crate) formats: Option<String>,
    /// Substring match against filename / path / tags (case-insensitive).
    pub(crate) q: Option<String>,
    /// Restrict to photos containing faces of these people (comma-separated person ids).
    pub(crate) person_ids: Option<String>,
    pub(crate) sort: Option<String>,
    pub(crate) offset: Option<u32>,
    pub(crate) limit: Option<u32>,
}

/// Logical filter request body (preferred). `filters` uses AND/OR/AND-NOT clauses.
#[derive(Debug, Deserialize)]
pub struct AlbumPhotosBody {
    pub(crate) album: String,
    #[serde(default)]
    pub(crate) filters: Vec<super::filters::FilterClause>,
    pub(crate) sort: Option<String>,
    pub(crate) offset: Option<u32>,
    pub(crate) limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StringListInput {
    List(Vec<String>),
    CommaSeparated(String),
}

fn deserialize_optional_string_list<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<StringListInput>::deserialize(deserializer).map(|value| {
        value.map(|input| match input {
            StringListInput::List(values) => values,
            StringListInput::CommaSeparated(value) => value
                .split(',')
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .collect(),
        })
    })
}

#[derive(Debug, Deserialize)]
pub struct UpdatePhotoMetaPayload {
    pub(crate) path: String,
    #[serde(default)]
    pub(crate) album: Option<String>,
    #[serde(default)]
    pub(crate) tags: Option<Vec<String>>,
    #[serde(default)]
    pub(crate) rating: Option<u8>,
    #[serde(default, deserialize_with = "deserialize_optional_string_list")]
    pub(crate) color_label: Option<Vec<String>>,
}

// ── Color Label ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum ColorLabel {
    #[default]
    None = 255,
    Red = 0,
    Yellow = 1,
    Green = 2,
    Blue = 3,
    Purple = 4,
}

impl ColorLabel {
    pub fn from_darktable(n: u8) -> Self {
        match n {
            0 => Self::Red,
            1 => Self::Yellow,
            2 => Self::Green,
            3 => Self::Blue,
            4 => Self::Purple,
            _ => Self::None,
        }
    }

    pub fn to_darktable(self) -> Option<u8> {
        match self {
            Self::None => None,
            Self::Red => Some(0),
            Self::Yellow => Some(1),
            Self::Green => Some(2),
            Self::Blue => Some(3),
            Self::Purple => Some(4),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Red => "red",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Purple => "purple",
        }
    }

    pub fn from_str_label(s: &str) -> Self {
        // Handle legacy comma-separated format by taking the first element
        let first = s.split(',').next().unwrap_or("").trim();
        match first {
            "red" => Self::Red,
            "yellow" => Self::Yellow,
            "green" => Self::Green,
            "blue" => Self::Blue,
            "purple" => Self::Purple,
            _ => Self::None,
        }
    }
}

impl Serialize for ColorLabel {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ColorLabel {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::from_str_label(&s))
    }
}

/// Deserialize `Vec<ColorLabel>` accepting either a JSON array or a legacy
/// comma-separated string (for backward compatibility with old index files).
mod de_color_labels {
    use super::ColorLabel;
    use serde::Deserializer;
    use serde::de::{self, SeqAccess, Visitor};
    use std::fmt;

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ColorLabel>, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Vec<ColorLabel>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("array of color label strings or legacy comma-separated string")
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Vec<ColorLabel>, E> {
                Ok(s.split(',')
                    .map(|x| ColorLabel::from_str_label(x.trim()))
                    .filter(|c| *c != ColorLabel::None)
                    .collect())
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<ColorLabel>, A::Error> {
                let mut labels = Vec::new();
                while let Some(s) = seq.next_element::<String>()? {
                    let cl = ColorLabel::from_str_label(&s);
                    if cl != ColorLabel::None {
                        labels.push(cl);
                    }
                }
                Ok(labels)
            }
        }
        d.deserialize_any(V)
    }
}

// ── ImageSoftwareMeta ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct ImageSoftwareMeta {
    #[serde(default)]
    pub(crate) tags: Vec<String>,
    #[serde(default)]
    pub(crate) rating: u8,
    #[serde(default, deserialize_with = "de_color_labels::deserialize")]
    pub(crate) color_label: Vec<ColorLabel>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ExifCacheEntry {
    #[serde(default)]
    pub(crate) date: String,
    #[serde(default)]
    pub(crate) datetime: String,
    #[serde(default)]
    pub(crate) camera_make: String,
    #[serde(default)]
    pub(crate) camera_model: String,
    #[serde(default)]
    pub(crate) lens: String,
    #[serde(default)]
    pub(crate) focal_length: String,
    #[serde(default)]
    pub(crate) aperture: String,
    #[serde(default)]
    pub(crate) shutter_speed: String,
    #[serde(default)]
    pub(crate) iso: Option<u32>,
    #[serde(default)]
    pub(crate) width: u32,
    #[serde(default)]
    pub(crate) height: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AlbumPhotoItem {
    pub(crate) path: String,
    pub(crate) name: String,
    pub(crate) date: String,
    pub(crate) datetime: String,
    pub(crate) size: u64,
    pub(crate) modified: Option<u64>,
    pub(crate) camera_make: String,
    pub(crate) camera_model: String,
    pub(crate) lens: String,
    pub(crate) focal_length: String,
    pub(crate) aperture: String,
    pub(crate) shutter_speed: String,
    pub(crate) iso: Option<u32>,
    #[serde(default)]
    pub(crate) width: u32,
    #[serde(default)]
    pub(crate) height: u32,
    pub(crate) rating: u8,
    #[serde(default, deserialize_with = "de_color_labels::deserialize")]
    pub(crate) color_label: Vec<ColorLabel>,
    pub(crate) tags: Vec<String>,
}

impl super::filters::FilterablePhoto for AlbumPhotoItem {
    fn path(&self) -> &str {
        &self.path
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn date(&self) -> &str {
        &self.date
    }
    fn rating(&self) -> u8 {
        self.rating
    }
    fn color_label_keys(&self) -> Vec<String> {
        self.color_label
            .iter()
            .copied()
            .map(ColorLabel::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }
    fn camera_label(&self) -> String {
        format!("{} {}", self.camera_make, self.camera_model)
            .trim()
            .to_string()
    }
    fn lens(&self) -> &str {
        &self.lens
    }
    fn tag_list(&self) -> &[String] {
        &self.tags
    }
    fn file_ext_lower(&self) -> String {
        std::path::Path::new(&self.name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AlbumIndex {
    pub(crate) version: u32,
    pub(crate) entries: Vec<AlbumPhotoItem>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct AlbumMeta {
    #[serde(default)]
    pub(crate) name: String,
    /// Stable cache key for `cache/index/{user}/{cache}.json`.
    /// Official album dir may be renamed without moving this pointer.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) cache: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) cover: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) shares: Vec<AlbumShare>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AlbumShare {
    pub username: String,
    #[serde(default)]
    pub can_write: bool,
    #[serde(default)]
    pub can_delete: bool,
}

