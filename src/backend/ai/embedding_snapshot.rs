//! Read-only embedding snapshot (`embedding_index.json` + `embedding_vectors.bin`)
//! built from per-photo JSON caches after AI indexing. Not source of truth.

use serde::{Deserialize, Serialize};
use std::path::Path;

use super::album_cache::{AlbumAiCache, EmbeddingCacheFile};

pub const EMBEDDING_SNAPSHOT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingSnapshotEntry {
    pub stable_key: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingSnapshotIndex {
    pub version: u32,
    pub generation: String,
    pub dimension: usize,
    pub count: usize,
    pub vectors_bytes: u64,
    pub entries: Vec<EmbeddingSnapshotEntry>,
}

#[derive(Debug, Clone)]
pub struct EmbeddingSnapshotLoaded {
    pub generation: String,
    pub items: Vec<(EmbeddingSnapshotEntry, Vec<f32>)>,
}

impl AlbumAiCache {
    pub fn embedding_snapshot_index_path(&self) -> std::path::PathBuf {
        self.root.join("embedding_index.json")
    }

    pub fn embedding_snapshot_vectors_path(&self) -> std::path::PathBuf {
        self.root.join("embedding_vectors.bin")
    }

    /// Build snapshot from per-photo `embeddings/*.json`. Safe to fail; JSON remains canonical.
    pub async fn build_embedding_snapshot_async(&self, generation: &str) -> Result<(), String> {
        let emb_files = self.list_embeddings_async().await?;
        if emb_files.is_empty() {
            let _ = tokio::fs::remove_file(self.embedding_snapshot_index_path()).await;
            let _ = tokio::fs::remove_file(self.embedding_snapshot_vectors_path()).await;
            return Ok(());
        }

        let mut unique = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for (_key, file) in emb_files {
            if !seen.insert(file.stable_key.clone()) {
                continue;
            }
            unique.push(file);
        }
        unique.sort_by(|a, b| a.stable_key.cmp(&b.stable_key));

        let mut dimension = 0usize;
        let mut entries = Vec::with_capacity(unique.len());
        let mut vector_bytes = Vec::new();
        for file in unique {
            let vector = file.decode_vector()?;
            if dimension == 0 {
                dimension = vector.len();
            }
            if vector.len() != dimension {
                return Err(format!(
                    "embedding snapshot dimension mismatch for {}: got {} expected {dimension}",
                    file.stable_key,
                    vector.len()
                ));
            }
            for value in &vector {
                if !value.is_finite() {
                    return Err(format!(
                        "embedding snapshot non-finite value for {}",
                        file.stable_key
                    ));
                }
            }
            entries.push(EmbeddingSnapshotEntry {
                stable_key: file.stable_key.clone(),
                source: file.source.clone(),
                provider_revision: file.provider_revision.clone(),
                content_signature: file.content_signature.clone(),
                width: file.width,
                height: file.height,
            });
            for value in vector {
                vector_bytes.extend_from_slice(&value.to_le_bytes());
            }
        }

        let index = EmbeddingSnapshotIndex {
            version: EMBEDDING_SNAPSHOT_VERSION,
            generation: generation.to_string(),
            dimension,
            count: entries.len(),
            vectors_bytes: vector_bytes.len() as u64,
            entries,
        };

        let index_bytes = serde_json::to_vec_pretty(&index)
            .map_err(|error| format!("serialize embedding_index.json: {error}"))?;
        let index_tmp = snapshot_tmp_path(self.embedding_snapshot_index_path().as_path());
        let vectors_tmp = snapshot_tmp_path(self.embedding_snapshot_vectors_path().as_path());
        snapshot_write_tmp(&index_tmp, &index_bytes).await?;
        snapshot_write_tmp(&vectors_tmp, &vector_bytes).await?;
        snapshot_commit_tmp(&index_tmp, self.embedding_snapshot_index_path().as_path()).await?;
        snapshot_commit_tmp(&vectors_tmp, self.embedding_snapshot_vectors_path().as_path()).await?;
        Ok(())
    }

    /// Load snapshot when present and consistent. Returns `None` → caller falls back to JSON files.
    pub async fn load_embedding_snapshot_async(
        &self,
        expected_generation: Option<&str>,
    ) -> Option<EmbeddingSnapshotLoaded> {
        let index_path = self.embedding_snapshot_index_path();
        let vectors_path = self.embedding_snapshot_vectors_path();
        let raw = tokio::fs::read_to_string(&index_path).await.ok()?;
        let index: EmbeddingSnapshotIndex = serde_json::from_str(&raw).ok()?;
        if index.version != EMBEDDING_SNAPSHOT_VERSION {
            return None;
        }
        if let Some(expected) = expected_generation {
            if index.generation != expected {
                return None;
            }
        }
        if index.dimension == 0 || index.count != index.entries.len() {
            return None;
        }
        let expected_bytes = index
            .count
            .checked_mul(index.dimension)
            .and_then(|n| n.checked_mul(4))?;
        if index.vectors_bytes as usize != expected_bytes {
            return None;
        }
        let bytes = tokio::fs::read(&vectors_path).await.ok()?;
        if bytes.len() != expected_bytes {
            return None;
        }

        let mut items = Vec::with_capacity(index.entries.len());
        for (entry_index, entry) in index.entries.into_iter().enumerate() {
            let start = entry_index
                .checked_mul(index.dimension)?
                .checked_mul(4)?;
            let end = start.checked_add(index.dimension.checked_mul(4)?)?;
            let chunk = bytes.get(start..end)?;
            let vector = decode_f32_le(chunk, index.dimension).ok()?;
            items.push((entry, vector));
        }

        Some(EmbeddingSnapshotLoaded {
            generation: index.generation,
            items,
        })
    }
}

fn decode_f32_le(bytes: &[u8], dimension: usize) -> Result<Vec<f32>, String> {
    if bytes.len() != dimension * 4 {
        return Err(format!(
            "unexpected vector byte length: {} (expected {})",
            bytes.len(),
            dimension * 4
        ));
    }
    let mut out = Vec::with_capacity(dimension);
    for chunk in bytes.chunks_exact(4) {
        let arr: [u8; 4] = chunk.try_into().unwrap();
        out.push(f32::from_le_bytes(arr));
    }
    Ok(out)
}

fn snapshot_tmp_path(path: &Path) -> std::path::PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("snapshot");
    path.with_file_name(format!(".{file_name}.tmp"))
}

