use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use super::duplicate::{
    build_exact_duplicate_groups, build_near_duplicate_groups, decode_sha256_hex,
};
use super::embedding::{decode_f16_vector_dim, encode_f16_vector_dim};
use super::faces::FACE_DETECTOR_REVISION;
use super::models::{
    AiFaceRecord, AiIndexedAlbumSummary, AiIndexedPhotoRecord, LoadedOwnerIndex,
    NormalizedAiIndexRequest, OwnerIndexStatus, StoredAiGenerationManifest,
};
use super::people::{FaceAssignment, PersonRecord};
use super::provider::{AiCapabilities, AiProviderHealth};

pub struct FileAiIndexStore {
    /// `dist/data/cache/ai/` — album embeddings under `{root}/{owner}/albums/{cache_id}/`,
    /// owner-level people under `{root}/{owner}/people/`.
    root: PathBuf,
}

impl FileAiIndexStore {
    pub fn new(root: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&root);
        Self { root }
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

    pub fn people_dir(&self, owner: &str) -> PathBuf {
        self.owner_dir(owner).join("people")
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
            .filter(|entry| entry.path().join("CURRENT").exists())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    fn read_current_generation(dir: &Path) -> Option<String> {
        std::fs::read_to_string(dir.join("CURRENT"))
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    pub(crate) fn current_album_generation(&self, owner: &str, cache_id: &str) -> Option<String> {
        Self::read_current_generation(&self.album_dir(owner, cache_id))
    }

    pub(crate) fn current_people_generation(&self, owner: &str) -> Option<String> {
        Self::read_current_generation(&self.people_dir(owner))
    }

    /// Backward-compatible alias used by older call sites; prefers any album CURRENT.
    pub(crate) fn current_generation(&self, owner: &str) -> Option<String> {
        self.list_album_cache_ids(owner).into_iter().next().and_then(|cache_id| {
            self.current_album_generation(owner, &cache_id)
        })
    }

    fn infer_vector_dim(embeddings: &[Option<Vec<f32>>]) -> Option<usize> {
        embeddings
            .iter()
            .find_map(|value| value.as_ref().map(|vector| vector.len()))
            .filter(|dim| *dim > 0)
    }

    fn read_vector_dim_from_manifest(generation_dir: &Path, key: &str) -> Option<usize> {
        let manifest_path = generation_dir.join("manifest.json");
        let raw = std::fs::read_to_string(manifest_path).ok()?;
        let value = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
        value
            .pointer(&format!("/files/{key}/dimension"))
            .and_then(|dim| dim.as_u64())
            .map(|dim| dim as usize)
            .filter(|dim| *dim > 0)
    }

    pub(crate) fn load_image_embeddings(
        generation_dir: &Path,
        records: &[AiIndexedPhotoRecord],
    ) -> HashMap<String, Vec<f32>> {
        let vectors_path = generation_dir.join("image-vectors.f16");
        let Ok(bytes) = std::fs::read(&vectors_path) else {
            return HashMap::new();
        };
        let dim = Self::read_vector_dim_from_manifest(generation_dir, "image_vectors")
            .or_else(|| {
                records.iter().find_map(|record| {
                    let offset = record.image_embedding_offset? as usize;
                    let next = records
                        .iter()
                        .filter_map(|other| other.image_embedding_offset)
                        .map(|value| value as usize)
                        .filter(|value| *value > offset)
                        .min();
                    next.map(|end| (end - offset) / 2)
                        .or_else(|| ((bytes.len() - offset) / 2).into())
                })
            })
            .unwrap_or(128);
        let mut embeddings = HashMap::new();
        for record in records {
            if record.image_embedding_state != "ready" {
                continue;
            }
            let Some(offset) = record.image_embedding_offset else {
                continue;
            };
            let start = offset as usize;
            let end = start.saturating_add(dim * 2);
            if end > bytes.len() {
                continue;
            }
            if let Ok(vector) = decode_f16_vector_dim(&bytes[start..end], dim) {
                embeddings.insert(record.embedding_key(), vector);
            }
        }
        embeddings
    }

    pub(crate) fn load_faces_meta(generation_dir: &Path) -> Vec<AiFaceRecord> {
        let faces_path = generation_dir.join("faces.meta");
        std::fs::read_to_string(&faces_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<AiFaceRecord>>(&raw).ok())
            .unwrap_or_default()
    }

    pub(crate) fn load_face_embeddings(
        generation_dir: &Path,
        faces: &[AiFaceRecord],
    ) -> HashMap<String, Vec<f32>> {
        let vectors_path = generation_dir.join("face-vectors.f16");
        let Ok(bytes) = std::fs::read(&vectors_path) else {
            return HashMap::new();
        };
        let dim = Self::read_vector_dim_from_manifest(generation_dir, "face_vectors")
            .or_else(|| {
                faces.iter().find_map(|face| {
                    let offset = face.embedding_offset? as usize;
                    let next = faces
                        .iter()
                        .filter_map(|other| other.embedding_offset)
                        .map(|value| value as usize)
                        .filter(|value| *value > offset)
                        .min();
                    next.map(|end| (end - offset) / 2)
                        .or_else(|| ((bytes.len().saturating_sub(offset)) / 2).into())
                })
            })
            .unwrap_or(128);
        let mut embeddings = HashMap::new();
        for face in faces {
            if face.embedding_state != "ready" {
                continue;
            }
            let Some(offset) = face.embedding_offset else {
                continue;
            };
            let start = offset as usize;
            let end = start.saturating_add(dim * 2);
            if end > bytes.len() {
                continue;
            }
            if let Ok(vector) = decode_f16_vector_dim(&bytes[start..end], dim) {
                embeddings.insert(face.face_id.clone(), vector);
            }
        }
        embeddings
    }

    pub(crate) fn load_people_json(generation_dir: &Path) -> Vec<PersonRecord> {
        let people_path = generation_dir.join("people.json");
        std::fs::read_to_string(&people_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<PersonRecord>>(&raw).ok())
            .unwrap_or_default()
    }

    pub(crate) fn load_assignments_json(generation_dir: &Path) -> Vec<FaceAssignment> {
        let assignments_path = generation_dir.join("assignments.json");
        std::fs::read_to_string(&assignments_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Vec<FaceAssignment>>(&raw).ok())
            .unwrap_or_default()
    }

    pub(crate) fn load_album_index(&self, owner: &str, cache_id: &str) -> Option<LoadedOwnerIndex> {
        let current_generation = self.current_album_generation(owner, cache_id)?;
        let generation_dir = self
            .album_dir(owner, cache_id)
            .join("generations")
            .join(&current_generation);
        self.load_index_from_generation_dir(&current_generation, &generation_dir, false)
    }

    fn load_index_from_generation_dir(
        &self,
        generation: &str,
        generation_dir: &Path,
        include_people: bool,
    ) -> Option<LoadedOwnerIndex> {
        let photos_path = generation_dir.join("photos.meta");
        let raw = std::fs::read_to_string(&photos_path).ok()?;
        let records = serde_json::from_str::<Vec<AiIndexedPhotoRecord>>(&raw).ok()?;
        let embeddings = Self::load_image_embeddings(generation_dir, &records);
        let faces_list = Self::load_faces_meta(generation_dir);
        let face_embeddings = Self::load_face_embeddings(generation_dir, &faces_list);
        let mut faces_by_stable_key = HashMap::<String, Vec<String>>::new();
        let faces = faces_list
            .into_iter()
            .map(|face| {
                faces_by_stable_key
                    .entry(face.stable_key.clone())
                    .or_default()
                    .push(face.face_id.clone());
                (face.face_id.clone(), face)
            })
            .collect::<HashMap<_, _>>();
        let (people, assignments) = if include_people {
            (
                Self::load_people_json(generation_dir)
                    .into_iter()
                    .map(|person| (person.id.clone(), person))
                    .collect::<HashMap<_, _>>(),
                Self::load_assignments_json(generation_dir)
                    .into_iter()
                    .map(|assignment| (assignment.face_id.clone(), assignment))
                    .collect::<HashMap<_, _>>(),
            )
        } else {
            (HashMap::new(), HashMap::new())
        };
        let photos = records
            .into_iter()
            .map(|record| (record.stable_key.clone(), record))
            .collect::<HashMap<_, _>>();
        Some(LoadedOwnerIndex {
            generation: generation.to_string(),
            photos,
            embeddings,
            faces,
            faces_by_stable_key,
            face_embeddings,
            people,
            assignments,
        })
    }

    fn load_people_index(&self, owner: &str) -> (HashMap<String, PersonRecord>, HashMap<String, FaceAssignment>) {
        let Some(generation) = self.current_people_generation(owner) else {
            return (HashMap::new(), HashMap::new());
        };
        let generation_dir = self.people_dir(owner).join("generations").join(generation);
        (
            Self::load_people_json(&generation_dir)
                .into_iter()
                .map(|person| (person.id.clone(), person))
                .collect(),
            Self::load_assignments_json(&generation_dir)
                .into_iter()
                .map(|assignment| (assignment.face_id.clone(), assignment))
                .collect(),
        )
    }

    /// Merge all per-album indexes for `owner` plus owner-level people assignments.
    pub(crate) fn load_current_index(&self, owner: &str) -> Option<LoadedOwnerIndex> {
        let cache_ids = self.list_album_cache_ids(owner);
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
            people: HashMap::new(),
            assignments: HashMap::new(),
        };
        for cache_id in &cache_ids {
            let Some(album_index) = self.load_album_index(owner, cache_id) else {
                continue;
            };
            merged.photos.extend(album_index.photos);
            merged.embeddings.extend(album_index.embeddings);
            for (face_id, face) in album_index.faces {
                merged
                    .faces_by_stable_key
                    .entry(face.stable_key.clone())
                    .or_default()
                    .push(face_id.clone());
                merged.faces.insert(face_id, face);
            }
            merged.face_embeddings.extend(album_index.face_embeddings);
        }
        let (people, assignments) = self.load_people_index(owner);
        merged.people = people;
        merged.assignments = assignments;
        if merged.photos.is_empty() && merged.people.is_empty() {
            return None;
        }
        Some(merged)
    }

    pub(crate) fn load_current_photos(
        &self,
        owner: &str,
    ) -> Option<(String, HashMap<String, AiIndexedPhotoRecord>)> {
        let loaded = self.load_current_index(owner)?;
        Some((loaded.generation, loaded.photos))
    }

    pub fn owner_status(&self, owner: &str) -> OwnerIndexStatus {
        let owner_root = self.owner_dir(owner);
        let cache_ids = self.list_album_cache_ids(owner);
        let current_generation = cache_ids
            .first()
            .and_then(|cache_id| self.current_album_generation(owner, cache_id));
        let mut manifest_present = false;
        let mut photos_meta_present = false;
        let mut hashes_bin_present = false;
        let mut state = None;
        let mut created_at_unix_ms = None;
        let mut stats = None;
        for cache_id in &cache_ids {
            let Some(generation) = self.current_album_generation(owner, cache_id) else {
                continue;
            };
            let generation_dir = self
                .album_dir(owner, cache_id)
                .join("generations")
                .join(generation);
            manifest_present |= generation_dir.join("manifest.json").exists();
            photos_meta_present |= generation_dir.join("photos.meta").exists();
            hashes_bin_present |= generation_dir.join("hashes.bin").exists();
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
            photos_meta_present,
            hashes_bin_present,
            state,
            created_at_unix_ms,
            sources_total: Some(cache_ids.len()),
            stats,
        }
    }

    pub(crate) fn write_generation_image_vectors(
        &self,
        generation_dir: &Path,
        photos: &mut [AiIndexedPhotoRecord],
        embeddings: &HashMap<String, Vec<f32>>,
    ) -> Result<Option<(usize, u64, usize)>, String> {
        let ready_count = photos
            .iter()
            .filter(|photo| embeddings.contains_key(&photo.embedding_key()))
            .count();
        if ready_count == 0 {
            for photo in photos.iter_mut() {
                photo.image_embedding_offset = None;
            }
            return Ok(None);
        }

        let dim = embeddings
            .values()
            .next()
            .map(|vector| vector.len())
            .ok_or_else(|| "image embeddings missing dimension".to_string())?;
        let mut bytes = Vec::with_capacity(ready_count * dim * 2);
        for photo in photos.iter_mut() {
            photo.image_embedding_offset = None;
            let Some(vector) = embeddings.get(&photo.embedding_key()) else {
                continue;
            };
            if vector.len() != dim {
                return Err(format!(
                    "image embedding dimension mismatch: got {}, expected {dim}",
                    vector.len()
                ));
            }
            let encoded = encode_f16_vector_dim(vector, dim)?;
            let offset = bytes.len() as u64;
            bytes.extend_from_slice(&encoded);
            photo.image_embedding_offset = Some(offset);
            photo.image_embedding_state = "ready".to_string();
        }

        let vectors_path = generation_dir.join("image-vectors.f16");
        std::fs::write(&vectors_path, &bytes)
            .map_err(|error| format!("write ai image vectors: {error}"))?;
        Ok(Some((ready_count, bytes.len() as u64, dim)))
    }

    pub(crate) fn write_generation_face_vectors(
        &self,
        generation_dir: &Path,
        faces: &mut [AiFaceRecord],
        embeddings: &[Option<Vec<f32>>],
    ) -> Result<Option<(usize, u64, usize)>, String> {
        if embeddings.len() != faces.len() {
            return Err("face embedding count does not match face count".to_string());
        }

        let ready_count = embeddings.iter().filter(|value| value.is_some()).count();
        if ready_count == 0 {
            for face in faces.iter_mut() {
                face.embedding_offset = None;
            }
            return Ok(None);
        }

        let dim = Self::infer_vector_dim(embeddings)
            .ok_or_else(|| "face embeddings missing dimension".to_string())?;
        let mut bytes = Vec::with_capacity(ready_count * dim * 2);
        for (face, embedding) in faces.iter_mut().zip(embeddings.iter()) {
            face.embedding_offset = None;
            let Some(vector) = embedding.as_ref() else {
                continue;
            };
            let encoded = encode_f16_vector_dim(vector, dim)?;
            let offset = bytes.len() as u64;
            bytes.extend_from_slice(&encoded);
            face.embedding_offset = Some(offset);
            face.embedding_state = "ready".to_string();
        }

        let vectors_path = generation_dir.join("face-vectors.f16");
        std::fs::write(&vectors_path, &bytes)
            .map_err(|error| format!("write ai face vectors: {error}"))?;
        Ok(Some((ready_count, bytes.len() as u64, dim)))
    }

    pub(crate) fn write_generation_faces_meta(
        &self,
        generation_dir: &Path,
        faces: &[AiFaceRecord],
    ) -> Result<(), String> {
        let faces_path = generation_dir.join("faces.meta");
        let face_bytes = serde_json::to_vec_pretty(faces)
            .map_err(|error| format!("serialize ai faces meta: {error}"))?;
        std::fs::write(&faces_path, face_bytes)
            .map_err(|error| format!("write ai faces meta: {error}"))
    }

    pub(crate) fn write_generation_people(
        &self,
        generation_dir: &Path,
        people: &[PersonRecord],
        assignments: &[FaceAssignment],
    ) -> Result<(), String> {
        let people_bytes = serde_json::to_vec_pretty(people)
            .map_err(|error| format!("serialize ai people json: {error}"))?;
        std::fs::write(generation_dir.join("people.json"), people_bytes)
            .map_err(|error| format!("write ai people json: {error}"))?;
        let assignments_bytes = serde_json::to_vec_pretty(assignments)
            .map_err(|error| format!("serialize ai assignments json: {error}"))?;
        std::fs::write(generation_dir.join("assignments.json"), assignments_bytes)
            .map_err(|error| format!("write ai assignments json: {error}"))
    }

    pub(crate) fn write_generation_hashes(
        &self,
        generation_dir: &Path,
        photos: &mut [AiIndexedPhotoRecord],
    ) -> Result<Option<(usize, u64)>, String> {
        // Layout kept as 32-byte SHA256 slot + 8-byte pHash for compatibility.
        // Exact SHA256 is currently disabled; the first 32 bytes are zeros.
        let ready_records = photos
            .iter()
            .filter(|photo| {
                photo.duplicate_hash_state == "ready"
                    && photo.duplicate_phash_valid
                    && photo.duplicate_phash_u64.is_some()
            })
            .count();
        if ready_records == 0 {
            return Ok(None);
        }

        let hashes_path = generation_dir.join("hashes.bin");
        let mut bytes = Vec::with_capacity(ready_records * 40);
        for photo in photos.iter_mut() {
            photo.duplicate_sha256_offset = None;
            photo.duplicate_phash_offset = None;
            if photo.duplicate_hash_state != "ready"
                || !photo.duplicate_phash_valid
                || photo.duplicate_phash_u64.is_none()
            {
                continue;
            }
            let offset = bytes.len() as u64;
            if let Some(sha256_hex) = photo.duplicate_sha256_hex.as_deref()
                && let Some(sha256_bytes) = decode_sha256_hex(sha256_hex)
            {
                bytes.extend_from_slice(&sha256_bytes);
            } else {
                photo.duplicate_sha256_hex = None;
                bytes.extend_from_slice(&[0_u8; 32]);
            }
            let phash = photo.duplicate_phash_u64.unwrap_or(0_u64);
            bytes.extend_from_slice(&phash.to_le_bytes());
            photo.duplicate_sha256_offset = Some(offset);
            photo.duplicate_phash_offset = Some(offset + 32);
        }

        std::fs::write(&hashes_path, &bytes)
            .map_err(|error| format!("write ai hashes bin: {error}"))?;
        Ok(Some((ready_records, bytes.len() as u64)))
    }

    pub(crate) fn write_generation_photos(
        &self,
        generation_dir: &Path,
        photos: &[AiIndexedPhotoRecord],
    ) -> Result<(), String> {
        let photos_path = generation_dir.join("photos.meta");
        let photo_bytes = serde_json::to_vec_pretty(photos)
            .map_err(|error| format!("serialize ai photos meta: {error}"))?;
        std::fs::write(&photos_path, photo_bytes)
            .map_err(|error| format!("write ai photos meta: {error}"))
    }

    pub(crate) fn write_generation_manifest(
        &self,
        owner: &str,
        cache_id: &str,
        request: &NormalizedAiIndexRequest,
        capabilities: &AiCapabilities,
        provider_health: &AiProviderHealth,
        indexed_albums: &[AiIndexedAlbumSummary],
        indexed_photos: &mut [AiIndexedPhotoRecord],
        embeddings: &HashMap<String, Vec<f32>>,
        faces: &mut [AiFaceRecord],
        face_embeddings: &[Option<Vec<f32>>],
        photos_total: usize,
    ) -> Result<String, String> {
        if cache_id.is_empty() {
            return Err("album cache_id is required for ai generation".to_string());
        }
        let album_root = self.album_dir(owner, cache_id);
        let generations_root = album_root.join("generations");
        std::fs::create_dir_all(&generations_root)
            .map_err(|error| format!("create ai generations root: {error}"))?;

        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let generation_id = format!("gen-{timestamp_ms}-{}", Uuid::new_v4().simple());
        let generation_dir = generations_root.join(&generation_id);
        std::fs::create_dir_all(&generation_dir)
            .map_err(|error| format!("create ai generation dir: {error}"))?;

        let hashes_file = self.write_generation_hashes(&generation_dir, indexed_photos)?;
        let vectors_file =
            self.write_generation_image_vectors(&generation_dir, indexed_photos, embeddings)?;
        let face_vectors_file =
            self.write_generation_face_vectors(&generation_dir, faces, face_embeddings)?;
        self.write_generation_faces_meta(&generation_dir, faces)?;
        self.write_generation_photos(&generation_dir, indexed_photos)?;

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
        let duplicate_groups = build_exact_duplicate_groups(indexed_photos).len()
            + build_near_duplicate_groups(indexed_photos, 8).len();
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
                "photos_meta": {
                    "file": "photos.meta",
                    "records": indexed_photos.len(),
                },
                "hashes_bin": hashes_file.map(|(records, bytes)| json!({
                    "file": "hashes.bin",
                    "records": records,
                    "bytes": bytes,
                    "entry_size": 40,
                })),
                "image_vectors": vectors_file.map(|(records, bytes, dim)| json!({
                    "file": "image-vectors.f16",
                    "records": records,
                    "bytes": bytes,
                    "dimension": dim,
                    "dtype": "f16",
                })),
                "face_vectors": face_vectors_file.map(|(records, bytes, dim)| json!({
                    "file": "face-vectors.f16",
                    "records": records,
                    "bytes": bytes,
                    "dimension": dim,
                    "dtype": "f16",
                })),
                "faces_meta": {
                    "file": "faces.meta",
                    "records": faces.len(),
                },
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
                "people": 0,
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
                "dimension": vectors_file.map(|(_, _, dim)| dim),
            },
            "face_embedding": {
                "dimension": face_vectors_file.map(|(_, _, dim)| dim),
            },
            "note": "per-album generation keyed by AlbumMeta.cache; people live under cache/ai/{owner}/people",
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

    pub(crate) fn write_people_generation(
        &self,
        owner: &str,
        people: &[PersonRecord],
        assignments: &[FaceAssignment],
    ) -> Result<String, String> {
        let people_root = self.people_dir(owner);
        let generations_root = people_root.join("generations");
        std::fs::create_dir_all(&generations_root)
            .map_err(|error| format!("create ai people generations root: {error}"))?;
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let generation_id = format!("gen-{timestamp_ms}-{}", Uuid::new_v4().simple());
        let generation_dir = generations_root.join(&generation_id);
        std::fs::create_dir_all(&generation_dir)
            .map_err(|error| format!("create ai people generation dir: {error}"))?;
        self.write_generation_people(&generation_dir, people, assignments)?;
        let manifest = json!({
            "version": 1,
            "owner": owner,
            "created_at_unix_ms": timestamp_ms,
            "files": {
                "people_json": { "file": "people.json", "records": people.len() },
                "assignments_json": { "file": "assignments.json", "records": assignments.len() },
            },
            "stats": {
                "people": people.len(),
                "assignments": assignments.len(),
            },
            "state": "ready",
        });
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("serialize ai people manifest: {error}"))?;
        std::fs::write(generation_dir.join("manifest.json"), manifest_bytes)
            .map_err(|error| format!("write ai people manifest: {error}"))?;
        let current_tmp = people_root.join("CURRENT.tmp");
        std::fs::write(&current_tmp, format!("{generation_id}\n"))
            .map_err(|error| format!("write ai people current tmp: {error}"))?;
        std::fs::rename(&current_tmp, people_root.join("CURRENT"))
            .map_err(|error| format!("commit ai people current: {error}"))?;
        Ok(generation_id)
    }

