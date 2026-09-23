use serde::{Deserialize, Serialize};

pub const WALLPAPER_PRESET_IDS: &[&str] = &["lumen", "dusk", "mist", "meadow"];

fn default_preset_id() -> String {
    "lumen".to_owned()
}

fn default_source_type() -> String {
    "preset".to_owned()
}

fn default_dark_mode() -> String {
    "auto".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WallpaperSource {
    #[serde(rename = "type", default = "default_source_type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hex: Option<String>,
}

impl Default for WallpaperSource {
    fn default() -> Self {
        Self::preset("lumen")
    }
}

impl WallpaperSource {
    pub fn preset(id: &str) -> Self {
        Self {
            kind: "preset".to_owned(),
            id: Some(id.to_owned()),
            file: None,
            hex: None,
        }
    }

    pub fn image(file: &str) -> Self {
        Self {
            kind: "image".to_owned(),
            id: None,
            file: Some(file.to_owned()),
            hex: None,
        }
    }

    pub fn normalize(self) -> Self {
        match self.kind.as_str() {
            "image" => {
                let Some(file) = self.file.filter(|name| is_safe_wallpaper_filename(name)) else {
                    return Self::preset("lumen");
                };
                Self {
                    kind: "image".to_owned(),
                    id: None,
                    file: Some(file),
                    hex: None,
                }
            }
            "color" => {
                let Some(hex) = self.hex.as_deref().and_then(normalize_hex) else {
                    return Self::preset("lumen");
                };
                Self {
                    kind: "color".to_owned(),
                    id: None,
                    file: None,
                    hex: Some(hex),
                }
            }
            _ => {
                let id = self
                    .id
                    .filter(|value| WALLPAPER_PRESET_IDS.contains(&value.as_str()))
                    .unwrap_or_else(default_preset_id);
                Self::preset(&id)
            }
        }
    }

    pub fn image_file(&self) -> Option<&str> {
        if self.kind == "image" {
            self.file.as_deref()
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WallpaperPrefs {
    #[serde(default = "default_dark_mode")]
    pub dark_mode: String,
    #[serde(default)]
    pub source: WallpaperSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dark_source: Option<WallpaperSource>,
}

impl Default for WallpaperPrefs {
    fn default() -> Self {
        Self {
            dark_mode: default_dark_mode(),
            source: WallpaperSource::default(),
            dark_source: None,
        }
    }
}

impl WallpaperPrefs {
    pub fn normalize(self) -> Self {
        let dark_mode = if self.dark_mode == "separate" {
            "separate"
        } else {
            "auto"
        };
        Self {
            dark_mode: dark_mode.to_owned(),
            source: self.source.normalize(),
            dark_source: self.dark_source.map(WallpaperSource::normalize),
        }
    }

    pub fn image_files(&self) -> Vec<String> {
        let mut files = Vec::new();
        if let Some(file) = self.source.image_file() {
            files.push(file.to_owned());
        }
        if let Some(file) = self
            .dark_source
            .as_ref()
            .and_then(WallpaperSource::image_file)
        {
            if !files.iter().any(|existing| existing == file) {
                files.push(file.to_owned());
            }
        }
        files
    }

    pub fn with_slot(mut self, slot: &str, source: WallpaperSource) -> Self {
        let source = source.normalize();
        if slot == "dark_source" {
            self.dark_source = Some(source);
            if self.dark_mode != "separate" {
                self.dark_mode = "separate".to_owned();
            }
        } else {
            self.source = source;
        }
        self
    }
}

pub fn is_safe_wallpaper_filename(name: &str) -> bool {
    if name.is_empty() || name.len() > 80 {
        return false;
    }
    if !name.ends_with(".webp") {
        return false;
    }
    name.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
        && !name.contains("..")
}

/// Uploaded wallpapers are stored as `{username}_{uuid}.webp`.
pub fn wallpaper_filename_owner(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".webp")?;
    let (owner, uuid) = stem.rsplit_once('_')?;
    if owner.is_empty() {
        return None;
    }
    let ok = uuid::Uuid::parse_str(uuid).is_ok()
        && owner
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    ok.then_some(owner)
}

fn normalize_hex(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    let valid = match hex.len() {
        3 | 6 => hex.chars().all(|ch| ch.is_ascii_hexdigit()),
        _ => false,
    };
    if !valid {
        return None;
    }
    Some(format!("#{}", hex.to_ascii_lowercase()))
}
