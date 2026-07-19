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
    root: PathBuf,
}

impl FileAiIndexStore {
    pub fn new(root: PathBuf, legacy_roots: Vec<PathBuf>) -> Self {
        let _ = std::fs::create_dir_all(&root);
        Self::migrate_legacy_roots(&root, &legacy_roots);
        Self { root }
    }

    fn migrate_legacy_roots(root: &Path, legacy_roots: &[PathBuf]) {
        for legacy_root in legacy_roots {
            Self::migrate_legacy_root(root, legacy_root.as_path());
        }
    }

    fn migrate_legacy_root(root: &Path, legacy_root: &Path) {
        if !legacy_root.exists() || !legacy_root.is_dir() {
            return;
        }

        let Ok(entries) = std::fs::read_dir(legacy_root) else {
            return;
        };

        for entry in entries.flatten() {
            let source_path = entry.path();
            if !source_path.is_dir() {
                continue;
            }
            let owner_name = entry.file_name();
            let owner_root = root.join(&owner_name);
            let target_path = owner_root.join("ai");
            if target_path.exists() {
                continue;
            }
            let _ = std::fs::create_dir_all(&owner_root);
            let _ = std::fs::rename(&source_path, &target_path);
        }

        let _ = std::fs::remove_dir(legacy_root);
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn owner_dir(&self, owner: &str) -> PathBuf {
        self.root.join(owner).join("ai")
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
        self.current_generation(owner).is_some()
    }

    pub(crate) fn current_generation(&self, owner: &str) -> Option<String> {
        let current_path = self.owner_dir(owner).join("CURRENT");
        std::fs::read_to_string(&current_path)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
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
                embeddings.insert(record.stable_key.clone(), vector);
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

    pub(crate) fn load_current_index(&self, owner: &str) -> Option<LoadedOwnerIndex> {
        let current_generation = self.current_generation(owner)?;
        let generation_dir = self
            .owner_dir(owner)
            .join("generations")
            .join(&current_generation);
        let photos_path = generation_dir.join("photos.meta");
        let raw = std::fs::read_to_string(&photos_path).ok()?;
        let records = serde_json::from_str::<Vec<AiIndexedPhotoRecord>>(&raw).ok()?;
        let embeddings = Self::load_image_embeddings(&generation_dir, &records);
        let faces_list = Self::load_faces_meta(&generation_dir);
        let face_embeddings = Self::load_face_embeddings(&generation_dir, &faces_list);
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
        let people = Self::load_people_json(&generation_dir)
            .into_iter()
            .map(|person| (person.id.clone(), person))
            .collect::<HashMap<_, _>>();
        let assignments = Self::load_assignments_json(&generation_dir)
            .into_iter()
            .map(|assignment| (assignment.face_id.clone(), assignment))
            .collect::<HashMap<_, _>>();
        let photos = records
            .into_iter()
            .map(|record| (record.stable_key.clone(), record))
            .collect::<HashMap<_, _>>();
        Some(LoadedOwnerIndex {
            generation: current_generation,
            photos,
            embeddings,
            faces,
            faces_by_stable_key,
            face_embeddings,
            people,
            assignments,
        })
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
        let current_generation = self.current_generation(owner);
        let generation_dir = current_generation
            .as_ref()
            .map(|generation| owner_root.join("generations").join(generation));
        let manifest_path = generation_dir.as_ref().map(|dir| dir.join("manifest.json"));
        let photos_meta_path = generation_dir.as_ref().map(|dir| dir.join("photos.meta"));
        let hashes_bin_path = generation_dir.as_ref().map(|dir| dir.join("hashes.bin"));
        let vectors_path = generation_dir
            .as_ref()
            .map(|dir| dir.join("image-vectors.f16"));
        let manifest_present = manifest_path
            .as_ref()
            .map(|path| path.exists())
            .unwrap_or(false);
        let photos_meta_present = photos_meta_path
            .as_ref()
            .map(|path| path.exists())
            .unwrap_or(false);
        let hashes_bin_present = hashes_bin_path
            .as_ref()
            .map(|path| path.exists())
            .unwrap_or(false);
        let _vectors_present = vectors_path
            .as_ref()
            .map(|path| path.exists())
            .unwrap_or(false);
        let manifest = manifest_path
            .as_ref()
            .filter(|path| path.exists())
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|raw| serde_json::from_str::<StoredAiGenerationManifest>(&raw).ok());
        OwnerIndexStatus {
            owner: owner.to_string(),
            root_exists: owner_root.exists(),
            current_generation,
            manifest_present,
            photos_meta_present,
            hashes_bin_present,
            state: manifest.as_ref().and_then(|value| value.state.clone()),
            created_at_unix_ms: manifest.as_ref().and_then(|value| value.created_at_unix_ms),
            sources_total: manifest.as_ref().map(|value| value.sources.len()),
            stats: manifest.and_then(|value| value.stats),
        }
    }

    pub(crate) fn write_generation_image_vectors(
        &self,
        generation_dir: &Path,
        photos: &mut [AiIndexedPhotoRecord],
        embeddings: &[Option<Vec<f32>>],
    ) -> Result<Option<(usize, u64, usize)>, String> {
        if embeddings.len() != photos.len() {
            return Err("embedding count does not match photo count".to_string());
        }

        let ready_count = embeddings.iter().filter(|value| value.is_some()).count();
        if ready_count == 0 {
            for photo in photos.iter_mut() {
                photo.image_embedding_offset = None;
            }
            return Ok(None);
        }

        let dim = Self::infer_vector_dim(embeddings)
            .ok_or_else(|| "image embeddings missing dimension".to_string())?;
        let mut bytes = Vec::with_capacity(ready_count * dim * 2);
        for (photo, embedding) in photos.iter_mut().zip(embeddings.iter()) {
            photo.image_embedding_offset = None;
            let Some(vector) = embedding.as_ref() else {
                continue;
            };
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
        let ready_records = photos
            .iter()
            .filter(|photo| {
                photo.duplicate_hash_state == "ready"
                    && photo
                        .duplicate_sha256_hex
                        .as_ref()
                        .map(|value| !value.is_empty())
                        .unwrap_or(false)
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
            if photo.duplicate_hash_state != "ready" {
                continue;
            }
            let Some(sha256_hex) = photo.duplicate_sha256_hex.as_deref() else {
                continue;
            };
            let Some(sha256_bytes) = decode_sha256_hex(sha256_hex) else {
                photo.duplicate_hash_state = "error".to_string();
                photo
                    .record_error
                    .get_or_insert_with(|| "invalid duplicate sha256 hex".to_string());
                continue;
            };
            let offset = bytes.len() as u64;
            bytes.extend_from_slice(&sha256_bytes);
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
        request: &NormalizedAiIndexRequest,
        capabilities: &AiCapabilities,
        provider_health: &AiProviderHealth,
        indexed_albums: &[AiIndexedAlbumSummary],
        indexed_photos: &mut [AiIndexedPhotoRecord],
        embeddings: &[Option<Vec<f32>>],
        faces: &mut [AiFaceRecord],
        face_embeddings: &[Option<Vec<f32>>],
        people: &[PersonRecord],
        assignments: &[FaceAssignment],
        photos_total: usize,
    ) -> Result<String, String> {
        let owner_root = self.owner_dir(owner);
        let generations_root = owner_root.join("generations");
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
        self.write_generation_people(&generation_dir, people, assignments)?;
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
            "version": 1,
            "owner": owner,
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
                "people_json": {
                    "file": "people.json",
                    "records": people.len(),
                },
                "assignments_json": {
                    "file": "assignments.json",
                    "records": assignments.len(),
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
                "albums_total": indexed_albums.len(),
                "photos_total": photos_total,
                "photos_indexed": indexed_photos.len(),
                "photos_reused": photos_reused,
                "duplicates": duplicate_groups,
                "faces": faces.len(),
                "people": people.len(),
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
            "note": "generation stores reusable photo metadata, duplicate hashes, embeddings, and faces",
        });

        let manifest_path = generation_dir.join("manifest.json");
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)
            .map_err(|error| format!("serialize ai manifest: {error}"))?;
        std::fs::write(&manifest_path, manifest_bytes)
            .map_err(|error| format!("write ai manifest: {error}"))?;

        let current_tmp = owner_root.join("CURRENT.tmp");
        std::fs::write(&current_tmp, format!("{generation_id}\n"))
            .map_err(|error| format!("write ai current tmp: {error}"))?;
        std::fs::rename(&current_tmp, owner_root.join("CURRENT"))
            .map_err(|error| format!("commit ai current generation: {error}"))?;

        Ok(generation_id)
    }

    pub(crate) fn albums_from_photos(photos: &[AiIndexedPhotoRecord]) -> Vec<AiIndexedAlbumSummary> {
        let mut grouped = HashMap::<(String, String, String), usize>::new();
        for photo in photos {
            *grouped
                .entry((
                    photo.owner.clone(),
                    photo.album.clone(),
                    photo.source_key.clone(),
                ))
                .or_default() += 1;
        }
        let mut albums = grouped
            .into_iter()
            .map(|((owner, album, source_key), photo_count)| AiIndexedAlbumSummary {
                owner,
                album: album.clone(),
                source_key,
                photo_count,
            })
            .collect::<Vec<_>>();
        albums.sort_by(|left, right| left.source_key.cmp(&right.source_key));
        albums
    }

    pub(crate) fn load_generation_rewrite_snapshot(
        &self,
        owner: &str,
    ) -> Result<
        (
            NormalizedAiIndexRequest,
            Vec<AiIndexedAlbumSummary>,
            Vec<AiIndexedPhotoRecord>,
            Vec<Option<Vec<f32>>>,
            Vec<AiFaceRecord>,
            Vec<Option<Vec<f32>>>,
            Vec<PersonRecord>,
            Vec<FaceAssignment>,
            usize,
        ),
        String,
    > {
        let Some(loaded) = self.load_current_index(owner) else {
            return Err("ai index not found".to_string());
        };
        let generation_dir = self
            .owner_dir(owner)
            .join("generations")
            .join(&loaded.generation);
        let manifest = generation_dir
            .join("manifest.json")
            .exists()
            .then(|| std::fs::read_to_string(generation_dir.join("manifest.json")).ok())
            .flatten()
            .and_then(|raw| serde_json::from_str::<StoredAiGenerationManifest>(&raw).ok());

        let mut indexed_photos = loaded.photos.values().cloned().collect::<Vec<_>>();
        indexed_photos.sort_by(|left, right| left.stable_key.cmp(&right.stable_key));
        let embeddings = indexed_photos
            .iter()
            .map(|photo| loaded.embeddings.get(&photo.stable_key).cloned())
            .collect::<Vec<_>>();

        let mut faces = loaded.faces.values().cloned().collect::<Vec<_>>();
        faces.sort_by(|left, right| left.face_id.cmp(&right.face_id));
        let face_embeddings = faces
            .iter()
            .map(|face| loaded.face_embeddings.get(&face.face_id).cloned())
            .collect::<Vec<_>>();

        let mut people = loaded.people.values().cloned().collect::<Vec<_>>();
        people.sort_by(|left, right| left.id.cmp(&right.id));
        let mut assignments = loaded.assignments.values().cloned().collect::<Vec<_>>();
        assignments.sort_by(|left, right| left.face_id.cmp(&right.face_id));

        let request = NormalizedAiIndexRequest {
            album: manifest.as_ref().and_then(|value| value.album.clone()),
            mode: manifest
                .as_ref()
                .and_then(|value| value.mode.clone())
                .unwrap_or_else(|| "incremental".to_string()),
            features: manifest
                .as_ref()
                .map(|value| {
                    if value.features.is_empty() {
                        vec![
                            "image".to_string(),
                            "duplicates".to_string(),
                            "faces".to_string(),
                        ]
                    } else {
                        value.features.clone()
                    }
                })
                .unwrap_or_else(|| {
                    vec![
                        "image".to_string(),
                        "duplicates".to_string(),
                        "faces".to_string(),
                    ]
                }),
        };
        let indexed_albums = Self::albums_from_photos(&indexed_photos);
        let photos_total = manifest
            .as_ref()
            .and_then(|value| value.stats.as_ref())
            .map(|stats| stats.photos_total)
            .unwrap_or(indexed_photos.len());

        Ok((
            request,
            indexed_albums,
            indexed_photos,
            embeddings,
            faces,
            face_embeddings,
            people,
            assignments,
            photos_total,
        ))
    }

    pub fn commit_people_mutation<F>(
        &self,
        owner: &str,
        capabilities: &AiCapabilities,
        provider_health: &AiProviderHealth,
        mutator: F,
    ) -> Result<String, String>
    where
        F: FnOnce(&mut Vec<PersonRecord>, &mut Vec<FaceAssignment>) -> Result<(), String>,
    {
        let (
            request,
            indexed_albums,
            mut indexed_photos,
            embeddings,
            mut faces,
            face_embeddings,
            mut people,
            mut assignments,
            photos_total,
        ) = self.load_generation_rewrite_snapshot(owner)?;
        mutator(&mut people, &mut assignments)?;
        self.write_generation_manifest(
            owner,
            &request,
            capabilities,
            provider_health,
            &indexed_albums,
            &mut indexed_photos,
            &embeddings,
            &mut faces,
            &face_embeddings,
            &people,
            &assignments,
            photos_total,
        )
    }
}


