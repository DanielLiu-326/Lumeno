//! Album-level LUT and preset files. The id is the content hash. The display name is not a path.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::cube::{lookup_lut, parse_cube, remember_lut};
use super::error::GradeError;

pub const PRESET_MAX_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    Lut,
    Preset,
}

impl ResourceKind {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "lut" => Some(Self::Lut),
            "preset" => Some(Self::Preset),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lut => "lut",
            Self::Preset => "preset",
        }
    }

    fn prefix(self) -> &'static str {
        match self {
            Self::Lut => "lut:",
            Self::Preset => "preset:",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ResourceRecord {
    pub id: String,
    pub hash: String,
    pub display_name: String,
    pub byte_size: u64,
    pub status: &'static str,
    pub ref_count: usize,
    pub ref_paths: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lut_size: Option<u32>,
    pub created: bool,
}

#[derive(Serialize, Deserialize)]
struct MetaFile {
    display_name: String,
    byte_size: u64,
    media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lut_size: Option<u32>,
}

struct Hit {
    count: usize,
    paths: Vec<String>,
}

pub fn content_hash(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut text = String::with_capacity(64);
    for byte in digest {
        text.push(HEX[(byte >> 4) as usize] as char);
        text.push(HEX[(byte & 0xf) as usize] as char);
    }
    text
}

pub fn is_content_hash(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

pub fn sanitize_display_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_control() || ch == '/' || ch == '\\' {
            continue;
        }
        out.push(ch);
        if out.len() >= 128 {
            break;
        }
    }
    let trimmed = out.trim().trim_start_matches('.');
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn import_lut(
    root: &Path,
    display_name: &str,
    bytes: &[u8],
) -> Result<ResourceRecord, GradeError> {
    let parsed = parse_cube(bytes)?;
    let hash = content_hash(bytes);
    let edge = parsed.size;
    remember_lut(&format!("lut:{hash}"), std::sync::Arc::new(parsed));
    let meta = MetaFile {
        display_name: sanitize_display_name(display_name),
        byte_size: bytes.len() as u64,
        media_type: "text/plain".to_string(),
        lut_size: Some(edge),
    };
    write_new(root, ResourceKind::Lut, &hash, bytes, meta)
}

/// Load a cube into the grade cache when this process has not parsed it yet.
pub fn ensure_grade_lut(root: &Path, id: &str) -> Result<(), GradeError> {
    if id.is_empty() || lookup_lut(id).is_some() {
        return Ok(());
    }
    let Some(hash) = id.strip_prefix("lut:") else {
        return Err(GradeError::Rejected("bad-id"));
    };
    if !is_content_hash(hash) {
        return Err(GradeError::Rejected("bad-id"));
    }
    let path = blob_file(root, ResourceKind::Lut, hash)?;
    let bytes = fs::read(&path).map_err(|_| GradeError::Rejected("missing"))?;
    let parsed = parse_cube(&bytes)?;
    remember_lut(id, std::sync::Arc::new(parsed));
    Ok(())
}

pub fn user_root_from_photo(photo: &Path) -> Option<PathBuf> {
    let mut cursor = photo.parent()?;
    loop {
        if cursor.file_name().and_then(|name| name.to_str()) == Some("albums") {
            return cursor.parent().map(Path::to_path_buf);
        }
        cursor = cursor.parent()?;
    }
}

pub fn import_preset(
    root: &Path,
    display_name: &str,
    bytes: &[u8],
) -> Result<ResourceRecord, GradeError> {
    let luts = preset_lut_refs(bytes)?;
    for hash in &luts {
        if !blob_file(root, ResourceKind::Lut, hash)?.is_file() {
            return Err(GradeError::Rejected("preset-lut"));
        }
    }
    let hash = content_hash(bytes);
    let meta = MetaFile {
        display_name: sanitize_display_name(display_name),
        byte_size: bytes.len() as u64,
        media_type: "application/json".to_string(),
        lut_size: None,
    };
    write_new(root, ResourceKind::Preset, &hash, bytes, meta)
}

pub fn list_resources(root: &Path, kind: ResourceKind) -> Result<Vec<ResourceRecord>, GradeError> {
    let hits = scan_refs(root)?;
    let mut rows = Vec::new();
    let dir = meta_dir(root, kind)?;
    if !dir.is_dir() {
        return Ok(rows);
    }
    let entries = fs::read_dir(&dir).map_err(|_| GradeError::Rejected("io"))?;
    for entry in entries {
        let entry = entry.map_err(|_| GradeError::Rejected("io"))?;
        if entry
            .file_type()
            .map(|kind| kind.is_symlink())
            .unwrap_or(true)
        {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(hash) = name.strip_suffix(".json") else {
            continue;
        };
        if !is_content_hash(hash) {
            continue;
        }
        let meta = read_meta(&entry.path())?;
        let blob = blob_file(root, kind, hash)?;
        let ready = blob.is_file()
            && fs::symlink_metadata(&blob)
                .map(|info| info.file_type().is_file() && info.len() == meta.byte_size)
                .unwrap_or(false);
        let id = format!("{}{hash}", kind.prefix());
        let hit = hits.get(&id);
        rows.push(ResourceRecord {
            id,
            hash: hash.to_string(),
            display_name: meta.display_name,
            byte_size: meta.byte_size,
            status: if ready { "ready" } else { "damaged" },
            ref_count: hit.map(|item| item.count).unwrap_or(0),
            ref_paths: hit.map(|item| item.paths.clone()).unwrap_or_default(),
            lut_size: meta.lut_size,
            created: false,
        });
    }
    rows.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    Ok(rows)
}

pub fn rename_resource(
    root: &Path,
    kind: ResourceKind,
    hash: &str,
    display_name: &str,
) -> Result<ResourceRecord, GradeError> {
    if !is_content_hash(hash) {
        return Err(GradeError::Rejected("bad-id"));
    }
    let path = meta_file(root, kind, hash)?;
    if !path.is_file() {
        return Err(GradeError::Rejected("missing"));
    }
    let mut meta = read_meta(&path)?;
    meta.display_name = sanitize_display_name(display_name);
    write_json(&path, &meta)?;
    let mut rows = list_resources(root, kind)?;
    rows.retain(|row| row.hash == hash);
    rows.pop().ok_or(GradeError::Rejected("missing"))
}

pub fn delete_resource(root: &Path, kind: ResourceKind, hash: &str) -> Result<(), GradeError> {
    if !is_content_hash(hash) {
        return Err(GradeError::Rejected("bad-id"));
    }
    let id = format!("{}{hash}", kind.prefix());
    let hits = scan_refs(root)?;
    if hits.get(&id).map(|hit| hit.count).unwrap_or(0) > 0 {
        return Err(GradeError::Rejected("referenced"));
    }
    let meta = meta_file(root, kind, hash)?;
    let blob = blob_file(root, kind, hash)?;
    if !meta.is_file() && !blob.is_file() {
        return Err(GradeError::Rejected("missing"));
    }
    remove_file(&meta)?;
    remove_file(&blob)?;
    Ok(())
}

fn write_new(
    root: &Path,
    kind: ResourceKind,
    hash: &str,
    bytes: &[u8],
    meta: MetaFile,
) -> Result<ResourceRecord, GradeError> {
    let blob = blob_file(root, kind, hash)?;
    let meta_path = meta_file(root, kind, hash)?;
    let created = !blob.is_file();
    if created {
        write_bytes(&blob, bytes)?;
    }
    if !meta_path.is_file() {
        write_json(&meta_path, &meta)?;
    }
    let stored = read_meta(&meta_path)?;
    Ok(ResourceRecord {
        id: format!("{}{hash}", kind.prefix()),
        hash: hash.to_string(),
        display_name: stored.display_name,
        byte_size: stored.byte_size,
        status: "ready",
        ref_count: 0,
        ref_paths: Vec::new(),
        lut_size: stored.lut_size,
        created,
    })
}

fn preset_lut_refs(bytes: &[u8]) -> Result<Vec<String>, GradeError> {
    if bytes.len() > PRESET_MAX_BYTES {
        return Err(GradeError::Rejected("preset-size"));
    }
    let value: Value =
        serde_json::from_slice(bytes).map_err(|_| GradeError::Rejected("preset-invalid"))?;
    if !value.is_object() {
        return Err(GradeError::Rejected("preset-invalid"));
    }
    let mut luts = Vec::new();
    walk_preset(&value, &mut luts)?;
    Ok(luts)
}

fn walk_preset(value: &Value, luts: &mut Vec<String>) -> Result<(), GradeError> {
    match value {
        Value::String(text) => {
            if text.contains("..") || text.contains('\\') || text.starts_with('/') {
                return Err(GradeError::Rejected("preset-invalid"));
            }
            if text.starts_with("preset:") || text.contains("alias:preset:") {
                return Err(GradeError::Rejected("preset-invalid"));
            }
            if let Some(hash) = text.strip_prefix("lut:") {
                if !is_content_hash(hash) {
                    return Err(GradeError::Rejected("preset-invalid"));
                }
                luts.push(hash.to_string());
            }
            Ok(())
        }
        Value::Array(items) => {
            for item in items {
                walk_preset(item, luts)?;
            }
            Ok(())
        }
        Value::Object(map) => {
            for item in map.values() {
                walk_preset(item, luts)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn scan_refs(root: &Path) -> Result<HashMap<String, Hit>, GradeError> {
    let mut hits = HashMap::new();
    let albums = root.join("albums");
    if albums.is_dir() {
        walk_albums(&albums, &albums, &mut hits)?;
    }
    let presets = blob_dir(root, ResourceKind::Preset)?;
    if presets.is_dir() {
        note_hashed_files(&presets, "preset", &mut hits)?;
    }
    Ok(hits)
}

fn note_hashed_files(
    dir: &Path,
    label: &str,
    hits: &mut HashMap<String, Hit>,
) -> Result<(), GradeError> {
    let entries = fs::read_dir(dir).map_err(|_| GradeError::Rejected("io"))?;
    for entry in entries.flatten() {
        if entry
            .file_type()
            .map(|kind| kind.is_symlink())
            .unwrap_or(true)
        {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            note_hashed_files(&path, label, hits)?;
            continue;
        }
        let Some(hash) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if is_content_hash(hash) {
            note_file(&path, &format!("{label}/{hash}"), hits)?;
        }
    }
    Ok(())
}

fn walk_albums(
    dir: &Path,
    albums: &Path,
    hits: &mut HashMap<String, Hit>,
) -> Result<(), GradeError> {
    let entries = fs::read_dir(dir).map_err(|_| GradeError::Rejected("io"))?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with("__") || name.starts_with('.') {
            continue;
        }
        let file_type = entry.file_type().map_err(|_| GradeError::Rejected("io"))?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            walk_albums(&path, albums, hits)?;
            continue;
        }
        if !name.ends_with(".json") {
            continue;
        }
        let rel = path
            .strip_prefix(albums)
            .map(|item| item.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default();
        note_file(&path, &rel, hits)?;
    }
    Ok(())
}

fn note_file(path: &Path, label: &str, hits: &mut HashMap<String, Hit>) -> Result<(), GradeError> {
    let info = fs::symlink_metadata(path).map_err(|_| GradeError::Rejected("io"))?;
    if !info.file_type().is_file() || info.len() > 1024 * 1024 {
        return Ok(());
    }
    let text = fs::read_to_string(path).unwrap_or_default();
    let mut found = Vec::new();
    collect_ids(&text, &mut found);
    for id in found {
        let hit = hits.entry(id).or_insert(Hit {
            count: 0,
            paths: Vec::new(),
        });
        hit.count += 1;
        if hit.paths.len() < 8 && !hit.paths.iter().any(|item| item == label) {
            hit.paths.push(label.to_string());
        }
    }
    Ok(())
}

fn collect_ids(text: &str, out: &mut Vec<String>) {
    for prefix in ["lut:", "preset:"] {
        let mut rest = text;
        while let Some(index) = rest.find(prefix) {
            let tail = &rest[index + prefix.len()..];
            let hash: String = tail.chars().take(64).collect();
            if is_content_hash(&hash) {
                let id = format!("{prefix}{hash}");
                if !out.iter().any(|item| item == &id) {
                    out.push(id);
                }
            }
            rest = &rest[index + prefix.len()..];
        }
    }
}

fn read_meta(path: &Path) -> Result<MetaFile, GradeError> {
    let bytes = fs::read(path).map_err(|_| GradeError::Rejected("io"))?;
    serde_json::from_slice(&bytes).map_err(|_| GradeError::Rejected("io"))
}

fn write_json(path: &Path, meta: &MetaFile) -> Result<(), GradeError> {
    let bytes = serde_json::to_vec_pretty(meta).map_err(|_| GradeError::Rejected("io"))?;
    write_bytes(path, &bytes)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), GradeError> {
    let parent = path.parent().ok_or(GradeError::Rejected("io"))?;
    fs::create_dir_all(parent).map_err(|_| GradeError::Rejected("io"))?;
    if !stays_inside(parent)? {
        return Err(GradeError::Rejected("bad-id"));
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("blob");
    let tmp = parent.join(format!(".{file_name}.tmp"));
    fs::write(&tmp, bytes).map_err(|_| GradeError::Rejected("io"))?;
    fs::rename(&tmp, path).map_err(|_| GradeError::Rejected("io"))?;
    Ok(())
}

fn remove_file(path: &Path) -> Result<(), GradeError> {
    match fs::symlink_metadata(path) {
        Ok(info) if info.file_type().is_symlink() => Err(GradeError::Rejected("bad-id")),
        Ok(_) => fs::remove_file(path).map_err(|_| GradeError::Rejected("io")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(GradeError::Rejected("io")),
    }
}

fn stays_inside(parent: &Path) -> Result<bool, GradeError> {
    let resource = parent;
    let mut cursor = resource;
    while cursor.file_name().and_then(|name| name.to_str()) != Some("__resource__") {
        let Some(next) = cursor.parent() else {
            return Ok(false);
        };
        cursor = next;
    }
    let canon_parent = parent
        .canonicalize()
        .map_err(|_| GradeError::Rejected("io"))?;
    let canon_root = cursor
        .canonicalize()
        .map_err(|_| GradeError::Rejected("io"))?;
    Ok(canon_parent.starts_with(canon_root))
}

fn resource_root(root: &Path, kind: ResourceKind) -> PathBuf {
    root.join("albums").join("__resource__").join(kind.as_str())
}

fn meta_dir(root: &Path, kind: ResourceKind) -> Result<PathBuf, GradeError> {
    Ok(resource_root(root, kind).join("meta"))
}

fn meta_file(root: &Path, kind: ResourceKind, hash: &str) -> Result<PathBuf, GradeError> {
    if !is_content_hash(hash) {
        return Err(GradeError::Rejected("bad-id"));
    }
    Ok(meta_dir(root, kind)?.join(format!("{hash}.json")))
}

fn blob_dir(root: &Path, kind: ResourceKind) -> Result<PathBuf, GradeError> {
    Ok(resource_root(root, kind).join("blobs"))
}

fn blob_file(root: &Path, kind: ResourceKind, hash: &str) -> Result<PathBuf, GradeError> {
    if !is_content_hash(hash) {
        return Err(GradeError::Rejected("bad-id"));
    }
    // The first byte spreads files. It is taken from the hash, never from a name.
    Ok(blob_dir(root, kind)?.join(&hash[..2]).join(hash))
}

const HEX: &[u8; 16] = b"0123456789abcdef";

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "lumeno-resource-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn cube() -> String {
        "LUT_3D_SIZE 2\n0 0 0\n1 0 0\n0 1 0\n1 1 0\n0 0 1\n1 0 1\n0 1 1\n1 1 1\n".to_string()
    }

    #[test]
    fn the_same_bytes_keep_one_id_when_renamed() {
        let root = temp_root();
        let first = import_lut(&root, "Film.cube", cube().as_bytes()).unwrap();
        assert!(first.created);
        let again = import_lut(&root, "../other.cube", cube().as_bytes()).unwrap();
        assert!(!again.created);
        assert_eq!(first.hash, again.hash);
        assert_eq!(again.display_name, "Film.cube");
        let renamed = rename_resource(&root, ResourceKind::Lut, &first.hash, "Summer").unwrap();
        assert_eq!(renamed.hash, first.hash);
        assert_eq!(renamed.display_name, "Summer");
        let blob = blob_file(&root, ResourceKind::Lut, &first.hash).unwrap();
        assert!(blob.is_file());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_referenced_lut_is_not_deleted() {
        let root = temp_root();
        let imported = import_lut(&root, "Film.cube", cube().as_bytes()).unwrap();
        let doc = root.join("albums").join("trips").join("grade.json");
        fs::create_dir_all(doc.parent().unwrap()).unwrap();
        fs::write(&doc, format!("{{\"lut\":\"{}\"}}", imported.id)).unwrap();
        let err = delete_resource(&root, ResourceKind::Lut, &imported.hash).unwrap_err();
        assert!(matches!(err, GradeError::Rejected("referenced")));
        assert!(
            blob_file(&root, ResourceKind::Lut, &imported.hash)
                .unwrap()
                .is_file()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_dotdot_id_is_rejected() {
        let root = temp_root();
        let err = delete_resource(&root, ResourceKind::Lut, "../secret").unwrap_err();
        assert!(matches!(err, GradeError::Rejected("bad-id")));
        let _ = fs::remove_dir_all(&root);
    }
}
