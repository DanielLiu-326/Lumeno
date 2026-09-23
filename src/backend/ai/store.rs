use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use super::faces::FACE_DETECTOR_REVISION;
use super::models::{
    AiFaceRecord, AiIndexedAlbumSummary, AiIndexedPhotoRecord, LoadedOwnerIndex,
    NormalizedAiIndexRequest, OwnerIndexStatus, StoredAiGenerationManifest,
};
use super::provider::{AiCapabilities, AiProviderHealth};

fn photo_record_from_stable_key(
    stable_key: &str,
    cache_id: &str,
    content_signature: &str,
    provider_revision: Option<String>,
) -> Option<AiIndexedPhotoRecord> {
    let mut parts = stable_key.splitn(3, '|');
    let owner = parts.next()?.to_string();
    let source_key = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let album = path
        .strip_prefix("albums/")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(source_key.as_str())
        .to_string();
    let (size_bytes, modified_unix_seconds) = parse_content_signature(content_signature);
    let stable_id = Sha256::digest(stable_key.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Some(AiIndexedPhotoRecord {
        photo_id: 0,
        owner,
        album,
        source_key,
        cache_id: cache_id.to_string(),
        path,
        stable_key: stable_key.to_string(),
        stable_id,
        size_bytes,
        modified_unix_seconds,
        content_signature: content_signature.to_string(),
        width: None,
        height: None,
        provider_revision,
        scan_state: "reused".to_string(),
        image_embedding_state: "pending".to_string(),
        duplicate_hash_state: "skipped".to_string(),
        duplicate_phash_u64: None,
        duplicate_phash_valid: false,
        face_embedding_state: "pending".to_string(),
        reused_from_generation: None,
        record_error: None,
        indexed_at_unix_ms: 0,
    })
}

fn parse_content_signature(signature: &str) -> (u64, Option<u64>) {
    let mut parts = signature.splitn(2, ':');
    let size = parts
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let modified = parts.next().and_then(|s| s.parse::<u64>().ok());
    (size, modified)
}

fn io_error_with_path(context: &str, path: &Path, error: std::io::Error) -> String {
    let hint = if error.kind() == std::io::ErrorKind::PermissionDenied {
        format!(
            " — {} is not writable (fix: chown/chmod the data cache tree, e.g. chown -R \"$(whoami)\" …/dist/data/cache)",
            path.display()
        )
    } else {
        String::new()
    };
    format!("{context} {}: {error}{hint}", path.display())
}

fn ensure_dir_writable(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|error| io_error_with_path("create dir", path, error))?;
    let probe = path.join(".write_probe");
    std::fs::write(&probe, b"").map_err(|error| io_error_with_path("write probe", path, error))?;
    let _ = std::fs::remove_file(probe);
    Ok(())
}

pub struct FileAiIndexStore {
    /// `dist/data/cache/ai/` — album embeddings under `{root}/{owner}/albums/{cache_id}/`.
    root: PathBuf,
}

impl FileAiIndexStore {
    pub fn new(root: PathBuf) -> Self {
        if let Err(error) = ensure_dir_writable(&root) {
            eprintln!(
                "warning: AI index cache not writable at {}: {error}",
                root.display()
            );
        }
        Self { root }
    }

    pub fn check_writable(&self) -> Result<(), String> {
        ensure_dir_writable(&self.root)
    }

