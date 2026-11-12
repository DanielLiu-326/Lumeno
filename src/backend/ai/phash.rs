//! Shared identity and content signature for the per-photo DCT pHash cache.
//!
//! The duplicate detector and burst stacking must derive the same cache key.
//! Keep these formulas here instead of copying them into each pipeline.

use sha2::{Digest, Sha256};

pub(crate) fn build_photo_content_signature(
    size_bytes: u64,
    modified_unix_seconds: Option<u64>,
) -> String {
    match modified_unix_seconds {
        Some(modified) => format!("{size_bytes}:{modified}"),
        None => format!("{size_bytes}:unknown"),
    }
}

pub(crate) fn build_photo_stable_identity(
    owner: &str,
    source_key: &str,
    path: &str,
) -> (String, String) {
    let stable_key = format!("{owner}|{source_key}|{path}");
    let mut hasher = Sha256::new();
    hasher.update(stable_key.as_bytes());
    let stable_id = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    (stable_key, stable_id)
}

#[cfg(test)]
mod tests {
    use super::{build_photo_content_signature, build_photo_stable_identity};

    #[test]
    fn content_signature_is_stable_without_mtime() {
        assert_eq!(build_photo_content_signature(10, None), "10:unknown");
        assert_eq!(build_photo_content_signature(10, Some(20)), "10:20");
    }

    #[test]
    fn stable_identity_includes_owner_album_and_path() {
        let (key, id) = build_photo_stable_identity("daniel", "trip", "albums/trip/a.jpg");
        assert_eq!(key, "daniel|trip|albums/trip/a.jpg");
        assert_eq!(id.len(), 64);
    }
}
