//! Per-album user face configuration: `data/{owner}/albums/{album}/faces.json`.
//!
//! Appearances hang under centers (no `center_id` link). Identity is `(source, box_xywh)`.
//! Face crop images live in AI cache under `face-crops/{face_id}.webp`
//! (`face_id` = hash(stable_key|box)).
//!
//! Old / incompatible JSON fails to parse and loads as empty (no migration).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use super::album_cache::{
    NamedPersonCenter, appearance_identity_key, decode_f32_vector_b64, max_similarity_to_centers,
};
use crate::backend::ai::faces::iou_box_xywh;
use crate::backend::constants::MAX_PERSON_NAME_LEN;
use crate::backend::constants::{
    DEFAULT_CLUSTER_SIMILARITY, DEFAULT_PERSON_MATCH_MIN_SCORE, RECONCILE_FACE_IOU_MIN,
};

pub const FACES_CONFIG_VERSION: u32 = 3;

fn default_cluster_similarity() -> f32 {
    DEFAULT_CLUSTER_SIMILARITY
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FacesConfig {
    pub version: u32,
    /// Neighbor similarity used the last time this album was clustered.
    /// Missing on older files; defaults to the historical DBSCAN threshold.
    #[serde(default = "default_cluster_similarity")]
    pub cluster_similarity: f32,
    #[serde(default)]
    pub people: Vec<AlbumPerson>,
}

impl Default for FacesConfig {
    fn default() -> Self {
        Self {
            version: FACES_CONFIG_VERSION,
            cluster_similarity: DEFAULT_CLUSTER_SIMILARITY,
            people: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlbumPerson {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub hidden: bool,
    /// Multiple embedding centers for the same person (mask / sunglasses / …).
    /// Appearances for each look live under that center.
    #[serde(default)]
    pub centers: Vec<PersonCenter>,
    /// User-selected avatar appearance for lists / filters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<PersonAppearance>,
}

impl AlbumPerson {
    pub fn appearances_iter(&self) -> impl Iterator<Item = &PersonAppearance> {
        self.centers.iter().flat_map(|c| c.appearances.iter())
    }

    pub fn appearance_count(&self) -> usize {
        self.centers.iter().map(|c| c.appearances.len()).sum()
    }

    /// Distinct source photos this person appears in (not face-box count).
    pub fn visible_photo_count(&self) -> usize {
        self.appearances_iter()
            .map(|a| a.source.as_str())
            .collect::<HashSet<_>>()
            .len()
    }

    /// Remove appearances whose identity keys are in `selected`.
    /// Drops centers that become empty (keeps person even if all centers gone).
    pub fn take_appearances_by_keys(
        &mut self,
        selected: &HashSet<String>,
    ) -> Vec<PersonAppearance> {
        let mut moved = Vec::new();
        for center in &mut self.centers {
            let (keep, take): (Vec<_>, Vec<_>) = center
                .appearances
                .drain(..)
                .partition(|a| !selected.contains(&a.identity_key()));
            center.appearances = keep;
            moved.extend(take);
        }
        self.centers.retain(|c| !c.appearances.is_empty());
        moved
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonCenter {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Relative path under album AI cache, e.g. `faces/{key}.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_ref: Option<String>,
    /// Optional inline center vector (f32 LE base64) so matching works without re-reading cache.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub center_b64: Option<String>,
    #[serde(default)]
    pub appearances: Vec<PersonAppearance>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonAppearance {
    /// Album-relative path, e.g. `2026/PANA5801.JPG`.
    pub source: String,
    pub box_xywh: [f32; 4],
    /// Deterministic face id from the AI index (when known).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face_id: Option<String>,
}

impl PersonAppearance {
    pub fn new(source: &str, box_xywh: [f32; 4]) -> Self {
        Self {
            source: source.to_string(),
            box_xywh,
            face_id: None,
        }
    }

    pub fn identity_key(&self) -> String {
        appearance_identity_key(&self.source, &self.box_xywh)
    }
}

pub fn faces_config_path(album_dir: &Path) -> PathBuf {
    album_dir.join("faces.json")
}

pub async fn collect_person_photo_paths_from_config_async(
    album_dir: &Path,
    album_name: &str,
    person_ids: &[String],
) -> HashSet<String> {
    if person_ids.is_empty() {
        return HashSet::new();
    }
    let wanted: HashSet<&str> = person_ids.iter().map(String::as_str).collect();
    let config = load_faces_config_async(album_dir).await;
    collect_person_photo_paths_from_loaded(&config, album_name, &wanted)
}

fn collect_person_photo_paths_from_loaded(
    config: &FacesConfig,
    album_name: &str,
    wanted: &HashSet<&str>,
) -> HashSet<String> {
    let mut paths = HashSet::new();
    for person in &config.people {
        if !wanted.contains(person.id.as_str()) {
            continue;
        }
        for appearance in person.appearances_iter() {
            let rel = appearance
                .source
                .replace('\\', "/")
                .trim_start_matches('/')
                .to_string();
            if rel.is_empty() {
                continue;
            }
            paths.insert(format!("albums/{album_name}/{rel}"));
        }
    }
    paths
}

pub async fn load_faces_config_async(album_dir: &Path) -> FacesConfig {
    let path = faces_config_path(album_dir);
    let Ok(raw) = tokio::fs::read_to_string(path).await else {
        return FacesConfig::default();
    };
    let mut config = serde_json::from_str::<FacesConfig>(&raw)
        .ok()
        .filter(|config| config.version == FACES_CONFIG_VERSION)
        .unwrap_or_default();
    config.sanitize_covers();
    config
}

pub async fn save_faces_config_async(album_dir: &Path, config: &FacesConfig) -> Result<(), String> {
    let path = faces_config_path(album_dir);
    let mut config = config.clone();
    config.version = FACES_CONFIG_VERSION;
    config.sanitize_covers();
    let bytes =
        serde_json::to_vec_pretty(&config).map_err(|e| format!("serialize faces.json: {e}"))?;
    let tmp = album_dir.join(".faces.json.tmp");
    tokio::fs::write(&tmp, &bytes)
        .await
        .map_err(|e| format!("write faces.json tmp: {e}"))?;
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| format!("commit faces.json: {e}"))?;
    Ok(())
}

pub fn normalize_person_name(name: &str) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("person name cannot be empty".to_string());
    }
    if trimmed.chars().count() > MAX_PERSON_NAME_LEN {
        return Err(format!(
            "person name exceeds {MAX_PERSON_NAME_LEN} characters"
        ));
    }
    Ok(trimmed.to_string())
}

/// Album-relative photo path key used to group appearances and detections.
pub fn normalize_source_path(source: &str) -> String {
    source
        .replace('\\', "/")
        .trim_start_matches('/')
        .to_string()
}

/// One face detected during the current index pass (for reconcile).
#[derive(Debug, Clone)]
pub struct ReconcileDetection {
    pub face_id: String,
    pub box_xywh: [f32; 4],
    pub embedding: Vec<f32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileStats {
    pub updated: usize,
    pub unchanged: usize,
    pub unmatched_appearances: usize,
}

struct AppearanceRef {
    person_id: String,
    center_idx: usize,
    app_idx: usize,
    old_box: [f32; 4],
}

fn boxes_close(left: &[f32; 4], right: &[f32; 4]) -> bool {
    (0..4).all(|i| (left[i] - right[i]).abs() < 1e-3)
}

fn decode_center_embedding(center: &PersonCenter) -> Option<Vec<f32>> {
    center
        .center_b64
        .as_deref()
        .and_then(|encoded| decode_f32_vector_b64(encoded, None).ok())
        .filter(|vector| !vector.is_empty())
}

fn apply_detection_to_appearance(
    config: &mut FacesConfig,
    person_id: &str,
    center_idx: usize,
    app_idx: usize,
    detection: &ReconcileDetection,
) -> bool {
    let Some(person) = config.find_person_mut(person_id) else {
        return false;
    };
    let Some(center) = person.centers.get_mut(center_idx) else {
        return false;
    };
    let Some(app) = center.appearances.get_mut(app_idx) else {
        return false;
    };
    let changed = !boxes_close(&app.box_xywh, &detection.box_xywh)
        || app.face_id.as_deref() != Some(detection.face_id.as_str());
    app.box_xywh = detection.box_xywh;
    app.face_id = Some(detection.face_id.clone());
    changed
}

fn reconcile_source(
    config: &mut FacesConfig,
    refs: &[AppearanceRef],
    detections: &[ReconcileDetection],
    stats: &mut ReconcileStats,
) {
    if refs.is_empty() {
        return;
    }
    let mut matched = vec![false; refs.len()];
    let mut det_used = vec![false; detections.len()];

    // Exact box match (re-index with stable detector output).
    for (ri, reference) in refs.iter().enumerate() {
        for (di, detection) in detections.iter().enumerate() {
            if det_used[di] || !boxes_close(&reference.old_box, &detection.box_xywh) {
                continue;
            }
            if apply_detection_to_appearance(
                config,
                &reference.person_id,
                reference.center_idx,
                reference.app_idx,
                detection,
            ) {
                stats.updated += 1;
            } else {
                stats.unchanged += 1;
            }
            matched[ri] = true;
            det_used[di] = true;
            break;
        }
    }

    // IoU match (box drift after re-detect).
    let mut iou_pairs = Vec::new();
    for (ri, reference) in refs.iter().enumerate() {
        if matched[ri] {
            continue;
        }
        for (di, detection) in detections.iter().enumerate() {
            let iou = iou_box_xywh(&reference.old_box, &detection.box_xywh);
            if iou >= RECONCILE_FACE_IOU_MIN {
                iou_pairs.push((iou, ri, di));
            }
        }
    }
    iou_pairs.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (_, ri, di) in iou_pairs {
        if matched[ri] || det_used[di] {
            continue;
        }
        let reference = &refs[ri];
        let detection = &detections[di];
        if apply_detection_to_appearance(
            config,
            &reference.person_id,
            reference.center_idx,
            reference.app_idx,
            detection,
        ) {
            stats.updated += 1;
        } else {
            stats.unchanged += 1;
        }
        matched[ri] = true;
        det_used[di] = true;
    }

    // Embedding fallback: match unmatched appearances to same-photo detections via person centers.
    for (ri, reference) in refs.iter().enumerate() {
        if matched[ri] {
            continue;
        }
        let mut best: Option<(usize, f32)> = None;
        for (di, detection) in detections.iter().enumerate() {
            if det_used[di] {
                continue;
            }
            let Some(score) = config.person_match_score(
                reference.person_id.as_str(),
                &detection.embedding,
                decode_center_embedding,
            ) else {
                continue;
            };
            if score < DEFAULT_PERSON_MATCH_MIN_SCORE {
                continue;
            }
            if best
                .map(|(_, best_score)| score > best_score)
                .unwrap_or(true)
            {
                best = Some((di, score));
            }
        }
        let Some((di, _)) = best else {
            stats.unmatched_appearances += 1;
            continue;
        };
        let detection = &detections[di];
        if apply_detection_to_appearance(
            config,
            &reference.person_id,
            reference.center_idx,
            reference.app_idx,
            detection,
        ) {
            stats.updated += 1;
        } else {
            stats.unchanged += 1;
        }
        matched[ri] = true;
        det_used[di] = true;
    }
}

/// After face re-detection, remap named appearances in `faces.json` onto new boxes/ids.
pub fn reconcile_faces_config(
    config: &mut FacesConfig,
    detections_by_source: &HashMap<String, Vec<ReconcileDetection>>,
) -> ReconcileStats {
    let mut by_source: HashMap<String, Vec<AppearanceRef>> = HashMap::new();
    for person in &config.people {
        for (center_idx, center) in person.centers.iter().enumerate() {
            for (app_idx, appearance) in center.appearances.iter().enumerate() {
                let source = normalize_source_path(&appearance.source);
                by_source.entry(source).or_default().push(AppearanceRef {
                    person_id: person.id.clone(),
                    center_idx,
                    app_idx,
                    old_box: appearance.box_xywh,
                });
            }
        }
    }

    let mut stats = ReconcileStats::default();
    for (source, refs) in by_source {
        let Some(detections) = detections_by_source.get(&source) else {
            stats.unmatched_appearances += refs.len();
            continue;
        };
        if detections.is_empty() {
            stats.unmatched_appearances += refs.len();
            continue;
        }
        reconcile_source(config, &refs, detections, &mut stats);
    }
    config.sanitize_covers();
    stats
}

/// Named-appearance lookup for excluding assigned faces from natural clustering.
pub struct AssignedFaceFilter {
    identity_keys: HashSet<String>,
    face_ids: HashSet<String>,
    boxes_by_source: HashMap<String, Vec<[f32; 4]>>,
}

impl AssignedFaceFilter {
    pub fn from_config(config: &FacesConfig) -> Self {
        let mut identity_keys = HashSet::new();
        let mut face_ids = HashSet::new();
        let mut boxes_by_source: HashMap<String, Vec<[f32; 4]>> = HashMap::new();
        for appearance in config
            .people
            .iter()
            .flat_map(|person| person.appearances_iter())
        {
            identity_keys.insert(appearance.identity_key());
            if let Some(face_id) = appearance.face_id.as_deref() {
                face_ids.insert(face_id.to_string());
            }
            boxes_by_source
                .entry(normalize_source_path(&appearance.source))
                .or_default()
                .push(appearance.box_xywh);
        }
        Self {
            identity_keys,
            face_ids,
            boxes_by_source,
        }
    }

    /// Whether a detected face belongs to a named person (exact key, face_id, or IoU on same photo).
    pub fn is_assigned(&self, source: &str, box_xywh: &[f32; 4], face_id: Option<&str>) -> bool {
        let key = appearance_identity_key(source, box_xywh);
        if self.identity_keys.contains(&key) {
            return true;
        }
        if face_id.is_some_and(|id| self.face_ids.contains(id)) {
            return true;
        }
        let source = normalize_source_path(source);
        self.boxes_by_source.get(&source).is_some_and(|boxes| {
            boxes
                .iter()
                .any(|named| iou_box_xywh(named, box_xywh) >= RECONCILE_FACE_IOU_MIN)
        })
    }
}

impl FacesConfig {
    /// Stable keys for named appearances (`hash(source|box)`).
    pub fn assigned_appearance_keys(&self) -> HashSet<String> {
        self.people
            .iter()
            .flat_map(|person| {
                person
                    .appearances_iter()
                    .map(PersonAppearance::identity_key)
            })
            .collect()
    }

    pub fn find_person_mut(&mut self, person_id: &str) -> Option<&mut AlbumPerson> {
        self.people.iter_mut().find(|p| p.id == person_id)
    }

    pub fn find_person(&self, person_id: &str) -> Option<&AlbumPerson> {
        self.people.iter().find(|p| p.id == person_id)
    }

    pub fn rename_person(&mut self, person_id: &str, name: &str) -> Result<(), String> {
        let name = normalize_person_name(name)?;
        let person = self
            .find_person_mut(person_id)
            .ok_or_else(|| format!("person {person_id} not found"))?;
        person.name = name;
        Ok(())
    }

    pub fn set_hidden(&mut self, person_id: &str, hidden: bool) -> Result<(), String> {
        let person = self
            .find_person_mut(person_id)
            .ok_or_else(|| format!("person {person_id} not found"))?;
        person.hidden = hidden;
        Ok(())
    }

    pub fn set_cover(
        &mut self,
        person_id: &str,
        appearance: &PersonAppearance,
    ) -> Result<(), String> {
        let key = appearance.identity_key();
        let person = self
            .find_person_mut(person_id)
            .ok_or_else(|| format!("person {person_id} not found"))?;
        let cover = person
            .appearances_iter()
            .find(|a| a.identity_key() == key)
            .cloned()
            .ok_or_else(|| "cover appearance not found on person".to_string())?;
        person.cover = Some(cover);
        Ok(())
    }

    pub fn clear_cover(&mut self, person_id: &str) -> Result<(), String> {
        let person = self
            .find_person_mut(person_id)
            .ok_or_else(|| format!("person {person_id} not found"))?;
        person.cover = None;
        Ok(())
    }

    /// Drop covers that no longer match any appearance on the person.
    pub fn sanitize_covers(&mut self) {
        for person in &mut self.people {
            let Some(cover) = person.cover.as_ref() else {
                continue;
            };
            let key = cover.identity_key();
            let valid = person
                .appearances_iter()
                .any(|appearance| appearance.identity_key() == key);
            if !valid {
                person.cover = None;
            }
        }
    }

    /// Remove a named person; their appearances become unassigned (suggestions again).
    pub fn remove_person(&mut self, person_id: &str) -> Result<(), String> {
        let before = self.people.len();
        self.people.retain(|person| person.id != person_id);
        if self.people.len() == before {
            return Err(format!("person {person_id} not found"));
        }
        Ok(())
    }

    /// Remove one center from a person; appearances become unassigned (suggestions again).
    /// If no centers remain, the person is deleted. Returns removed appearance count.
    pub fn remove_center(&mut self, person_id: &str, center_id: &str) -> Result<usize, String> {
        let removed_count = {
            let person = self
                .find_person_mut(person_id)
                .ok_or_else(|| format!("person {person_id} not found"))?;
            let center_idx = person
                .centers
                .iter()
                .position(|c| c.id == center_id)
                .ok_or_else(|| format!("center {center_id} not found on person {person_id}"))?;
            let removed_count = person.centers[center_idx].appearances.len();
            person.centers.remove(center_idx);
            removed_count
        };
        self.sanitize_covers();
        if self
            .find_person(person_id)
            .is_some_and(|person| person.centers.is_empty())
        {
            self.remove_person(person_id)?;
        }
        Ok(removed_count)
    }

    fn assert_appearances_unassigned(
        &self,
        appearances: &[PersonAppearance],
    ) -> Result<(), String> {
        let assigned = self.assigned_appearance_keys();
        for appearance in appearances {
            let key = appearance.identity_key();
            if assigned.contains(&key) {
                return Err(format!(
                    "appearance {} @ {:?} is already assigned",
                    appearance.source, appearance.box_xywh
                ));
            }
        }
        Ok(())
    }

    /// Adopt a cluster as a new named person (single center owning the appearances).
    pub fn adopt_cluster_as_person(
        &mut self,
        name: &str,
        appearances: Vec<PersonAppearance>,
        mut center: PersonCenter,
    ) -> Result<String, String> {
        let name = normalize_person_name(name)?;
        if appearances.is_empty() {
            return Err("no faces to adopt".to_string());
        }
        self.assert_appearances_unassigned(&appearances)?;
        center.appearances = appearances;
        let person_id = Uuid::new_v4().to_string();
        self.people.push(AlbumPerson {
            id: person_id.clone(),
            name,
            hidden: false,
            centers: vec![center],
            cover: None,
        });
        Ok(person_id)
    }

    /// Merge an additional cluster into an existing person as a new center (no averaging).
    pub fn append_cluster_as_center(
        &mut self,
        person_id: &str,
        appearances: Vec<PersonAppearance>,
        mut center: PersonCenter,
    ) -> Result<(), String> {
        if appearances.is_empty() {
            return Err("no faces to merge".to_string());
        }
        let assigned = self.assigned_appearance_keys();
        for appearance in &appearances {
            let key = appearance.identity_key();
            // Allow re-adding only if already on this person (idempotent merge).
            if assigned.contains(&key) {
                let on_self = self
                    .find_person(person_id)
                    .map(|p| p.appearances_iter().any(|a| a.identity_key() == key))
                    .unwrap_or(false);
                if !on_self {
                    return Err(format!(
                        "appearance {} @ {:?} belongs to another person",
                        appearance.source, appearance.box_xywh
                    ));
                }
            }
        }
        // Drop appearances already on this person (idempotent).
        let person = self
            .find_person(person_id)
            .ok_or_else(|| format!("person {person_id} not found"))?;
        let existing: HashSet<String> = person
            .appearances_iter()
            .map(PersonAppearance::identity_key)
            .collect();
        let appearances: Vec<_> = appearances
            .into_iter()
            .filter(|a| !existing.contains(&a.identity_key()))
            .collect();
        if appearances.is_empty() {
            return Ok(());
        }
        center.appearances = appearances;
        let person = self
            .find_person_mut(person_id)
            .ok_or_else(|| format!("person {person_id} not found"))?;
        person.centers.push(center);
        Ok(())
    }

    /// Match a query embedding against a person's centers (max similarity).
    pub fn person_match_score(
        &self,
        person_id: &str,
        query: &[f32],
        decode_center: impl Fn(&PersonCenter) -> Option<Vec<f32>>,
    ) -> Option<f32> {
        let person = self.find_person(person_id)?;
        let centers: Vec<Vec<f32>> = person
            .centers
            .iter()
            .filter_map(|c| decode_center(c))
            .collect();
        if centers.is_empty() {
            return None;
        }
        Some(max_similarity_to_centers(query, &centers))
    }
}

/// Named-person center embeddings for matching unnamed face suggestions.
pub fn collect_named_person_centers(config: &FacesConfig) -> Vec<NamedPersonCenter> {
    let mut out = Vec::new();
    for person in &config.people {
        if person.hidden {
            continue;
        }
        let name = person.name.trim();
        if name.is_empty() {
            continue;
        }
        for center in &person.centers {
            let Some(center_b64) = center.center_b64.as_deref() else {
                continue;
            };
            let Ok(embedding) = decode_f32_vector_b64(center_b64, None) else {
                continue;
            };
            if embedding.is_empty() {
                continue;
            }
            out.push(NamedPersonCenter {
                person_id: person.id.clone(),
                person_name: name.to_string(),
                center_id: center.id.clone(),
                embedding,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai::album_cache::encode_f32_vector_b64;

    #[tokio::test]
    async fn faces_config_roundtrip_nested_appearances() {
        let dir = std::env::temp_dir().join(format!("album-faces-cfg-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = FacesConfig::default();
        let center = PersonCenter {
            id: "c1".to_string(),
            label: Some("default".to_string()),
            embedding_ref: None,
            center_b64: Some(encode_f32_vector_b64(&[1.0, 0.0])),
            appearances: Vec::new(),
        };
        let appearance = PersonAppearance::new("a.jpg", [0.1, 0.2, 0.3, 0.4]);
        let person_id = config
            .adopt_cluster_as_person("Alice", vec![appearance], center)
            .unwrap();

        let center2 = PersonCenter {
            id: "c2".to_string(),
            label: Some("masked".to_string()),
            embedding_ref: None,
            center_b64: Some(encode_f32_vector_b64(&[0.0, 1.0])),
            appearances: Vec::new(),
        };
        config
            .append_cluster_as_center(
                &person_id,
                vec![PersonAppearance::new("b.jpg", [0.2, 0.2, 0.3, 0.4])],
                center2,
            )
            .unwrap();

        save_faces_config_async(&dir, &config).await.unwrap();
        let loaded = load_faces_config_async(&dir).await;
        assert_eq!(loaded.version, 3);
        assert_eq!(loaded.people.len(), 1);
        assert_eq!(loaded.people[0].centers.len(), 2);
        assert_eq!(loaded.people[0].centers[0].appearances.len(), 1);
        assert_eq!(loaded.people[0].centers[1].appearances.len(), 1);
        assert_eq!(loaded.assigned_appearance_keys().len(), 2);
        let raw = serde_json::to_string(&loaded).unwrap();
        assert!(!raw.contains("face_id"));
        assert!(!raw.contains("center_id"));
        assert_eq!(loaded.people[0].appearance_count(), 2);
        assert_eq!(loaded.people[0].visible_photo_count(), 2);

        let score = loaded
            .person_match_score(&person_id, &[0.0, 1.0], |c| {
                c.center_b64.as_ref().and_then(|b| {
                    crate::backend::ai::album_cache::decode_f32_vector_b64(b, None).ok()
                })
            })
            .unwrap();
        assert!(score > 0.99);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn visible_photo_count_dedupes_same_source() {
        let mut config = FacesConfig::default();
        config
            .adopt_cluster_as_person(
                "Pat",
                vec![
                    PersonAppearance::new("a.jpg", [0.1, 0.1, 0.2, 0.2]),
                    PersonAppearance::new("a.jpg", [0.5, 0.5, 0.2, 0.2]),
                    PersonAppearance::new("b.jpg", [0.1, 0.1, 0.2, 0.2]),
                ],
                PersonCenter {
                    id: "c1".to_string(),
                    label: None,
                    embedding_ref: None,
                    center_b64: None,
                    appearances: Vec::new(),
                },
            )
            .unwrap();
        assert_eq!(config.people[0].appearance_count(), 3);
        assert_eq!(config.people[0].visible_photo_count(), 2);
    }

    #[test]
    fn remove_person_drops_all_centers_and_appearances() {
        let dir = std::env::temp_dir().join(format!("album-faces-rm-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = FacesConfig::default();
        let person_id = config
            .adopt_cluster_as_person(
                "Bob",
                vec![PersonAppearance::new("x.jpg", [0.1, 0.1, 0.2, 0.2])],
                PersonCenter {
                    id: "c1".into(),
                    label: None,
                    embedding_ref: None,
                    center_b64: None,
                    appearances: Vec::new(),
                },
            )
            .unwrap();
        assert_eq!(config.assigned_appearance_keys().len(), 1);
        config.remove_person(&person_id).unwrap();
        assert!(config.people.is_empty());
        assert!(config.assigned_appearance_keys().is_empty());
        assert!(config.remove_person("missing").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn person_paths_from_config_use_album_prefix() {
        let dir = std::env::temp_dir().join(format!("album-faces-paths-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut config = FacesConfig::default();
        let center = PersonCenter {
            id: "c1".to_string(),
            label: None,
            embedding_ref: None,
            center_b64: None,
            appearances: Vec::new(),
        };
        config
            .adopt_cluster_as_person(
                "Bob",
                vec![PersonAppearance::new("2026/a.jpg", [0.1, 0.2, 0.3, 0.4])],
                center,
            )
            .unwrap();
        save_faces_config_async(&dir, &config).await.unwrap();
        let paths = collect_person_photo_paths_from_config_async(
            &dir,
            "raw",
            &[config.people[0].id.clone()],
        )
        .await;
        assert!(paths.contains("albums/raw/2026/a.jpg"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn incompatible_or_legacy_faces_json_loads_empty() {
        let dir = std::env::temp_dir().join(format!("album-faces-legacy-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let legacy = r#"{
          "version": 2,
          "people": [{
            "id": "p1",
            "name": "Ada",
            "hidden": false,
            "centers": [{"id": "c1", "center_b64": null}],
            "appearances": [{
              "source": "a.jpg",
              "box_xywh": [0.1, 0.2, 0.3, 0.4],
              "center_id": "c1"
            }]
          }]
        }"#;
        std::fs::write(dir.join("faces.json"), legacy).unwrap();
        let loaded = load_faces_config_async(&dir).await;
        assert!(loaded.people.is_empty());
        assert_eq!(loaded.version, FACES_CONFIG_VERSION);

        std::fs::write(dir.join("faces.json"), "{not json").unwrap();
        let loaded = load_faces_config_async(&dir).await;
        assert!(loaded.people.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn take_appearances_by_keys_cleans_empty_centers() {
        let mut person = AlbumPerson {
            id: "p".into(),
            name: "A".into(),
            hidden: false,
            cover: None,
            centers: vec![
                PersonCenter {
                    id: "c1".into(),
                    label: None,
                    embedding_ref: None,
                    center_b64: None,
                    appearances: vec![PersonAppearance::new("a.jpg", [0.1, 0.2, 0.3, 0.4])],
                },
                PersonCenter {
                    id: "c2".into(),
                    label: None,
                    embedding_ref: None,
                    center_b64: None,
                    appearances: vec![PersonAppearance::new("b.jpg", [0.2, 0.2, 0.3, 0.4])],
                },
            ],
        };
        let key = person.centers[0].appearances[0].identity_key();
        let mut selected = HashSet::new();
        selected.insert(key);
        let moved = person.take_appearances_by_keys(&selected);
        assert_eq!(moved.len(), 1);
        assert_eq!(person.centers.len(), 1);
        assert_eq!(person.centers[0].id, "c2");
    }

    #[test]
    fn set_cover_requires_person_appearance() {
        let mut config = FacesConfig::default();
        let person_id = config
            .adopt_cluster_as_person(
                "Ada",
                vec![PersonAppearance::new("a.jpg", [0.1, 0.2, 0.3, 0.4])],
                PersonCenter {
                    id: "c1".into(),
                    label: None,
                    embedding_ref: None,
                    center_b64: None,
                    appearances: Vec::new(),
                },
            )
            .unwrap();
        let appearance = PersonAppearance::new("a.jpg", [0.1, 0.2, 0.3, 0.4]);
        config.set_cover(&person_id, &appearance).unwrap();
        assert!(config.find_person(&person_id).unwrap().cover.is_some());

        let other = PersonAppearance::new("missing.jpg", [0.1, 0.2, 0.3, 0.4]);
        assert!(config.set_cover(&person_id, &other).is_err());

        config.clear_cover(&person_id).unwrap();
        assert!(config.find_person(&person_id).unwrap().cover.is_none());
    }

    #[test]
    fn sanitize_covers_drops_stale_cover() {
        let mut config = FacesConfig::default();
        let person_id = config
            .adopt_cluster_as_person(
                "Bob",
                vec![PersonAppearance::new("a.jpg", [0.1, 0.2, 0.3, 0.4])],
                PersonCenter {
                    id: "c1".into(),
                    label: None,
                    embedding_ref: None,
                    center_b64: None,
                    appearances: Vec::new(),
                },
            )
            .unwrap();
        config
            .set_cover(
                &person_id,
                &PersonAppearance::new("a.jpg", [0.1, 0.2, 0.3, 0.4]),
            )
            .unwrap();
        let person = config.find_person_mut(&person_id).unwrap();
        person.centers.clear();
        config.sanitize_covers();
        assert!(config.find_person(&person_id).unwrap().cover.is_none());
    }

    fn two_center_person(config: &mut FacesConfig) -> (String, String, String) {
        let person_id = config
            .adopt_cluster_as_person(
                "Ada",
                vec![PersonAppearance::new("a.jpg", [0.1, 0.2, 0.3, 0.4])],
                PersonCenter {
                    id: "c1".into(),
                    label: Some("look-a".into()),
                    embedding_ref: None,
                    center_b64: None,
                    appearances: Vec::new(),
                },
            )
            .unwrap();
        config
            .append_cluster_as_center(
                &person_id,
                vec![PersonAppearance::new("b.jpg", [0.2, 0.2, 0.3, 0.4])],
                PersonCenter {
                    id: "c2".into(),
                    label: Some("look-b".into()),
                    embedding_ref: None,
                    center_b64: None,
                    appearances: Vec::new(),
                },
            )
            .unwrap();
        (person_id, "c1".to_string(), "c2".to_string())
    }

    #[test]
    fn remove_center_unassigns_one_cluster() {
        let mut config = FacesConfig::default();
        let (person_id, c1, c2) = two_center_person(&mut config);
        let removed = config.remove_center(&person_id, &c1).unwrap();
        assert_eq!(removed, 1);
        let person = config.find_person(&person_id).unwrap();
        assert_eq!(person.centers.len(), 1);
        assert_eq!(person.centers[0].id, c2);
        assert!(
            !config
                .assigned_appearance_keys()
                .contains(&PersonAppearance::new("a.jpg", [0.1, 0.2, 0.3, 0.4]).identity_key(),)
        );
    }

    #[test]
    fn remove_center_missing_errors() {
        let mut config = FacesConfig::default();
        let (person_id, _, _) = two_center_person(&mut config);
        assert!(config.remove_center(&person_id, "missing").is_err());
        assert!(config.remove_center("missing", "c1").is_err());
    }

    #[test]
    fn assigned_face_filter_matches_reconciled_box_by_iou() {
        let mut config = FacesConfig::default();
        config
            .adopt_cluster_as_person(
                "Ada",
                vec![PersonAppearance {
                    source: "a.jpg".into(),
                    box_xywh: [0.11, 0.11, 0.20, 0.20],
                    face_id: Some("named-face".into()),
                }],
                PersonCenter {
                    id: "c1".into(),
                    label: None,
                    embedding_ref: None,
                    center_b64: None,
                    appearances: Vec::new(),
                },
            )
            .unwrap();
        let filter = AssignedFaceFilter::from_config(&config);
        assert!(filter.is_assigned("a.jpg", &[0.10, 0.10, 0.20, 0.20], None));
        assert!(filter.is_assigned("a.jpg", &[0.10, 0.10, 0.20, 0.20], Some("named-face")));
        assert!(!filter.is_assigned("b.jpg", &[0.10, 0.10, 0.20, 0.20], None));
    }

    #[test]
    fn reconcile_faces_config_matches_by_iou() {
        let mut config = FacesConfig::default();
        let old_box = [0.10, 0.10, 0.20, 0.20];
        let person_id = config
            .adopt_cluster_as_person(
                "Ada",
                vec![PersonAppearance::new("2026/photo.jpg", old_box)],
                PersonCenter {
                    id: "c1".into(),
                    label: None,
                    embedding_ref: None,
                    center_b64: Some(encode_f32_vector_b64(&[1.0, 0.0])),
                    appearances: Vec::new(),
                },
            )
            .unwrap();

        let new_box = [0.11, 0.11, 0.20, 0.20];
        let mut detections = HashMap::new();
        detections.insert(
            "2026/photo.jpg".to_string(),
            vec![ReconcileDetection {
                face_id: "new-face-id".to_string(),
                box_xywh: new_box,
                embedding: vec![0.0, 1.0],
            }],
        );
        let stats = reconcile_faces_config(&mut config, &detections);
        assert_eq!(stats.updated, 1);
        assert_eq!(stats.unmatched_appearances, 0);

        let app = &config.find_person(&person_id).unwrap().centers[0].appearances[0];
        assert_eq!(app.box_xywh, new_box);
        assert_eq!(app.face_id.as_deref(), Some("new-face-id"));
        assert!(
            config
                .assigned_appearance_keys()
                .contains(&PersonAppearance::new("2026/photo.jpg", new_box).identity_key(),)
        );
    }

    #[test]
    fn reconcile_faces_config_falls_back_to_embedding() {
        let mut config = FacesConfig::default();
        let person_id = config
            .adopt_cluster_as_person(
                "Bob",
                vec![PersonAppearance::new("x.jpg", [0.05, 0.05, 0.15, 0.15])],
                PersonCenter {
                    id: "c1".into(),
                    label: None,
                    embedding_ref: None,
                    center_b64: Some(encode_f32_vector_b64(&[1.0, 0.0])),
                    appearances: Vec::new(),
                },
            )
            .unwrap();

        let new_box = [0.55, 0.55, 0.15, 0.15];
        let mut detections = HashMap::new();
        detections.insert(
            "x.jpg".to_string(),
            vec![ReconcileDetection {
                face_id: "emb-match".to_string(),
                box_xywh: new_box,
                embedding: vec![1.0, 0.0],
            }],
        );
        let stats = reconcile_faces_config(&mut config, &detections);
        assert_eq!(stats.updated, 1);
        let app = &config.find_person(&person_id).unwrap().centers[0].appearances[0];
        assert_eq!(app.box_xywh, new_box);
        assert_eq!(app.face_id.as_deref(), Some("emb-match"));
    }

    #[test]
    fn reconcile_faces_config_one_to_one_two_people() {
        let mut config = FacesConfig::default();
        let box_a = [0.10, 0.10, 0.20, 0.20];
        let box_b = [0.55, 0.10, 0.20, 0.20];
        config
            .adopt_cluster_as_person(
                "Left",
                vec![PersonAppearance::new("group.jpg", box_a)],
                PersonCenter {
                    id: "c1".into(),
                    label: None,
                    embedding_ref: None,
                    center_b64: Some(encode_f32_vector_b64(&[1.0, 0.0])),
                    appearances: Vec::new(),
                },
            )
            .unwrap();
        config
            .adopt_cluster_as_person(
                "Right",
                vec![PersonAppearance::new("group.jpg", box_b)],
                PersonCenter {
                    id: "c2".into(),
                    label: None,
                    embedding_ref: None,
                    center_b64: Some(encode_f32_vector_b64(&[0.0, 1.0])),
                    appearances: Vec::new(),
                },
            )
            .unwrap();

        let drift_a = [0.12, 0.12, 0.20, 0.20];
        let drift_b = [0.57, 0.12, 0.20, 0.20];
        let mut detections = HashMap::new();
        detections.insert(
            "group.jpg".to_string(),
            vec![
                ReconcileDetection {
                    face_id: "face-a".to_string(),
                    box_xywh: drift_a,
                    embedding: vec![1.0, 0.0],
                },
                ReconcileDetection {
                    face_id: "face-b".to_string(),
                    box_xywh: drift_b,
                    embedding: vec![0.0, 1.0],
                },
            ],
        );
        let stats = reconcile_faces_config(&mut config, &detections);
        assert_eq!(stats.updated, 2);
        assert_eq!(stats.unmatched_appearances, 0);

        let mut boxes: Vec<[f32; 4]> = config
            .people
            .iter()
            .flat_map(|p| {
                p.centers
                    .iter()
                    .flat_map(|c| c.appearances.iter().map(|a| a.box_xywh))
            })
            .collect();
        boxes.sort_by(|left, right| {
            left[0]
                .partial_cmp(&right[0])
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        assert_eq!(boxes, vec![drift_a, drift_b]);
    }

    #[test]
    fn remove_last_center_deletes_person() {
        let mut config = FacesConfig::default();
        let person_id = config
            .adopt_cluster_as_person(
                "Solo",
                vec![PersonAppearance::new("a.jpg", [0.1, 0.2, 0.3, 0.4])],
                PersonCenter {
                    id: "c1".into(),
                    label: None,
                    embedding_ref: None,
                    center_b64: None,
                    appearances: Vec::new(),
                },
            )
            .unwrap();
        let removed = config.remove_center(&person_id, "c1").unwrap();
        assert_eq!(removed, 1);
        assert!(config.find_person(&person_id).is_none());
    }
}
