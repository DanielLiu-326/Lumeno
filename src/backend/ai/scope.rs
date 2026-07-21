use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet};

use super::faces::normalize_box_xywh;
use super::people::PersonRecord;
use super::{AiFaceRecord, AiIndexedPhotoRecord, LoadedOwnerIndex};

fn api_face_box(face: &AiFaceRecord, photo: &AiIndexedPhotoRecord) -> [f32; 4] {
    normalize_box_xywh(
        face.box_xywh,
        photo.width.unwrap_or(1),
        photo.height.unwrap_or(1),
    )
}

/// Keep only photos the caller may see for `(owner, album)` pairs.
pub(crate) fn photo_in_allowed_scope(
    record: &AiIndexedPhotoRecord,
    albums_by_owner: &HashMap<String, BTreeSet<String>>,
) -> bool {
    albums_by_owner
        .get(record.owner.as_str())
        .is_some_and(|albums| albums.contains(record.album.as_str()))
}

pub(crate) fn filter_photos_for_scope<I>(
    records: I,
    albums_by_owner: &HashMap<String, BTreeSet<String>>,
) -> Vec<AiIndexedPhotoRecord>
where
    I: IntoIterator<Item = AiIndexedPhotoRecord>,
{
    records
        .into_iter()
        .filter(|record| photo_in_allowed_scope(record, albums_by_owner))
        .collect()
}

pub(crate) fn accessible_albums_by_owner(
    targets: &[super::super::album::AiAlbumScanTarget],
) -> HashMap<String, BTreeSet<String>> {
    let mut albums_by_owner = HashMap::<String, BTreeSet<String>>::new();
    for target in targets {
        albums_by_owner
            .entry(target.owner.clone())
            .or_default()
            .insert(target.album_name.clone());
    }
    albums_by_owner
}

pub(crate) fn visible_face_photo<'a>(
    face: &'a AiFaceRecord,
    photos: &'a HashMap<String, AiIndexedPhotoRecord>,
    allowed_albums: &BTreeSet<String>,
) -> Option<&'a AiIndexedPhotoRecord> {
    let photo = photos.get(&face.stable_key)?;
    allowed_albums.contains(&photo.album).then_some(photo)
}

pub(crate) fn build_person_cover(
    person: &PersonRecord,
    index: &LoadedOwnerIndex,
    allowed_albums: &BTreeSet<String>,
) -> Option<Value> {
    let preferred_face_ids = [
        person.cover_face_id.clone(),
        index
            .assignments
            .values()
            .find(|assignment| assignment.person_id == person.id)
            .map(|assignment| assignment.face_id.clone()),
    ];
    for face_id in preferred_face_ids.into_iter().flatten() {
        if let Some(face) = index.faces.get(face_id.as_str())
            && let Some(photo) = visible_face_photo(face, &index.photos, allowed_albums)
        {
            return Some(json!({
                "owner": photo.owner,
                "album": photo.album,
                "path": photo.path,
                "face_box": api_face_box(face, photo),
            }));
        }
    }
    // Shared viewers may lack the preferred cover face; fall back to any visible face.
    for assignment in index.assignments.values() {
        if assignment.person_id != person.id {
            continue;
        }
        let Some(face) = index.faces.get(&assignment.face_id) else {
            continue;
        };
        let Some(photo) = visible_face_photo(face, &index.photos, allowed_albums) else {
            continue;
        };
        return Some(json!({
            "owner": photo.owner,
            "album": photo.album,
            "path": photo.path,
            "face_box": api_face_box(face, photo),
        }));
    }
    None
}

pub(crate) fn count_visible_person_photos(
    person_id: &str,
    index: &LoadedOwnerIndex,
    allowed_albums: &BTreeSet<String>,
) -> usize {
    let mut photo_keys = HashSet::<String>::new();
    for assignment in index.assignments.values() {
        if assignment.person_id != person_id {
            continue;
        }
        let Some(face) = index.faces.get(&assignment.face_id) else {
            continue;
        };
        if visible_face_photo(face, &index.photos, allowed_albums).is_some() {
            photo_keys.insert(face.stable_key.clone());
        }
    }
    photo_keys.len()
}

/// Paths of photos in `album` that contain a face assigned to `person_id`.
pub(crate) fn collect_person_photo_paths_in_album(
    person_id: &str,
    index: &LoadedOwnerIndex,
    album: &str,
) -> HashSet<String> {
    let mut allowed = BTreeSet::new();
    allowed.insert(album.to_string());
    let mut paths = HashSet::new();
    for assignment in index.assignments.values() {
        if assignment.person_id != person_id {
            continue;
        }
        let Some(face) = index.faces.get(&assignment.face_id) else {
            continue;
        };
        let Some(photo) = visible_face_photo(face, &index.photos, &allowed) else {
            continue;
        };
        paths.insert(photo.path.clone());
    }
    paths
}