    fn ensure_album_layout(&self, owner: &str, cache_id: &str) -> Result<PathBuf, String> {
        let album_root = self.album_dir(owner, cache_id);
        ensure_dir_writable(&album_root)?;
        Ok(album_root)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn owner_dir(&self, owner: &str) -> PathBuf {
        self.root.join(owner)
    }

    /// Per-album AI index root, keyed by `AlbumMeta.cache`.
    pub fn album_dir(&self, owner: &str, cache_id: &str) -> PathBuf {
        self.owner_dir(owner).join("albums").join(cache_id)
    }

    pub fn purge_owner(&self, owner: &str) -> Result<(), String> {
        let owner_root = self.owner_dir(owner);
        if !owner_root.exists() {
            return Ok(());
        }
        std::fs::remove_dir_all(&owner_root)
            .map_err(|error| format!("purge ai index for {owner} failed: {error}"))
    }

    pub fn owner_has_current_index(&self, owner: &str) -> bool {
        !self.list_album_cache_ids(owner).is_empty()
    }

    pub(crate) fn list_album_cache_ids(&self, owner: &str) -> Vec<String> {
        let albums_root = self.owner_dir(owner).join("albums");
        let Ok(entries) = std::fs::read_dir(albums_root) else {
            return Vec::new();
        };
        let mut ids = entries
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .filter(|entry| {
                let path = entry.path();
                path.join("CURRENT").exists()
                    || Self::dir_has_json(&path.join("embeddings"))
                    || Self::dir_has_json(&path.join("faces"))
            })
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    pub(crate) async fn list_album_cache_ids_async(&self, owner: &str) -> Vec<String> {
        let albums_root = self.owner_dir(owner).join("albums");
        let Ok(mut entries) = tokio::fs::read_dir(albums_root).await else {
            return Vec::new();
        };
        let mut ids = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let is_dir = tokio::fs::metadata(&path)
                .await
                .map(|m| m.is_dir())
                .unwrap_or(false);
            if !is_dir {
                continue;
            }
            let has_current = tokio::fs::try_exists(path.join("CURRENT"))
                .await
                .unwrap_or(false);
            let has_cache = Self::dir_has_json_async(&path.join("embeddings")).await
                || Self::dir_has_json_async(&path.join("faces")).await;
            if has_current || has_cache {
                ids.push(entry.file_name().to_string_lossy().to_string());
            }
        }
        ids.sort();
        ids
    }

    fn dir_has_json(dir: &Path) -> bool {
        std::fs::read_dir(dir).is_ok_and(|entries| {
            entries.flatten().any(|entry| {
                entry.path().extension().and_then(|value| value.to_str()) == Some("json")
            })
        })
    }

    async fn dir_has_json_async(dir: &Path) -> bool {
        let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
            return false;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.path().extension().and_then(|value| value.to_str()) == Some("json") {
                return true;
            }
        }
        false
    }

