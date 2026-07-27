pub use crate::backend::constants::MAX_AI_TEXT_QUERY_CHARS;

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