pub(crate) fn collect_visible_person_photos(
    person_id: &str,
    index: &LoadedOwnerIndex,
    allowed_albums: &BTreeSet<String>,
) -> Vec<Value> {
    let mut photos = HashMap::<String, Value>::new();
    for assignment in index.assignments.values() {
        if assignment.person_id != person_id {
            continue;
        }
        let Some(face) = index.faces.get(&assignment.face_id) else {
            continue;
        };
        let Some(photo) = visible_face_photo(face, &index.photos, allowed_albums) else {
            continue;
        };
        photos.entry(photo.stable_key.clone()).or_insert_with(|| {
            json!({
                "owner": photo.owner,
                "album": photo.album,
                "path": photo.path,
                "face_ids": Vec::<String>::new(),
                "face_boxes": Vec::<[f32; 4]>::new(),
            })
        });
        if let Some(entry) = photos.get_mut(&photo.stable_key) {
            if let Some(face_ids) = entry.get_mut("face_ids").and_then(|value| value.as_array_mut())
            {
                face_ids.push(json!(face.face_id));
            }
            if let Some(face_boxes) = entry
                .get_mut("face_boxes")
                .and_then(|value| value.as_array_mut())
            {
                face_boxes.push(json!(api_face_box(face, photo)));
            }
        }
    }
    let mut items = photos.into_values().collect::<Vec<_>>();
    items.sort_by(|left, right| {
        left.get("path")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .cmp(
                right
                    .get("path")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default(),
            )
    });
    items
}

/// Person edit APIs always mutate the caller's own index, never a shared owner's.
pub(crate) fn person_mutation_owner(session_username: &str) -> &str {
    session_username
}

pub(crate) fn person_is_editable(person_owner: &str, session_username: &str) -> bool {
    person_owner == session_username
}

/// Reject path traversal and absolute paths used as AI photo identifiers.
pub(crate) fn normalize_ai_photo_path(path: &str) -> Result<String, String> {
    let path = path.trim().replace('\\', "/");
    if path.is_empty() {
        return Err("path is required".to_string());
    }
    if path.starts_with('/') || path.contains(':') {
        return Err("absolute image paths are not allowed".to_string());
    }
    if path
        .split('/')
        .any(|segment| segment == ".." || segment == ".")
    {
        return Err("image path must not contain '.' or '..' segments".to_string());
    }
    Ok(path)
}

pub(crate) const MAX_AI_TEXT_QUERY_CHARS: usize = 512;

