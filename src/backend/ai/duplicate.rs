use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;

use image::DynamicImage;
use image::imageops::FilterType;

use crate::backend::album::decoder::{
    DecodeOptions, DecoderCreateOptions, create_decoder, decode_bytes_with_path,
};

use super::album_cache::{
    AlbumAiCache, DEFAULT_NEAR_DUPLICATE_DISTANCE, DUPLICATES_INDEX_VERSION, PhashCacheFile,
    phash_cache_key,
};
use super::models::AiIndexedPhotoRecord;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AiDuplicateGroup {
    pub(crate) group_key: String,
    pub(crate) kind: String,
    pub(crate) sha256: Option<String>,
    pub(crate) max_hamming_distance: Option<u32>,
    pub(crate) photo_count: usize,
    pub(crate) photos: Vec<AiDuplicatePhotoItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DuplicatesIndexFile {
    pub(crate) version: u32,
    pub(crate) max_distance: u32,
    pub(crate) algorithm: String,
    pub(crate) groups: Vec<AiDuplicateGroup>,
}

pub(crate) fn compute_photo_phash(path: &Path) -> Result<u64, String> {
    let decoder = create_decoder(DecoderCreateOptions::auto())
        .map_err(|error| format!("create decoder failed: {error}"))?;
    let bytes = std::fs::read(path).map_err(|error| format!("read photo failed: {error}"))?;
    let image = decode_bytes_with_path(decoder.as_ref(), path, &bytes, DecodeOptions::new())
        .map_err(|error| format!("decode photo for perceptual hash failed: {error}"))?;
    Ok(compute_photo_phash_image(&image))
}

/// Prefer an existing preview/thumbnail for pHash (32×32 sample). Falls back to original.
pub(crate) fn compute_photo_phash_preferring_preview(
    original: &Path,
    preview: Option<&Path>,
) -> Result<u64, String> {
    if let Some(preview) = preview
        && preview.exists()
    {
        let decoder = create_decoder(DecoderCreateOptions::auto())
            .map_err(|error| format!("create decoder failed: {error}"))?;
        if let Ok(bytes) = std::fs::read(preview)
            && let Ok(image) = decode_bytes_with_path(
                decoder.as_ref(),
                preview,
                &bytes,
                DecodeOptions::raw_pixels(),
            )
        {
            return Ok(compute_photo_phash_image(&image));
        }
    }
    compute_photo_phash(original)
}

fn compute_photo_phash_image(image: &DynamicImage) -> u64 {
    const SAMPLE_SIZE: usize = 32;
    const HASH_SIZE: usize = 8;

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
    hash
}

/// Perceptual hash for near-duplicate detection.
pub(crate) fn compute_duplicate_hash_pair(
    photo_path: &Path,
    preview: Option<&Path>,
) -> (String, Option<u64>, bool, Option<String>) {
    match compute_photo_phash_preferring_preview(photo_path, preview) {
        Ok(phash) => ("ready".to_string(), Some(phash), true, None),
        Err(error) => ("error".to_string(), None, false, Some(error)),
    }
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

/// Undirected adjacency for pairs with Hamming distance ≤ `max_distance`.
fn build_phash_adjacency(
    candidates: &[(&AiIndexedPhotoRecord, u64)],
    max_distance: u32,
) -> Vec<std::collections::HashSet<usize>> {
    let n = candidates.len();
    let mut neighbors = vec![std::collections::HashSet::<usize>::new(); n];
    let mut tree: Option<PhashBkNode> = None;

    for (index, (_, phash)) in candidates.iter().enumerate() {
        if let Some(root) = tree.as_ref() {
            let mut matches = Vec::new();
            root.search(*phash, max_distance, &mut matches);
            for matched_index in matches {
                if matched_index == index {
                    continue;
                }
                neighbors[index].insert(matched_index);
                neighbors[matched_index].insert(index);
            }
        }
        if let Some(root) = tree.as_mut() {
            root.insert(*phash, index);
        } else {
            tree = Some(PhashBkNode::new(*phash, index));
        }
    }
    neighbors
}

/// Bron–Kerbosch with pivoting: enumerate maximal cliques in an undirected graph.
fn bron_kerbosch_maximal_cliques(
    neighbors: &[std::collections::HashSet<usize>],
) -> Vec<Vec<usize>> {
    use std::collections::BTreeSet;

    let n = neighbors.len();
    let mut cliques = Vec::new();
    let mut r = Vec::new();
    let mut p: BTreeSet<usize> = (0..n).collect();
    let mut x: BTreeSet<usize> = BTreeSet::new();

    fn recurse(
        neighbors: &[std::collections::HashSet<usize>],
        r: &mut Vec<usize>,
        p: &mut BTreeSet<usize>,
        x: &mut BTreeSet<usize>,
        cliques: &mut Vec<Vec<usize>>,
    ) {
        if p.is_empty() && x.is_empty() {
            if r.len() >= 2 {
                let mut clique = r.clone();
                clique.sort_unstable();
                cliques.push(clique);
            }
            return;
        }

        let pivot = p
            .iter()
            .chain(x.iter())
            .max_by_key(|&&u| neighbors[u].iter().filter(|v| p.contains(v)).count())
            .copied();

        let candidates: Vec<usize> = match pivot {
            Some(u) => p
                .iter()
                .copied()
                .filter(|v| !neighbors[u].contains(v))
                .collect(),
            None => p.iter().copied().collect(),
        };

        for v in candidates {
            r.push(v);
            let n_v = &neighbors[v];
            let mut p_new: BTreeSet<usize> =
                p.iter().copied().filter(|u| n_v.contains(u)).collect();
            let mut x_new: BTreeSet<usize> =
                x.iter().copied().filter(|u| n_v.contains(u)).collect();
            recurse(neighbors, r, &mut p_new, &mut x_new, cliques);
            r.pop();
            p.remove(&v);
            x.insert(v);
        }
    }

    recurse(neighbors, &mut r, &mut p, &mut x, &mut cliques);
    cliques
}

/// Greedy set cover of vertices by largest maximal cliques (non-overlapping).
fn select_cliques_greedy(mut cliques: Vec<Vec<usize>>, vertex_count: usize) -> Vec<Vec<usize>> {
    cliques.sort_by(|left, right| right.len().cmp(&left.len()).then_with(|| left.cmp(right)));
    let mut used = vec![false; vertex_count];
    let mut selected = Vec::new();
    for clique in cliques {
        if clique.iter().any(|&v| used[v]) {
            continue;
        }
        for &v in &clique {
            used[v] = true;
        }
        selected.push(clique);
    }
    selected
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

pub(crate) async fn write_photo_phash_cache_async(
    ai_root: &Path,
    owner: &str,
    cache_id: &str,
    photo: &AiIndexedPhotoRecord,
) -> Result<(), String> {
    let Some(phash) = photo
        .duplicate_phash_u64
        .filter(|_| photo.duplicate_phash_valid)
    else {
        return Ok(());
    };
    let cache = AlbumAiCache::new(ai_root, owner, cache_id);
    let key = phash_cache_key(&photo.stable_key, &photo.content_signature);
    let file = PhashCacheFile::new(
        &photo.stable_key,
        &photo.album_relative_path(),
        phash,
        Some(photo.content_signature.as_str()),
    );
    cache.write_phash_async(&key, &file).await
}

pub(crate) async fn lookup_photo_phash_async(
    ai_root: &Path,
    owner: &str,
    cache_id: &str,
    stable_key: &str,
    content_signature: &str,
) -> Option<u64> {
    if cache_id.is_empty() || content_signature.is_empty() {
        return None;
    }
    let cache = AlbumAiCache::new(ai_root, owner, cache_id);
    let key = phash_cache_key(stable_key, content_signature);
    let file = cache.read_phash_async(&key).await?;
    if let Some(sig) = file.content_signature.as_deref()
        && sig != content_signature
    {
        return None;
    }
    Some(file.phash_u64)
}

pub(crate) async fn publish_duplicates_index_async(
    ai_root: &Path,
    owner: &str,
    cache_id: &str,
    photos: &[AiIndexedPhotoRecord],
    max_distance: u32,
) -> Result<usize, String> {
    let cache = AlbumAiCache::new(ai_root, owner, cache_id);
    let mut album_photos: Vec<_> = photos
        .iter()
        .filter(|p| p.cache_id == cache_id)
        .cloned()
        .collect();
    for photo in &mut album_photos {
        if photo.duplicate_phash_valid && photo.duplicate_phash_u64.is_some() {
            if let Err(error) = write_photo_phash_cache_async(ai_root, owner, cache_id, photo).await
            {
                eprintln!("phash cache write failed for {}: {error}", photo.stable_key);
            }
            continue;
        }
        if let Some(phash) = lookup_photo_phash_async(
            ai_root,
            owner,
            cache_id,
            &photo.stable_key,
            &photo.content_signature,
        )
        .await
        {
            photo.duplicate_phash_u64 = Some(phash);
            photo.duplicate_phash_valid = true;
            photo.duplicate_hash_state = "ready".to_string();
        }
    }
    let groups = build_near_duplicate_groups(&album_photos, max_distance);
    let index = DuplicatesIndexFile {
        version: DUPLICATES_INDEX_VERSION,
        max_distance,
        algorithm: "bron-kerbosch".to_string(),
        groups,
    };
    let value =
        serde_json::to_value(&index).map_err(|e| format!("serialize duplicates index: {e}"))?;
    cache.write_duplicates_index_json_async(&value).await?;
    Ok(index.groups.len())
}

pub(crate) async fn read_duplicates_index_async(
    ai_root: &Path,
    owner: &str,
    cache_id: &str,
) -> Option<DuplicatesIndexFile> {
    let cache = AlbumAiCache::new(ai_root, owner, cache_id);
    let value = cache.read_duplicates_index_json_async().await?;
    serde_json::from_value(value).ok()
}

pub(crate) fn default_near_duplicate_distance() -> u32 {
    DEFAULT_NEAR_DUPLICATE_DISTANCE
}

/// Near-duplicate groups via Bron–Kerbosch maximal cliques + greedy non-overlapping selection.
///
/// Graph edge = Hamming(pHash) ≤ `max_distance`. Each group is a true clique (every pair
/// within threshold), avoiding union-find transitive chaining.
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
    if candidates.len() < 2 {
        return Vec::new();
    }

    let neighbors = build_phash_adjacency(&candidates, max_distance);
    let cliques = bron_kerbosch_maximal_cliques(&neighbors);
    let selected = select_cliques_greedy(cliques, candidates.len());

    selected
        .into_iter()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn photo_with_phash(path: &str, phash: u64) -> AiIndexedPhotoRecord {
        let stable_key = format!("o|a|{path}");
        AiIndexedPhotoRecord {
            photo_id: 0,
            owner: "o".into(),
            album: "a".into(),
            source_key: "a".into(),
            cache_id: "a".into(),
            path: path.into(),
            stable_key: stable_key.clone(),
            stable_id: stable_key,
            size_bytes: 1,
            modified_unix_seconds: None,
            content_signature: String::new(),
            width: Some(1),
            height: Some(1),
            provider_revision: None,
            scan_state: "ready".into(),
            image_embedding_state: "ready".into(),
            duplicate_hash_state: "ready".into(),
            duplicate_phash_u64: Some(phash),
            duplicate_phash_valid: true,
            face_embedding_state: "ready".into(),
            reused_from_generation: None,
            record_error: None,
            indexed_at_unix_ms: 0,
        }
    }

    #[test]
    fn chain_does_not_form_one_clique() {
        // A--B--C with edges only A-B and B-C (distance 1), A-C distance 2 > threshold 1.
        let a = photo_with_phash("a.jpg", 0b000);
        let b = photo_with_phash("b.jpg", 0b001);
        let c = photo_with_phash("c.jpg", 0b011);
        let groups = build_near_duplicate_groups(&[a, b, c], 1);
        assert_eq!(
            groups.len(),
            1,
            "greedy keeps one edge-clique, leaves one vertex out"
        );
        assert_eq!(groups[0].photo_count, 2);
        assert!(groups[0].max_hamming_distance.unwrap() <= 1);
    }

    #[test]
    fn triangle_forms_one_clique() {
        // All pairs within distance 2 → one triangle (hypercube has no 3-cycles at dist 1).
        let a = photo_with_phash("a.jpg", 0b000);
        let b = photo_with_phash("b.jpg", 0b001);
        let c = photo_with_phash("c.jpg", 0b011);
        let groups = build_near_duplicate_groups(&[a, b, c], 2);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].photo_count, 3);
        assert!(groups[0].max_hamming_distance.unwrap() <= 2);
    }

    #[test]
    fn identical_phash_forms_one_group() {
        let a = photo_with_phash("a.jpg", 0xABCD);
        let b = photo_with_phash("b.jpg", 0xABCD);
        let c = photo_with_phash("c.jpg", 0xABCD);
        let groups = build_near_duplicate_groups(&[a, b, c], 8);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].photo_count, 3);
        assert_eq!(groups[0].max_hamming_distance, Some(0));
    }
}
