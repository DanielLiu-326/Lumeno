pub const ACCENT_PRESET_IDS: &[&str] = &[
    "amber", "orange", "rose", "violet", "sky", "teal", "emerald", "blue",
];

pub fn default_accent() -> String {
    "amber".to_owned()
}

pub fn normalize_accent(value: &str) -> String {
    let trimmed = value.trim();
    let lower = trimmed.to_ascii_lowercase();
    if ACCENT_PRESET_IDS.contains(&lower.as_str()) {
        return lower;
    }
    normalize_hex(trimmed).unwrap_or_else(default_accent)
}

fn normalize_hex(value: &str) -> Option<String> {
    let hex = value.strip_prefix('#').unwrap_or(value);
    let valid = match hex.len() {
        3 | 6 => hex.chars().all(|ch| ch.is_ascii_hexdigit()),
        _ => false,
    };
    if !valid {
        return None;
    }
    Some(format!("#{}", hex.to_ascii_lowercase()))
}