async fn snapshot_write_tmp(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("no parent for {}", path.display()))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| format!("create parent dir: {error}"))?;
    tokio::fs::write(path, bytes)
        .await
        .map_err(|error| format!("write tmp {}: {error}", path.display()))
}

async fn snapshot_commit_tmp(tmp: &Path, dest: &Path) -> Result<(), String> {
    tokio::fs::rename(tmp, dest)
        .await
        .map_err(|error| format!("rename {} -> {}: {error}", tmp.display(), dest.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ai::album_cache::AlbumAiCache;

    #[tokio::test]
    async fn snapshot_roundtrip_from_json_caches() {
        let root = std::env::temp_dir().join(format!("album-emb-snap-{}", uuid::Uuid::new_v4()));
        let cache = AlbumAiCache::new(&root, "owner", "cache1");
        cache.ensure_dirs_async().await.unwrap();
        let vector = vec![0.1_f32, 0.2, 0.3, 0.4];
        let file = EmbeddingCacheFile::from_vector(
            "owner|album|albums/a/x.jpg",
            "x.jpg",
            &vector,
            Some("rev"),
            Some("123:456"),
        );
        cache
            .write_embedding_async("key1", &file)
            .await
            .expect("write embedding json");
        cache
            .build_embedding_snapshot_async("gen-test")
            .await
            .expect("build snapshot");
        let loaded = cache
            .load_embedding_snapshot_async(Some("gen-test"))
            .await
            .expect("load snapshot");
        assert_eq!(loaded.items.len(), 1);
        assert_eq!(loaded.items[0].1, vector);
        let _ = tokio::fs::remove_dir_all(&root).await;
    }
}
