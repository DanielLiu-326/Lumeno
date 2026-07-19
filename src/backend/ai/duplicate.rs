use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use image::imageops::FilterType;

use super::models::AiIndexedPhotoRecord;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AiDuplicatePhotoItem {
    pub(crate) owner: String,
    pub(crate) album: String,
    pub(crate) source_key: String,
    pub(crate) path: String,
    pub(crate) stable_id: String,
    pub(crate) size_bytes: u64,
    pub(crate) modified_unix_seconds: Option<u64>,
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AiDuplicateGroup {
    pub(crate) group_key: String,
    pub(crate) kind: String,
    pub(crate) sha256: Option<String>,
    pub(crate) max_hamming_distance: Option<u32>,
    pub(crate) photo_count: usize,
    pub(crate) photos: Vec<AiDuplicatePhotoItem>,
}

pub(crate) fn compute_photo_phash(path: &Path) -> Result<u64, String> {
    const SAMPLE_SIZE: usize = 32;
    const HASH_SIZE: usize = 8;

    let image = image::open(path)
        .map_err(|error| format!("decode photo for perceptual hash failed: {error}"))?;
    let grayscale = image
        .grayscale()
        .resize_exact(SAMPLE_SIZE as u32, SAMPLE_SIZE as u32, FilterType::Triangle)
        .to_luma8();
    let pixels = grayscale
        .pixels()
        .map(|pixel| f64::from(pixel[0]))
        .collect::<Vec<_>>();
    let mut coefficients = [0_f64; HASH_SIZE * HASH_SIZE];

    for frequency_y in 0..HASH_SIZE {
        for frequency_x in 0..HASH_SIZE {
            let mut sum = 0_f64;
            for sample_y in 0..SAMPLE_SIZE {
                let cosine_y =
                    ((2 * sample_y + 1) as f64 * frequency_y as f64 * std::f64::consts::PI
                        / (2 * SAMPLE_SIZE) as f64)
                        .cos();
                for sample_x in 0..SAMPLE_SIZE {
                    let cosine_x =
                        ((2 * sample_x + 1) as f64 * frequency_x as f64 * std::f64::consts::PI
                            / (2 * SAMPLE_SIZE) as f64)
                            .cos();
                    sum += pixels[sample_y * SAMPLE_SIZE + sample_x] * cosine_x * cosine_y;
                }
            }
            coefficients[frequency_y * HASH_SIZE + frequency_x] = sum;
        }
    }

    let mut comparison = coefficients[1..].to_vec();
    comparison.sort_by(f64::total_cmp);
    let median = comparison[comparison.len() / 2];
    let mut hash = 0_u64;
    for (index, coefficient) in coefficients.iter().enumerate() {
        if *coefficient > median {
            hash |= 1_u64 << index;
        }
    }
    Ok(hash)
}

pub(crate) fn compute_photo_sha256(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path)
        .map_err(|error| format!("open photo for sha256 failed: {error}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 1024];

    loop {
        let bytes_read = file
            .read(&mut buffer)
            .map_err(|error| format!("read photo for sha256 failed: {error}"))?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>())
}

pub(crate) fn decode_sha256_hex(value: &str) -> Option<[u8; 32]> {
    if value.len() != 64 {
        return None;
    }
    let mut bytes = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let hex = std::str::from_utf8(chunk).ok()?;
        bytes[index] = u8::from_str_radix(hex, 16).ok()?;
    }
    Some(bytes)
}

struct PhashBkNode {
    hash: u64,
    photo_index: usize,
    children: HashMap<u32, Box<PhashBkNode>>,
}

impl PhashBkNode {
    fn new(hash: u64, photo_index: usize) -> Self {
        Self {
            hash,
            photo_index,
            children: HashMap::new(),
        }
    }

    fn insert(&mut self, hash: u64, photo_index: usize) {
        let distance = (self.hash ^ hash).count_ones();
        if let Some(child) = self.children.get_mut(&distance) {
            child.insert(hash, photo_index);
        } else {
            self.children
                .insert(distance, Box::new(Self::new(hash, photo_index)));
        }
    }

    fn search(&self, hash: u64, max_distance: u32, matches: &mut Vec<usize>) {
        let distance = (self.hash ^ hash).count_ones();
        if distance <= max_distance {
            matches.push(self.photo_index);
        }
        let lower = distance.saturating_sub(max_distance);
        let upper = distance.saturating_add(max_distance);
        for child_distance in lower..=upper {
            if let Some(child) = self.children.get(&child_distance) {
                child.search(hash, max_distance, matches);
            }
        }
    }
}

struct DuplicateUnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl DuplicateUnionFind {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
            rank: vec![0; len],
        }
    }

    fn find(&mut self, index: usize) -> usize {
        if self.parent[index] != index {
            self.parent[index] = self.find(self.parent[index]);
        }
        self.parent[index]
    }

    fn union(&mut self, left: usize, right: usize) {
        let left_root = self.find(left);
        let right_root = self.find(right);
        if left_root == right_root {
            return;
        }
        if self.rank[left_root] < self.rank[right_root] {
            self.parent[left_root] = right_root;
        } else if self.rank[left_root] > self.rank[right_root] {
            self.parent[right_root] = left_root;
        } else {
            self.parent[right_root] = left_root;
            self.rank[left_root] += 1;
        }
    }
}

