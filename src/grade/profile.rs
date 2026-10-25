//! Imported camera profiles. A profile is a matrix from linear camera RGB to XYZ D50.
//! ICC matrix tags and a small JSON document are accepted. The bytes are not stored.

use super::color::rec2020_from_camera_xyz_d50;

const MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug)]
pub struct ImportedProfile {
    pub name: String,
    pub to_working: [f32; 9],
}

#[derive(Debug)]
pub enum ProfileError {
    Invalid,
}

pub fn parse_camera_profile(
    bytes: &[u8],
    fallback_name: &str,
) -> Result<ImportedProfile, ProfileError> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(ProfileError::Invalid);
    }
    if let Some(matrix) = icc_camera_to_xyz_d50(bytes) {
        return Ok(ImportedProfile {
            name: profile_name(fallback_name),
            to_working: rec2020_from_camera_xyz_d50(matrix),
        });
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ProfileError::Invalid)?;
    let value: serde_json::Value = serde_json::from_str(text).map_err(|_| ProfileError::Invalid)?;
    let obj = value.as_object().ok_or(ProfileError::Invalid)?;
    let matrix = matrix9(obj.get("camera_to_xyz_d50").ok_or(ProfileError::Invalid)?)?;
    let name = obj
        .get("name")
        .and_then(|item| item.as_str())
        .unwrap_or(fallback_name);
    Ok(ImportedProfile {
        name: profile_name(name),
        to_working: rec2020_from_camera_xyz_d50(matrix),
    })
}

pub fn profile_name(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if ch.is_control() || ch == '/' || ch == '\\' {
            continue;
        }
        out.push(ch);
        if out.chars().count() >= 64 {
            break;
        }
    }
    let out = out.trim().trim_matches('.').to_string();
    if out.is_empty() {
        "profile".to_string()
    } else {
        out
    }
}

fn matrix9(value: &serde_json::Value) -> Result<[f32; 9], ProfileError> {
    let items = value.as_array().ok_or(ProfileError::Invalid)?;
    if items.len() != 9 {
        return Err(ProfileError::Invalid);
    }
    let mut out = [0f32; 9];
    for (slot, item) in out.iter_mut().zip(items) {
        let number = item.as_f64().ok_or(ProfileError::Invalid)? as f32;
        if !number.is_finite() || number.abs() > 16.0 {
            return Err(ProfileError::Invalid);
        }
        *slot = number;
    }
    Ok(out)
}

fn icc_camera_to_xyz_d50(bytes: &[u8]) -> Option<[f32; 9]> {
    if bytes.len() < 132 || &bytes[36..40] != b"acsp" {
        return None;
    }
    let space = &bytes[16..20];
    let pcs = &bytes[20..24];
    if space != b"RGB " || pcs != b"XYZ " {
        return None;
    }
    let class = &bytes[12..16];
    if class != b"mntr" && class != b"scnr" && class != b"spac" {
        return None;
    }
    let count = be_u32(bytes, 128)? as usize;
    if count > 64 || 132 + count * 12 > bytes.len() {
        return None;
    }
    let mut columns = [None; 3];
    for index in 0..count {
        let at = 132 + index * 12;
        let sig = bytes.get(at..at + 4)?;
        let offset = be_u32(bytes, at + 4)? as usize;
        let size = be_u32(bytes, at + 8)? as usize;
        let column = match sig {
            b"rXYZ" => 0,
            b"gXYZ" => 1,
            b"bXYZ" => 2,
            _ => continue,
        };
        if size < 20 || offset.checked_add(size)? > bytes.len() {
            return None;
        }
        if &bytes[offset..offset + 4] != b"XYZ " {
            return None;
        }
        columns[column] = Some([
            s15fixed16(bytes, offset + 8)?,
            s15fixed16(bytes, offset + 12)?,
            s15fixed16(bytes, offset + 16)?,
        ]);
    }
    let [r, g, b] = [columns[0]?, columns[1]?, columns[2]?];
    Some([r[0], g[0], b[0], r[1], g[1], b[1], r[2], g[2], b[2]])
}

fn be_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let chunk = bytes.get(at..at + 4)?;
    Some(u32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
}

fn s15fixed16(bytes: &[u8], at: usize) -> Option<f32> {
    let raw = be_u32(bytes, at)? as i32;
    let value = raw as f32 / 65536.0;
    if value.is_finite() && value.abs() <= 16.0 {
        Some(value)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_profile_becomes_a_working_matrix() {
        let bytes =
            br#"{"name":"Desk","camera_to_xyz_d50":[0.4,0.3,0.2, 0.2,0.7,0.1, 0.0,0.1,0.8]}"#;
        let profile = parse_camera_profile(bytes, "ignored").unwrap();
        assert_eq!(profile.name, "Desk");
        assert!(profile.to_working.iter().all(|n| n.is_finite()));
        assert!(profile.to_working.iter().any(|n| n.abs() > 0.01));
    }

    #[test]
    fn name_drops_path_pieces() {
        assert_eq!(profile_name("../camera/../Z6.icc"), "camera..Z6.icc");
        assert_eq!(profile_name(".."), "profile");
    }

    #[test]
    fn icc_matrix_profile_is_read() {
        let mut bytes = vec![0u8; 132 + 36 + 20 * 3];
        bytes[12..16].copy_from_slice(b"mntr");
        bytes[16..20].copy_from_slice(b"RGB ");
        bytes[20..24].copy_from_slice(b"XYZ ");
        bytes[36..40].copy_from_slice(b"acsp");
        bytes[128..132].copy_from_slice(&3u32.to_be_bytes());
        let tags = [(b"rXYZ", 0u8), (b"gXYZ", 1u8), (b"bXYZ", 2u8)];
        for (index, (sig, column)) in tags.iter().enumerate() {
            let entry = 132 + index * 12;
            bytes[entry..entry + 4].copy_from_slice(*sig);
            let offset = 132 + 36 + (*column as usize) * 20;
            bytes[entry + 4..entry + 8].copy_from_slice(&(offset as u32).to_be_bytes());
            bytes[entry + 8..entry + 12].copy_from_slice(&20u32.to_be_bytes());
            bytes[offset..offset + 4].copy_from_slice(b"XYZ ");
            // X = column + 1, so the matrix columns are distinguishable.
            let x = (((*column as i32) + 1) * 65536).to_be_bytes();
            bytes[offset + 8..offset + 12].copy_from_slice(&x);
        }
        let profile = parse_camera_profile(&bytes, "body.icc").unwrap();
        assert_eq!(profile.name, "body.icc");
        assert!(profile.to_working.iter().all(|n| n.is_finite()));
    }

    #[test]
    fn cmyk_profile_is_rejected() {
        let mut bytes = vec![0u8; 132];
        bytes[16..20].copy_from_slice(b"CMYK");
        bytes[20..24].copy_from_slice(b"XYZ ");
        bytes[36..40].copy_from_slice(b"acsp");
        assert!(parse_camera_profile(&bytes, "nope").is_err());
    }
}