    fn read_current_generation(dir: &Path) -> Option<String> {
        std::fs::read_to_string(dir.join("CURRENT"))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    async fn read_current_generation_async(dir: &Path) -> Option<String> {
        tokio::fs::read_to_string(dir.join("CURRENT"))
            .await
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    pub(crate) fn current_album_generation(&self, owner: &str, cache_id: &str) -> Option<String> {
        Self::read_current_generation(&self.album_dir(owner, cache_id))
    }

    pub(crate) async fn current_album_generation_async(
        &self,
        owner: &str,
        cache_id: &str,
    ) -> Option<String> {
        Self::read_current_generation_async(&self.album_dir(owner, cache_id)).await
    }

    pub(crate) async fn load_album_index_from_file_cache_async(
        &self,
        owner: &str,
        cache_id: &str,
        include_faces: bool,
    ) -> Option<LoadedOwnerIndex> {
        use super::album_cache::AlbumAiCache;
        let cache = AlbumAiCache::new(&self.root, owner, cache_id);
        let generation = self
            .current_album_generation_async(owner, cache_id)
            .await
            .unwrap_or_else(|| format!("file-cache:{cache_id}"));
        let face_files = if include_faces {
            cache
                .list_face_photos_async()
                .await
                .ok()
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let embedding_maps = if let Some(snapshot) = cache
            .load_embedding_snapshot_async(Some(generation.as_str()))
            .await
        {
            Some(Self::build_embedding_maps_from_snapshot(
                cache_id, &snapshot,
            ))
        } else {
            let emb_files = cache.list_embeddings_async().await.ok()?;
            if emb_files.is_empty() {
                None
            } else {
                Some(Self::build_embedding_maps_from_json(cache_id, emb_files))
            }
        };

        let (photos, embeddings) = embedding_maps.unwrap_or_default();
        if photos.is_empty() && face_files.is_empty() {
            return None;
        }
        Some(Self::assemble_loaded_index(
            cache_id, generation, photos, embeddings, face_files,
        ))
        .filter(|loaded| !loaded.photos.is_empty())
    }

    fn merge_loaded_indexes(into: &mut LoadedOwnerIndex, album_index: LoadedOwnerIndex) {
        into.photos.extend(album_index.photos);
        into.embeddings.extend(album_index.embeddings);
        for (face_id, face) in album_index.faces {
            into.faces_by_stable_key
                .entry(face.stable_key.clone())
                .or_default()
                .push(face_id.clone());
            into.faces.insert(face_id, face);
        }
        into.face_embeddings.extend(album_index.face_embeddings);
    }

    /// Load specific album cache dirs for an owner (empty `cache_ids` → none loaded).
    pub(crate) async fn load_albums_index_async(
        &self,
        owner: &str,
        cache_ids: &[String],
        include_faces: bool,
    ) -> Option<LoadedOwnerIndex> {
        if cache_ids.is_empty() {
            return None;
        }
        let mut merged = LoadedOwnerIndex {
            generation: format!("albums:{}", cache_ids.len()),
            photos: HashMap::new(),
            embeddings: HashMap::new(),
            faces: HashMap::new(),
            faces_by_stable_key: HashMap::new(),
            face_embeddings: HashMap::new(),
        };
        let mut loaded_any = false;
        for cache_id in cache_ids {
            let Some(album_index) = self
                .load_album_index_from_file_cache_async(owner, cache_id, include_faces)
                .await
            else {
                continue;
            };
            loaded_any = true;
            Self::merge_loaded_indexes(&mut merged, album_index);
        }
        if !loaded_any || merged.photos.is_empty() {
            return None;
        }
        Some(merged)
    }

    fn build_index_from_cache_files(
        cache_id: &str,
        generation: String,
        emb_files: Vec<(String, super::album_cache::EmbeddingCacheFile)>,
        face_files: Vec<(String, super::album_cache::FacePhotoCacheFile)>,
    ) -> LoadedOwnerIndex {
        let (photos, embeddings) = Self::build_embedding_maps_from_json(cache_id, emb_files);
        Self::assemble_loaded_index(cache_id, generation, photos, embeddings, face_files)
    }

    fn build_embedding_maps_from_json(
        cache_id: &str,
        emb_files: Vec<(String, super::album_cache::EmbeddingCacheFile)>,
    ) -> (
        HashMap<String, AiIndexedPhotoRecord>,
        HashMap<String, Vec<f32>>,
    ) {
        let mut photos = HashMap::new();
        let mut embeddings = HashMap::new();
        for (_key, file) in emb_files {
            if photos.contains_key(&file.stable_key) {
                continue;
            }
            let Ok(vector) = file.decode_vector() else {
                continue;
            };
            Self::insert_embedding_record(
                cache_id,
                &file.stable_key,
                file.content_signature.as_deref().unwrap_or(""),
                file.provider_revision.clone(),
                file.width,
                file.height,
                vector,
                &mut photos,
                &mut embeddings,
            );
        }
        (photos, embeddings)
    }

    fn build_embedding_maps_from_snapshot(
        cache_id: &str,
        snapshot: &super::embedding_snapshot::EmbeddingSnapshotLoaded,
    ) -> (
        HashMap<String, AiIndexedPhotoRecord>,
        HashMap<String, Vec<f32>>,
    ) {
        let mut photos = HashMap::new();
        let mut embeddings = HashMap::new();
        for (entry, vector) in &snapshot.items {
            if photos.contains_key(&entry.stable_key) {
                continue;
            }
            Self::insert_embedding_record(
                cache_id,
                entry.stable_key.as_str(),
                entry.content_signature.as_deref().unwrap_or(""),
                entry.provider_revision.clone(),
                entry.width,
                entry.height,
                vector.clone(),
                &mut photos,
                &mut embeddings,
            );
        }
        (photos, embeddings)
    }

    fn insert_embedding_record(
        cache_id: &str,
        stable_key: &str,
        content_signature: &str,
        provider_revision: Option<String>,
        width: Option<u32>,
        height: Option<u32>,
        vector: Vec<f32>,
        photos: &mut HashMap<String, AiIndexedPhotoRecord>,
        embeddings: &mut HashMap<String, Vec<f32>>,
    ) {
        let Some(mut record) = photo_record_from_stable_key(
            stable_key,
            cache_id,
            content_signature,
            provider_revision,
        ) else {
            return;
        };
        record.width = width;
        record.height = height;
        record.image_embedding_state = "ready".to_string();
        record.photo_id = photos.len();
        let embedding_key = record.embedding_key();
        embeddings.insert(embedding_key, vector);
        photos.insert(record.stable_key.clone(), record);
    }

    fn assemble_loaded_index(
        cache_id: &str,
        generation: String,
        mut photos: HashMap<String, AiIndexedPhotoRecord>,
        embeddings: HashMap<String, Vec<f32>>,
        face_files: Vec<(String, super::album_cache::FacePhotoCacheFile)>,
    ) -> LoadedOwnerIndex {
        let mut faces_by_stable_key = HashMap::<String, Vec<String>>::new();
        let mut face_embeddings = HashMap::new();
        let mut faces = HashMap::new();
        let mut loaded_face_photos = HashSet::new();
        for (_key, file) in face_files {
            if !loaded_face_photos.insert(file.stable_key.clone()) {
                continue;
            }
            if !photos.contains_key(&file.stable_key) {
                if let Some(mut record) = photo_record_from_stable_key(
                    &file.stable_key,
                    cache_id,
                    file.content_signature.as_deref().unwrap_or(""),
                    None,
                ) {
                    record.width = file.width;
                    record.height = file.height;
                    record.face_embedding_state = "ready".to_string();
                    record.image_embedding_state = "skipped".to_string();
                    record.photo_id = photos.len();
                    photos.insert(record.stable_key.clone(), record);
                }
            }
            let photo_id = photos
                .get(&file.stable_key)
                .map(|p| p.photo_id)
                .unwrap_or(0);
            if let Some(photo) = photos.get_mut(&file.stable_key) {
                if photo.width.is_none() {
                    photo.width = file.width;
                }
                if photo.height.is_none() {
                    photo.height = file.height;
                }
                photo.face_embedding_state = "ready".to_string();
            }
            for entry in file.faces {
                let Ok(vector) = entry.decode_embedding() else {
                    continue;
                };
                let face = AiFaceRecord {
                    face_id: entry.face_id.clone(),
                    photo_id,
                    stable_key: file.stable_key.clone(),
                    box_xywh: entry.box_xywh,
                    landmarks: entry.landmarks,
                    detection_score: entry.detection_score,
                    quality_score: entry.quality_score,
                    detector_revision: file.detector_revision.clone().unwrap_or_default(),
                    embedding_state: "ready".to_string(),
                };
                faces_by_stable_key
                    .entry(file.stable_key.clone())
                    .or_default()
                    .push(entry.face_id.clone());
                face_embeddings.insert(entry.face_id.clone(), vector);
                faces.insert(entry.face_id, face);
            }
        }

        LoadedOwnerIndex {
            generation,
            photos,
            embeddings,
            faces,
            faces_by_stable_key,
            face_embeddings,
        }
    }

    /// Async variant for HTTP / task paths — loads all album caches for an owner.
    pub(crate) async fn load_current_index_async(&self, owner: &str) -> Option<LoadedOwnerIndex> {
        let cache_ids = self.list_album_cache_ids_async(owner).await;
        self.load_albums_index_async(owner, &cache_ids, true).await
    }

    pub fn owner_status(&self, owner: &str) -> OwnerIndexStatus {
        let owner_root = self.owner_dir(owner);
        let cache_ids = self.list_album_cache_ids(owner);
        let current_generation = cache_ids
            .first()
            .and_then(|cache_id| self.current_album_generation(owner, cache_id));
        let mut manifest_present = false;
        let mut embeddings_present = false;
        let mut duplicates_index_present = false;
        let mut state = None;
        let mut created_at_unix_ms = None;
        let mut stats = None;
        for cache_id in &cache_ids {
            let album_root = self.album_dir(owner, cache_id);
            embeddings_present |= album_root.join("embeddings").is_dir()
                && std::fs::read_dir(album_root.join("embeddings"))
                    .map(|mut d| d.next().is_some())
                    .unwrap_or(false);
            duplicates_index_present |= album_root.join("duplicates_index.json").is_file();
            let Some(generation) = self.current_album_generation(owner, cache_id) else {
                continue;
            };
            let generation_dir = album_root.join("generations").join(generation);
            manifest_present |= generation_dir.join("manifest.json").exists();
            if let Some(manifest) = std::fs::read_to_string(generation_dir.join("manifest.json"))
                .ok()
                .and_then(|raw| serde_json::from_str::<StoredAiGenerationManifest>(&raw).ok())
            {
                if state.is_none() {
                    state = manifest.state.clone();
                }
                created_at_unix_ms = created_at_unix_ms.or(manifest.created_at_unix_ms);
                stats = stats.or(manifest.stats);
            }
        }
        OwnerIndexStatus {
            owner: owner.to_string(),
            root_exists: owner_root.exists(),
            current_generation,
            manifest_present,
            embeddings_present,
            duplicates_index_present,
            state,
            created_at_unix_ms,
            sources_total: Some(cache_ids.len()),
            stats,
        }
    }

    pub(crate) fn write_generation_manifest(
        &self,
        owner: &str,
        cache_id: &str,
        request: &NormalizedAiIndexRequest,
        capabilities: &AiCapabilities,
        provider_health: &AiProviderHealth,
        indexed_albums: &[AiIndexedAlbumSummary],
        indexed_photos: &[AiIndexedPhotoRecord],
        faces: &[AiFaceRecord],
        photos_total: usize,
    ) -> Result<String, String> {
        if cache_id.is_empty() {
            return Err("album cache_id is required for ai generation".to_string());
        }
        let album_root = self.ensure_album_layout(owner, cache_id)?;
        let generations_root = album_root.join("generations");
        std::fs::create_dir_all(&generations_root).map_err(|error| {
            io_error_with_path("create ai generations root", &generations_root, error)
        })?;

        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let generation_id = format!("gen-{timestamp_ms}-{}", Uuid::new_v4().simple());
        let generation_dir = generations_root.join(&generation_id);
        std::fs::create_dir_all(&generation_dir).map_err(|error| {
            io_error_with_path("create ai generation dir", &generation_dir, error)
        })?;

        let photos_reused = indexed_photos
            .iter()
            .filter(|photo| photo.scan_state == "reused")
            .count();
        let image_embeddings_pending = indexed_photos
            .iter()
            .filter(|photo| photo.image_embedding_state == "pending")
            .count();
        let image_embeddings_ready = indexed_photos
            .iter()
            .filter(|photo| photo.image_embedding_state == "ready")
            .count();
        let duplicate_hashes_pending = indexed_photos
            .iter()
            .filter(|photo| photo.duplicate_hash_state == "pending")
            .count();
        let face_embeddings_pending = indexed_photos
            .iter()
            .filter(|photo| photo.face_embedding_state == "pending")
            .count();
        let duplicate_groups = super::album_cache::AlbumAiCache::new(&self.root, owner, cache_id)
            .read_duplicates_index_json()
            .and_then(|value| {
                value
                    .get("groups")
                    .and_then(serde_json::Value::as_array)
                    .map(Vec::len)
            })
            .unwrap_or(0);
        let duplicates_requested = request
            .features
            .iter()
            .any(|feature| feature == "duplicates");
        let image_requested = request.features.iter().any(|feature| feature == "image");
        let faces_requested = request.features.iter().any(|feature| feature == "faces");
        let generation_state = if provider_health.healthy
            && (!image_requested || image_embeddings_pending == 0)
            && (!faces_requested || face_embeddings_pending == 0)
        {
            "ready"
        } else if duplicates_requested || image_embeddings_ready > 0 || !faces.is_empty() {
            "partial"
        } else {
            "unavailable"
        };

        let manifest = json!({
            "version": 2,
            "owner": owner,
            "cache_id": cache_id,
            "album": request.album,
            "mode": request.mode,
            "features": request.features,
            "created_at_unix_ms": timestamp_ms,
            "sources": indexed_albums,
            "files": {
                "embeddings_dir": "embeddings",
                "faces_dir": "faces",
                "phash_dir": "phash",
                "duplicates_index": "duplicates_index.json",
                "face_index": "face_index.json",
            },
            "provider": {
                "kind": provider_health.kind,
                "backend": provider_health.backend,
                "healthy": provider_health.healthy,
                "model_manifest_present": provider_health.model_manifest_present,
                "model_root": provider_health.model_root,
                "revision": provider_health.revision,
            },
            "capabilities": capabilities,
            "stats": {
                "albums_total": 1,
                "photos_total": photos_total,
                "photos_indexed": indexed_photos.len(),
                "photos_reused": photos_reused,
                "duplicates": duplicate_groups,
                "faces": faces.len(),
                "image_embeddings_pending": image_embeddings_pending,
                "duplicate_hashes_pending": duplicate_hashes_pending,
                "face_embeddings_pending": face_embeddings_pending,
            },
            "state": generation_state,
            "face_detector_revision": provider_health
                .revision
                .clone()
                .unwrap_or_else(|| FACE_DETECTOR_REVISION.to_string()),
            "image_embedding": {
                "dimension": capabilities.image_embedding_dimension,
            },
            "face_embedding": {
                "dimension": capabilities.face_embedding_dimension,
            },
            "note": "per-album generation keyed by AlbumMeta.cache; named people live in data/{owner}/albums/{album}/faces.json",
        });

        let manifest_path = generation_dir.join("manifest.json");
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("serialize ai manifest: {error}"))?;
        std::fs::write(&manifest_path, manifest_bytes)
            .map_err(|error| format!("write ai manifest: {error}"))?;

        let current_tmp = album_root.join("CURRENT.tmp");
        std::fs::write(&current_tmp, format!("{generation_id}\n"))
            .map_err(|error| format!("write ai current tmp: {error}"))?;
        std::fs::rename(&current_tmp, album_root.join("CURRENT"))
            .map_err(|error| format!("commit ai current generation: {error}"))?;

        Ok(generation_id)
    }
}