fn duplicate_photo_item(record: &AiIndexedPhotoRecord) -> AiDuplicatePhotoItem {
    AiDuplicatePhotoItem {
        owner: record.owner.clone(),
        album: record.album.clone(),
        source_key: record.source_key.clone(),
        path: record.path.clone(),
        stable_id: record.stable_id.clone(),
        size_bytes: record.size_bytes,
        modified_unix_seconds: record.modified_unix_seconds,
        width: record.width,
        height: record.height,
    }
}

fn sort_duplicate_photos(photos: &mut [AiDuplicatePhotoItem]) {
    photos.sort_by(|left, right| {
        left.source_key
            .cmp(&right.source_key)
            .then(left.path.cmp(&right.path))
    });
}

pub(crate) fn build_exact_duplicate_groups(records: &[AiIndexedPhotoRecord]) -> Vec<AiDuplicateGroup> {
    let mut grouped = HashMap::<String, Vec<AiDuplicatePhotoItem>>::new();
    for record in records {
        let Some(sha256) = record.duplicate_sha256_hex.as_ref() else {
            continue;
        };
        grouped
            .entry(sha256.clone())
            .or_default()
            .push(duplicate_photo_item(record));
    }

    grouped
        .into_iter()
        .filter_map(|(sha256, mut photos)| {
            if photos.len() < 2 {
                return None;
            }
            sort_duplicate_photos(&mut photos);
            Some(AiDuplicateGroup {
                group_key: format!("exact:{sha256}"),
                kind: "exact".to_string(),
                sha256: Some(sha256),
                max_hamming_distance: Some(0),
                photo_count: photos.len(),
                photos,
            })
        })
        .collect()
}

pub(crate) fn build_near_duplicate_groups(
    records: &[AiIndexedPhotoRecord],
    max_distance: u32,
) -> Vec<AiDuplicateGroup> {
    let candidates = records
        .iter()
        .filter_map(|record| {
            record
                .duplicate_phash_u64
                .filter(|_| record.duplicate_phash_valid)
                .map(|phash| (record, phash))
        })
        .collect::<Vec<_>>();
    let mut union_find = DuplicateUnionFind::new(candidates.len());
    let mut tree: Option<PhashBkNode> = None;

    for (index, (record, phash)) in candidates.iter().enumerate() {
        if let Some(root) = tree.as_ref() {
            let mut matches = Vec::new();
            root.search(*phash, max_distance, &mut matches);
            for matched_index in matches {
                let matched_record = candidates[matched_index].0;
                if record.duplicate_sha256_hex != matched_record.duplicate_sha256_hex {
                    union_find.union(index, matched_index);
                }
            }
        }
        if let Some(root) = tree.as_mut() {
            root.insert(*phash, index);
        } else {
            tree = Some(PhashBkNode::new(*phash, index));
        }
    }

    let mut components = HashMap::<usize, Vec<usize>>::new();
    for index in 0..candidates.len() {
        let root = union_find.find(index);
        components.entry(root).or_default().push(index);
    }

    components
        .into_values()
        .filter_map(|indices| {
            if indices.len() < 2 {
                return None;
            }
            let mut max_observed_distance = 0_u32;
            for (position, left_index) in indices.iter().enumerate() {
                for right_index in indices.iter().skip(position + 1) {
                    let distance =
                        (candidates[*left_index].1 ^ candidates[*right_index].1).count_ones();
                    max_observed_distance = max_observed_distance.max(distance);
                }
            }
            let mut photos = indices
                .iter()
                .map(|index| duplicate_photo_item(candidates[*index].0))
                .collect::<Vec<_>>();
            sort_duplicate_photos(&mut photos);
            let mut key_hasher = Sha256::new();
            for photo in &photos {
                key_hasher.update(photo.stable_id.as_bytes());
            }
            let group_key = key_hasher
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            Some(AiDuplicateGroup {
                group_key: format!("near:{group_key}"),
                kind: "near".to_string(),
                sha256: None,
                max_hamming_distance: Some(max_observed_distance),
                photo_count: photos.len(),
                photos,
            })
        })
        .collect()
}