    pub(crate) fn albums_from_photos(photos: &[AiIndexedPhotoRecord]) -> Vec<AiIndexedAlbumSummary> {
        let mut grouped = HashMap::<(String, String, String, String), usize>::new();
        for photo in photos {
            *grouped
                .entry((
                    photo.owner.clone(),
                    photo.album.clone(),
                    photo.source_key.clone(),
                    photo.cache_id.clone(),
                ))
                .or_default() += 1;
        }
        let mut albums = grouped
            .into_iter()
            .map(
                |((owner, album, source_key, cache_id), photo_count)| AiIndexedAlbumSummary {
                    owner,
                    album: album.clone(),
                    source_key,
                    cache_id,
                    photo_count,
                },
            )
            .collect::<Vec<_>>();
        albums.sort_by(|left, right| {
            left.cache_id
                .cmp(&right.cache_id)
                .then(left.source_key.cmp(&right.source_key))
        });
        albums
    }

    pub fn commit_people_mutation<F>(
        &self,
        owner: &str,
        _capabilities: &AiCapabilities,
        _provider_health: &AiProviderHealth,
        mutator: F,
    ) -> Result<String, String>
    where
        F: FnOnce(&mut Vec<PersonRecord>, &mut Vec<FaceAssignment>) -> Result<(), String>,
    {
        let (people_map, assignments_map) = self.load_people_index(owner);
        let mut people = people_map.into_values().collect::<Vec<_>>();
        let mut assignments = assignments_map.into_values().collect::<Vec<_>>();
        people.sort_by(|left, right| left.id.cmp(&right.id));
        assignments.sort_by(|left, right| left.face_id.cmp(&right.face_id));
        mutator(&mut people, &mut assignments)?;
        self.write_people_generation(owner, &people, &assignments)
    }
}
