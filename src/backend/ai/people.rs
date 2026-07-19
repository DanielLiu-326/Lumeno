use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use super::embedding::cosine_similarity;

pub const DEFAULT_CLUSTER_EPS: f32 = 0.38;
pub const DEFAULT_CLUSTER_MIN_SAMPLES: usize = 2;
pub const MAX_PERSON_NAME_LEN: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentSource {
    Automatic,
    ManualName,
    ManualMerge,
    ManualSplit,
}

impl AssignmentSource {
    pub fn is_manual(self) -> bool {
        !matches!(self, Self::Automatic)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonRecord {
    pub id: String,
    pub name: String,
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover_face_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FaceAssignment {
    pub face_id: String,
    pub person_id: String,
    pub source: AssignmentSource,
}

#[derive(Debug, Clone)]
pub struct ClusterFace {
    pub face_id: String,
    pub embedding: Vec<f32>,
}

fn cosine_distance(left: &[f32], right: &[f32]) -> f32 {
    1.0 - cosine_similarity(left, right)
}

fn region_query(faces: &[ClusterFace], index: usize, eps: f32) -> Vec<usize> {
    faces
        .iter()
        .enumerate()
        .filter(|(candidate_index, candidate)| {
            *candidate_index != index
                && cosine_distance(&faces[index].embedding, &candidate.embedding) <= eps
        })
        .map(|(candidate_index, _)| candidate_index)
        .collect()
}

/// DBSCAN clustering on cosine distance `1 - dot`. Returns face_id -> person_id for clustered faces.
pub fn cluster_faces(
    faces: &[ClusterFace],
    eps: f32,
    min_samples: usize,
) -> HashMap<String, String> {
    let len = faces.len();
    if len == 0 {
        return HashMap::new();
    }

    let mut labels = vec![None::<isize>; len];
    let mut cluster_id = 0_isize;

    for index in 0..len {
        if labels[index].is_some() {
            continue;
        }
        let neighbors = region_query(faces, index, eps);
        if neighbors.len() + 1 < min_samples {
            labels[index] = Some(-1);
            continue;
        }

        labels[index] = Some(cluster_id);
        let mut seeds = neighbors;
        let mut seed_index = 0usize;
        while seed_index < seeds.len() {
            let current = seeds[seed_index];
            if labels[current] == Some(-1) {
                labels[current] = Some(cluster_id);
            }
            if labels[current].is_some() {
                seed_index += 1;
                continue;
            }
            labels[current] = Some(cluster_id);
            let current_neighbors = region_query(faces, current, eps);
            if current_neighbors.len() + 1 >= min_samples {
                for neighbor in current_neighbors {
                    if !seeds.contains(&neighbor) {
                        seeds.push(neighbor);
                    }
                }
            }
            seed_index += 1;
        }
        cluster_id += 1;
    }

    let mut assignments = HashMap::new();
    for (label, face) in labels.into_iter().zip(faces.iter()) {
        let Some(cluster) = label.filter(|value| *value >= 0) else {
            continue;
        };
        let person_id = format!("cluster-{cluster}");
        assignments.insert(face.face_id.clone(), person_id);
    }

    // Replace temporary cluster keys with UUIDs.
    let mut cluster_to_person = HashMap::<String, String>::new();
    for person_key in assignments.values() {
        cluster_to_person
            .entry(person_key.clone())
            .or_insert_with(|| Uuid::new_v4().to_string());
    }
    assignments
        .into_iter()
        .map(|(face_id, cluster_key)| {
            (
                face_id,
                cluster_to_person
                    .get(&cluster_key)
                    .cloned()
                    .unwrap_or_else(|| Uuid::new_v4().to_string()),
            )
        })
        .collect()
}

fn validate_single_assignment(assignments: &[FaceAssignment]) -> Result<(), String> {
    let mut seen = HashSet::new();
    for assignment in assignments {
        if !seen.insert(assignment.face_id.clone()) {
            return Err(format!(
                "face {} assigned to multiple people",
                assignment.face_id
            ));
        }
    }
    Ok(())
}

fn normalize_person_name(name: &str) -> Result<String, String> {
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

pub fn update_person(
    people: &mut [PersonRecord],
    person_id: &str,
    name: Option<&str>,
    hidden: Option<bool>,
) -> Result<(), String> {
    let person = people
        .iter_mut()
        .find(|person| person.id == person_id)
        .ok_or_else(|| format!("person {person_id} not found"))?;
    if let Some(name) = name {
        person.name = normalize_person_name(name)?;
    }
    if let Some(hidden) = hidden {
        person.hidden = hidden;
    }
    Ok(())
}

pub fn merge_people(
    people: &mut Vec<PersonRecord>,
    assignments: &mut Vec<FaceAssignment>,
    target_person_id: &str,
    source_person_ids: &[String],
) -> Result<(), String> {
    if source_person_ids.is_empty() {
        return Err("no source people selected for merge".to_string());
    }
    if source_person_ids.iter().any(|id| id == target_person_id) {
        return Err("target person cannot also be a merge source".to_string());
    }
    if !people.iter().any(|person| person.id == target_person_id) {
        return Err(format!("target person {target_person_id} not found"));
    }

    let source_set: HashSet<_> = source_person_ids.iter().cloned().collect();
    for assignment in assignments.iter_mut() {
        if source_set.contains(&assignment.person_id) {
            assignment.person_id = target_person_id.to_string();
            if assignment.source == AssignmentSource::Automatic {
                assignment.source = AssignmentSource::ManualMerge;
            }
        }
    }

    people.retain(|person| !source_set.contains(&person.id));
    validate_single_assignment(assignments)?;
    Ok(())
}

pub fn split_person(
    people: &mut Vec<PersonRecord>,
    assignments: &mut Vec<FaceAssignment>,
    source_person_id: &str,
    face_ids: &[String],
    new_name: Option<&str>,
) -> Result<String, String> {
    if face_ids.is_empty() {
        return Err("no faces selected for split".to_string());
    }
    if !people.iter().any(|person| person.id == source_person_id) {
        return Err(format!("source person {source_person_id} not found"));
    }

    let selected: HashSet<_> = face_ids.iter().cloned().collect();
    for face_id in &selected {
        let Some(assignment) = assignments.iter().find(|assignment| &assignment.face_id == face_id)
        else {
            return Err(format!("face {face_id} is not assigned"));
        };
        if assignment.person_id != source_person_id {
            return Err(format!("face {face_id} does not belong to source person"));
        }
    }

    let new_person_id = Uuid::new_v4().to_string();
    people.push(PersonRecord {
        id: new_person_id.clone(),
        name: new_name
            .map(normalize_person_name)
            .transpose()?
            .unwrap_or_default(),
        hidden: false,
        cover_face_id: face_ids.first().cloned(),
    });

    for assignment in assignments.iter_mut() {
        if selected.contains(&assignment.face_id) {
            assignment.person_id = new_person_id.clone();
            assignment.source = AssignmentSource::ManualSplit;
        }
    }

    validate_single_assignment(assignments)?;
    Ok(new_person_id)
}

pub fn rebuild_people_assignments(
    faces: &[ClusterFace],
    previous_people: Vec<PersonRecord>,
    previous_assignments: Vec<FaceAssignment>,
    eps: f32,
    min_samples: usize,
) -> (Vec<PersonRecord>, Vec<FaceAssignment>) {
    let face_ids: HashSet<_> = faces.iter().map(|face| face.face_id.clone()).collect();

    let mut assignments: Vec<FaceAssignment> = previous_assignments
        .into_iter()
        .filter(|assignment| {
            face_ids.contains(&assignment.face_id) && assignment.source.is_manual()
        })
        .collect();

    let assigned_faces: HashSet<_> = assignments
        .iter()
        .map(|assignment| assignment.face_id.clone())
        .collect();
    let unassigned_faces = faces
        .iter()
        .filter(|face| !assigned_faces.contains(&face.face_id))
        .cloned()
        .collect::<Vec<_>>();

    let clustered = cluster_faces(&unassigned_faces, eps, min_samples);
    let mut people: HashMap<String, PersonRecord> = previous_people
        .into_iter()
        .filter(|person| {
            assignments
                .iter()
                .any(|assignment| assignment.person_id == person.id)
        })
        .map(|person| (person.id.clone(), person))
        .collect();

    for (face_id, person_id) in clustered {
        people.entry(person_id.clone()).or_insert_with(|| PersonRecord {
            id: person_id.clone(),
            name: String::new(),
            hidden: false,
            cover_face_id: Some(face_id.clone()),
        });
        assignments.push(FaceAssignment {
            face_id,
            person_id,
            source: AssignmentSource::Automatic,
        });
    }

    let mut people = people.into_values().collect::<Vec<_>>();
    people.sort_by(|left, right| left.id.cmp(&right.id));
    assignments.sort_by(|left, right| left.face_id.cmp(&right.face_id));
    let _ = validate_single_assignment(&assignments);
    (people, assignments)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_vector(first: f32, second: f32) -> Vec<f32> {
        let norm = (first * first + second * second).sqrt();
        vec![first / norm, second / norm]
    }

    #[test]
    fn dbscan_groups_similar_faces() {
        let faces = vec![
            ClusterFace {
                face_id: "a".to_string(),
                embedding: unit_vector(1.0, 0.0),
            },
            ClusterFace {
                face_id: "b".to_string(),
                embedding: unit_vector(0.99, 0.01),
            },
            ClusterFace {
                face_id: "c".to_string(),
                embedding: unit_vector(0.0, 1.0),
            },
        ];
        let clustered = cluster_faces(&faces, 0.2, 2);
        assert_eq!(clustered.len(), 2);
        assert_eq!(clustered.get("a"), clustered.get("b"));
        assert_ne!(clustered.get("a"), clustered.get("c"));
    }

    #[test]
    fn rebuild_preserves_manual_assignments() {
        let faces = vec![
            ClusterFace {
                face_id: "a".to_string(),
                embedding: unit_vector(1.0, 0.0),
            },
            ClusterFace {
                face_id: "b".to_string(),
                embedding: unit_vector(0.0, 1.0),
            },
        ];
        let people = vec![PersonRecord {
            id: "p1".to_string(),
            name: "Alice".to_string(),
            hidden: false,
            cover_face_id: Some("a".to_string()),
        }];
        let assignments = vec![FaceAssignment {
            face_id: "a".to_string(),
            person_id: "p1".to_string(),
            source: AssignmentSource::ManualName,
        }];
        let (people, assignments) =
            rebuild_people_assignments(&faces, people, assignments, 0.2, 1);
        assert!(people.iter().any(|person| person.id == "p1"));
        assert!(assignments.iter().any(|assignment| {
            assignment.face_id == "a" && assignment.person_id == "p1"
        }));
        assert!(assignments.iter().any(|assignment| assignment.face_id == "b"));
    }
}
