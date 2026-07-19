use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelManifest {
    pub schema_version: u32,
    pub revision: String,
    pub image_text: ModelArtifactSpec,
    pub face_detector: ModelArtifactSpec,
    pub face_embedder: ModelArtifactSpec,
    #[serde(default)]
    pub tokenizer: Option<TokenizerArtifactSpec>,
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelArtifactSpec {
    pub id: String,
    pub revision: String,
    pub format: String,
    pub file: String,
    pub sha256: String,
    pub dimension: usize,
    #[serde(default)]
    pub input_size: Option<[u32; 2]>,
    #[serde(default)]
    pub extras: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenizerArtifactSpec {
    pub file: String,
    pub sha256: String,
    #[serde(default)]
    pub max_length: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct VerifiedModelManifest {
    pub root: PathBuf,
    pub manifest: ModelManifest,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("model manifest missing at {0}")]
    Missing(PathBuf),
    #[error("model manifest parse error: {0}")]
    Parse(String),
    #[error("model manifest schema_version {found} unsupported (expected {expected})")]
    Schema { found: u32, expected: u32 },
    #[error("model artifact missing: {0}")]
    ArtifactMissing(PathBuf),
    #[error("model artifact checksum mismatch for {path}: expected {expected}, got {actual}")]
    Checksum {
        path: PathBuf,
        expected: String,
        actual: String,
    },
    #[error("model artifact invalid: {0}")]
    Invalid(String),
}

impl ModelManifest {
    pub fn load_path(path: &Path) -> Result<Self, ManifestError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|_| ManifestError::Missing(path.to_path_buf()))?;
        let manifest: Self = serde_json::from_str(&raw)
            .map_err(|error| ManifestError::Parse(error.to_string()))?;
        if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(ManifestError::Schema {
                found: manifest.schema_version,
                expected: MANIFEST_SCHEMA_VERSION,
            });
        }
        Ok(manifest)
    }

    pub fn verify_in_dir(root: &Path) -> Result<VerifiedModelManifest, ManifestError> {
        let path = root.join("manifest.json");
        let manifest = Self::load_path(&path)?;
        manifest.validate_artifacts(root)?;
        Ok(VerifiedModelManifest {
            root: root.to_path_buf(),
            manifest,
        })
    }

    pub fn validate_artifacts(&self, root: &Path) -> Result<(), ManifestError> {
        self.validate_one(root, &self.image_text)?;
        self.validate_one(root, &self.face_detector)?;
        self.validate_one(root, &self.face_embedder)?;
        if let Some(tokenizer) = &self.tokenizer {
            let path = root.join(&tokenizer.file);
            verify_sha256(&path, &tokenizer.sha256)?;
        }
        if self.image_text.dimension == 0 || self.face_embedder.dimension == 0 {
            return Err(ManifestError::Invalid(
                "embedding dimension must be > 0".to_string(),
            ));
        }
        if !matches!(
            self.image_text.format.as_str(),
            "safetensors" | "burnpack" | "bpk"
        ) {
            return Err(ManifestError::Invalid(format!(
                "unsupported image_text format: {}",
                self.image_text.format
            )));
        }
        // Optional paired text burnpack for SigLIP (vision+text split export).
        if let Some(text_file) = self
            .image_text
            .extras
            .get("text_file")
            .and_then(|v| v.as_str())
        {
            let path = root.join(text_file);
            if let Some(expected) = self
                .image_text
                .extras
                .get("text_sha256")
                .and_then(|v| v.as_str())
            {
                verify_sha256(&path, expected)?;
            } else if !path.is_file() {
                return Err(ManifestError::ArtifactMissing(path));
            }
        }
        Ok(())
    }

    fn validate_one(&self, root: &Path, spec: &ModelArtifactSpec) -> Result<(), ManifestError> {
        let path = root.join(&spec.file);
        verify_sha256(&path, &spec.sha256)?;
        Ok(())
    }

    pub fn combined_revision(&self) -> String {
        format!(
            "{}+{}+{}",
            self.image_text.revision, self.face_detector.revision, self.face_embedder.revision
        )
    }
}

pub fn sha256_hex_file(path: &Path) -> Result<String, ManifestError> {
    let mut file =
        File::open(path).map_err(|_| ManifestError::ArtifactMissing(path.to_path_buf()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 1024 * 64];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| ManifestError::Invalid(format!("read {}: {error}", path.display())))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex_encode(hasher.finalize().as_slice()))
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), ManifestError> {
    if !path.is_file() {
        return Err(ManifestError::ArtifactMissing(path.to_path_buf()));
    }
    let actual = sha256_hex_file(path)?;
    let expected = expected.trim().to_ascii_lowercase();
    if actual != expected {
        return Err(ManifestError::Checksum {
            path: path.to_path_buf(),
            expected,
            actual,
        });
    }
    Ok(())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn verifies_checksum_and_rejects_mismatch() {
        let dir = tempfile_dir();
        let weights = dir.join("siglip2.safetensors");
        std::fs::write(&weights, b"fake-weights").unwrap();
        let digest = sha256_hex_file(&weights).unwrap();

        // Reuse one bytes file for all artifact paths (checksum plumbing smoke).
        let mut image = artifact("siglip2.safetensors", &digest, 768);
        image.file = "siglip2.safetensors".to_string();
        let mut det = artifact("scrfd.safetensors", &digest, 5);
        det.file = "siglip2.safetensors".to_string();
        let mut face = artifact("arcface.safetensors", &digest, 512);
        face.file = "siglip2.safetensors".to_string();
        let manifest = ModelManifest {
            schema_version: MANIFEST_SCHEMA_VERSION,
            revision: "test-rev".to_string(),
            image_text: image,
            face_detector: det,
            face_embedder: face,
            tokenizer: None,
            notes: None,
        };
        std::fs::write(
            dir.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();
        ModelManifest::verify_in_dir(&dir).unwrap();

        std::fs::write(&weights, b"tampered").unwrap();
        assert!(ModelManifest::verify_in_dir(&dir).is_err());
    }

    fn artifact(file: &str, sha: &str, dim: usize) -> ModelArtifactSpec {
        ModelArtifactSpec {
            id: file.to_string(),
            revision: "r1".to_string(),
            format: "safetensors".to_string(),
            file: file.to_string(),
            sha256: sha.to_string(),
            dimension: dim,
            input_size: Some([256, 256]),
            extras: HashMap::new(),
        }
    }

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("album-manifest-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = File::create(dir.join(".keep")).and_then(|mut f| f.write_all(b""));
        dir
    }
}
