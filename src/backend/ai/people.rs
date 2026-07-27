use std::collections::HashMap;

use super::embedding::cosine_similarity;

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

/// DBSCAN clustering on cosine distance `1 - dot`. Returns face_id -> cluster key.
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
        assignments.insert(face.face_id.clone(), format!("cluster-{cluster}"));
    }
    assignments
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
        let groups = cluster_faces(&faces, 0.2, 2);
        assert_eq!(groups.get("a"), groups.get("b"));
        assert_ne!(groups.get("a"), groups.get("c"));
        assert!(!groups.contains_key("c"));
    }
}