pub(crate) fn normalize_ai_text_query(text: &str) -> Result<String, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("text is required".to_string());
    }
    if text.chars().count() > MAX_AI_TEXT_QUERY_CHARS {
        return Err(format!(
            "text query exceeds {MAX_AI_TEXT_QUERY_CHARS} characters"
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai::people::{AssignmentSource, FaceAssignment, PersonRecord};

    fn photo(owner: &str, album: &str, path: &str) -> AiIndexedPhotoRecord {
        let stable_key = format!("{owner}|{album}|{path}");
        AiIndexedPhotoRecord {
            photo_id: 0,
            owner: owner.to_string(),
            album: album.to_string(),
            source_key: album.to_string(),
            cache_id: album.to_string(),
            path: path.to_string(),
            stable_key: stable_key.clone(),
            stable_id: stable_key,
            size_bytes: 10,
            modified_unix_seconds: None,
            content_signature: String::new(),
            width: Some(1),
            height: Some(1),
            provider_revision: None,
            scan_state: "ready".to_string(),
            image_embedding_state: "ready".to_string(),
            image_embedding_offset: None,
            duplicate_hash_state: "ready".to_string(),
            duplicate_sha256_hex: None,
            duplicate_sha256_offset: None,
            duplicate_phash_u64: None,
            duplicate_phash_offset: None,
            duplicate_phash_valid: false,
            face_embedding_state: "ready".to_string(),
            reused_from_generation: None,
            record_error: None,
            indexed_at_unix_ms: 0,
        }
    }

    fn sample_index() -> LoadedOwnerIndex {
        let private = photo("owner", "private", "albums/private/a.jpg");
        let shared = photo("owner", "shared", "albums/shared/b.jpg");
        let face_private = AiFaceRecord {
            face_id: "face-private".to_string(),
            photo_id: 0,
            stable_key: private.stable_key.clone(),
            box_xywh: [0.1, 0.1, 0.2, 0.2],
            landmarks: [[0.0; 2]; 5],
            detection_score: 1.0,
            quality_score: 1.0,
            detector_revision: "test".to_string(),
            embedding_state: "ready".to_string(),
            embedding_offset: None,
        };
        let face_shared = AiFaceRecord {
            face_id: "face-shared".to_string(),
            photo_id: 1,
            stable_key: shared.stable_key.clone(),
            box_xywh: [0.2, 0.2, 0.2, 0.2],
            landmarks: [[0.0; 2]; 5],
            detection_score: 1.0,
            quality_score: 1.0,
            detector_revision: "test".to_string(),
            embedding_state: "ready".to_string(),
            embedding_offset: None,
        };
        let person = PersonRecord {
            id: "person-1".to_string(),
            name: "Ada".to_string(),
            hidden: false,
            cover_face_id: Some("face-private".to_string()),
        };
        LoadedOwnerIndex {
            generation: "gen-1".to_string(),
            photos: HashMap::from([
                (private.stable_key.clone(), private),
                (shared.stable_key.clone(), shared),
            ]),
            embeddings: HashMap::new(),
            faces: HashMap::from([
                (face_private.face_id.clone(), face_private),
                (face_shared.face_id.clone(), face_shared),
            ]),
            face_embeddings: HashMap::new(),
            faces_by_stable_key: HashMap::new(),
            people: HashMap::from([(person.id.clone(), person)]),
            assignments: HashMap::from([
                (
                    "face-private".to_string(),
                    FaceAssignment {
                        face_id: "face-private".to_string(),
                        person_id: "person-1".to_string(),
                        source: AssignmentSource::Automatic,
                    },
                ),
                (
                    "face-shared".to_string(),
                    FaceAssignment {
                        face_id: "face-shared".to_string(),
                        person_id: "person-1".to_string(),
                        source: AssignmentSource::Automatic,
                    },
                ),
            ]),
        }
    }

    #[test]
    fn shared_scope_excludes_private_album_photos() {
        let albums = HashMap::from([(
            "owner".to_string(),
            BTreeSet::from(["shared".to_string()]),
        )]);
        let records = vec![
            photo("owner", "private", "albums/private/a.jpg"),
            photo("owner", "shared", "albums/shared/b.jpg"),
            photo("other", "shared", "albums/shared/c.jpg"),
        ];
        let filtered = filter_photos_for_scope(records, &albums);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].album, "shared");
        assert_eq!(filtered[0].owner, "owner");
    }

    #[test]
    fn admin_without_album_access_sees_nothing() {
        let albums = HashMap::<String, BTreeSet<String>>::new();
        let records = vec![photo("owner", "private", "albums/private/a.jpg")];
        assert!(filter_photos_for_scope(records, &albums).is_empty());
    }

    #[test]
    fn shared_user_only_sees_faces_in_shared_albums() {
        let index = sample_index();
        let allowed = BTreeSet::from(["shared".to_string()]);
        assert_eq!(count_visible_person_photos("person-1", &index, &allowed), 1);
        let photos = collect_visible_person_photos("person-1", &index, &allowed);
        assert_eq!(photos.len(), 1);
        assert_eq!(photos[0]["album"], "shared");
        let person = index.people.get("person-1").unwrap();
        let cover = build_person_cover(person, &index, &allowed);
        // Preferred cover is private; fall back to the shared face.
        assert_eq!(cover.unwrap()["album"], "shared");
    }

    #[test]
    fn person_with_no_visible_faces_is_empty() {
        let index = sample_index();
        let allowed = BTreeSet::from(["missing".to_string()]);
        assert_eq!(count_visible_person_photos("person-1", &index, &allowed), 0);
        assert!(collect_visible_person_photos("person-1", &index, &allowed).is_empty());
    }

    #[test]
    fn only_owner_can_edit_people() {
        assert!(person_is_editable("alice", "alice"));
        assert!(!person_is_editable("alice", "bob"));
        assert_eq!(person_mutation_owner("bob"), "bob");
    }

    #[test]
    fn rejects_malicious_photo_paths() {
        assert!(normalize_ai_photo_path("../etc/passwd").is_err());
        assert!(normalize_ai_photo_path("/absolute.jpg").is_err());
        assert!(normalize_ai_photo_path("albums/foo/./bar.jpg").is_err());
        assert!(normalize_ai_photo_path("albums/foo/bar.jpg").is_ok());
    }

    #[test]
    fn rejects_oversized_text_queries() {
        let long = "字".repeat(MAX_AI_TEXT_QUERY_CHARS + 1);
        assert!(normalize_ai_text_query(&long).is_err());
        assert!(normalize_ai_text_query("海边日落").is_ok());
    }
}
