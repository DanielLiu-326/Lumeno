//! Adobe `.cube` 3D LUT. The file is kept as the resource blob. A parsed table is cached by id
//! so the grade node can sample it without opening a path.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use super::error::GradeError;

/// A text cube larger than this is refused before the lattice is read.
pub const CUBE_MAX_BYTES: usize = 256 * 1024 * 1024;
pub const CUBE_EDGE_MIN: u32 = 2;
pub const CUBE_EDGE_MAX: u32 = 256;

pub struct ParsedCube {
    pub size: u32,
    pub domain_min: [f32; 3],
    pub domain_max: [f32; 3],
    /// `size³` RGB triplets, R index changes fastest.
    pub table: Vec<f32>,
}

impl ParsedCube {
    pub fn sample(&self, r: f32, g: f32, b: f32) -> [f32; 3] {
        let last = (self.size - 1) as f32;
        let pr = axis(r, self.domain_min[0], self.domain_max[0], last);
        let pg = axis(g, self.domain_min[1], self.domain_max[1], last);
        let pb = axis(b, self.domain_min[2], self.domain_max[2], last);
        let r0 = pr.floor() as u32;
        let g0 = pg.floor() as u32;
        let b0 = pb.floor() as u32;
        let r1 = (r0 + 1).min(self.size - 1);
        let g1 = (g0 + 1).min(self.size - 1);
        let b1 = (b0 + 1).min(self.size - 1);
        let fx = pr - r0 as f32;
        let fy = pg - g0 as f32;
        let fz = pb - b0 as f32;
        let c000 = self.at(r0, g0, b0);
        // Six tetrahedra. The diagonal from 000 to 111 is shared, so a gradient
        // does not crease across that diagonal the way trilinear does.
        if fx >= fy {
            if fy >= fz {
                tetra(
                    c000,
                    self.at(r1, g0, b0),
                    self.at(r1, g1, b0),
                    self.at(r1, g1, b1),
                    fx,
                    fy,
                    fz,
                )
            } else if fx >= fz {
                tetra(
                    c000,
                    self.at(r1, g0, b0),
                    self.at(r1, g0, b1),
                    self.at(r1, g1, b1),
                    fx,
                    fz,
                    fy,
                )
            } else {
                tetra(
                    c000,
                    self.at(r0, g0, b1),
                    self.at(r1, g0, b1),
                    self.at(r1, g1, b1),
                    fz,
                    fx,
                    fy,
                )
            }
        } else if fz >= fy {
            tetra(
                c000,
                self.at(r0, g0, b1),
                self.at(r0, g1, b1),
                self.at(r1, g1, b1),
                fz,
                fy,
                fx,
            )
        } else if fz >= fx {
            tetra(
                c000,
                self.at(r0, g1, b0),
                self.at(r0, g1, b1),
                self.at(r1, g1, b1),
                fy,
                fz,
                fx,
            )
        } else {
            tetra(
                c000,
                self.at(r0, g1, b0),
                self.at(r1, g1, b0),
                self.at(r1, g1, b1),
                fy,
                fx,
                fz,
            )
        }
    }

    fn at(&self, r: u32, g: u32, b: u32) -> [f32; 3] {
        let n = self.size as usize;
        let index = (r as usize + n * (g as usize + n * b as usize)) * 3;
        [
            self.table[index],
            self.table[index + 1],
            self.table[index + 2],
        ]
    }
}

/// Parsed cubes are large: a 256³ table is ~200 MB as `f32`. Keep a few of them,
/// never more than the byte budget, and drop the least recently used first.
const LUT_CACHE_MAX_ENTRIES: usize = 4;
const LUT_CACHE_MAX_BYTES: usize = 512 * 1024 * 1024;

struct CachedLut {
    lut: Arc<ParsedCube>,
    bytes: usize,
    last_used: std::time::Instant,
}

pub fn remember_lut(id: &str, lut: Arc<ParsedCube>) {
    if id.is_empty() {
        return;
    }
    let bytes = lut.table.len() * std::mem::size_of::<f32>();
    let mut map = cache().lock().unwrap_or_else(|err| err.into_inner());
    map.insert(
        id.to_string(),
        CachedLut {
            lut,
            bytes,
            last_used: std::time::Instant::now(),
        },
    );
    loop {
        let total: usize = map.values().map(|entry| entry.bytes).sum();
        if map.len() <= LUT_CACHE_MAX_ENTRIES && total <= LUT_CACHE_MAX_BYTES {
            break;
        }
        let victim = map
            .iter()
            .filter(|(key, _)| key.as_str() != id)
            .min_by_key(|(_, entry)| entry.last_used)
            .map(|(key, _)| key.clone());
        match victim {
            Some(key) => {
                map.remove(&key);
            }
            None => break,
        }
    }
}

pub fn lookup_lut(id: &str) -> Option<Arc<ParsedCube>> {
    let mut map = cache().lock().unwrap_or_else(|err| err.into_inner());
    let entry = map.get_mut(id)?;
    entry.last_used = std::time::Instant::now();
    Some(entry.lut.clone())
}

/// Drop a cube the app just deleted, so the parsed copy does not outlive the blob.
pub fn forget_lut(id: &str) {
    cache()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .remove(id);
}

fn cache() -> &'static Mutex<HashMap<String, CachedLut>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CachedLut>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn axis(value: f32, min: f32, max: f32, last: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    ((value - min) / (max - min) * last).clamp(0.0, last)
}

fn tetra(
    c0: [f32; 3],
    c1: [f32; 3],
    c2: [f32; 3],
    c3: [f32; 3],
    d1: f32,
    d2: f32,
    d3: f32,
) -> [f32; 3] {
    [
        c0[0] + d1 * (c1[0] - c0[0]) + d2 * (c2[0] - c1[0]) + d3 * (c3[0] - c2[0]),
        c0[1] + d1 * (c1[1] - c0[1]) + d2 * (c2[1] - c1[1]) + d3 * (c3[1] - c2[1]),
        c0[2] + d1 * (c1[2] - c0[2]) + d2 * (c2[2] - c1[2]) + d3 * (c3[2] - c2[2]),
    ]
}

pub fn parse_cube(bytes: &[u8]) -> Result<ParsedCube, GradeError> {
    check_cube_len(bytes.len())?;
    let text = std::str::from_utf8(bytes).map_err(|_| GradeError::Rejected("cube-invalid"))?;
    let mut size: Option<u32> = None;
    let mut domain_min = [0.0; 3];
    let mut domain_max = [1.0; 3];
    let mut table = Vec::new();
    let mut expected: Option<u64> = None;
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = keyword(line, "LUT_1D_SIZE") {
            let _ = rest;
            return Err(GradeError::Rejected("cube-invalid"));
        }
        if let Some(rest) = keyword(line, "LUT_3D_SIZE") {
            if size.is_some() || !table.is_empty() {
                return Err(GradeError::Rejected("cube-invalid"));
            }
            let edge = rest
                .trim()
                .parse::<u32>()
                .map_err(|_| GradeError::Rejected("cube-edge"))?;
            if !(CUBE_EDGE_MIN..=CUBE_EDGE_MAX).contains(&edge) {
                return Err(GradeError::Rejected("cube-edge"));
            }
            size = Some(edge);
            let count = u64::from(edge).pow(3);
            expected = Some(count);
            table.reserve((count as usize).saturating_mul(3));
            continue;
        }
        if let Some(rest) = keyword(line, "DOMAIN_MIN") {
            domain_min = domain(rest)?;
            continue;
        }
        if let Some(rest) = keyword(line, "DOMAIN_MAX") {
            domain_max = domain(rest)?;
            continue;
        }
        if keyword(line, "TITLE").is_some() {
            continue;
        }
        let Some(need) = expected else {
            return Err(GradeError::Rejected("cube-invalid"));
        };
        let mut parts = line.split_whitespace();
        let r = finite(parts.next())?;
        let g = finite(parts.next())?;
        let b = finite(parts.next())?;
        if parts.next().is_some() {
            return Err(GradeError::Rejected("cube-invalid"));
        }
        table.extend_from_slice(&[r, g, b]);
        if (table.len() as u64) / 3 > need {
            return Err(GradeError::Rejected("cube-invalid"));
        }
    }
    let Some(edge) = size else {
        return Err(GradeError::Rejected("cube-invalid"));
    };
    if (table.len() as u64) / 3 != expected.unwrap_or(0) {
        return Err(GradeError::Rejected("cube-invalid"));
    }
    for channel in 0..3 {
        if !domain_min[channel].is_finite()
            || !domain_max[channel].is_finite()
            || domain_min[channel] >= domain_max[channel]
        {
            return Err(GradeError::Rejected("cube-invalid"));
        }
    }
    Ok(ParsedCube {
        size: edge,
        domain_min,
        domain_max,
        table,
    })
}

fn check_cube_len(len: usize) -> Result<(), GradeError> {
    if len > CUBE_MAX_BYTES {
        Err(GradeError::Rejected("cube-size"))
    } else {
        Ok(())
    }
}

fn strip_comment(line: &str) -> &str {
    match line.find('#') {
        Some(index) => &line[..index],
        None => line,
    }
}

fn keyword<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let mut parts = line.splitn(2, char::is_whitespace);
    let head = parts.next()?;
    if !head.eq_ignore_ascii_case(name) {
        return None;
    }
    Some(parts.next().unwrap_or(""))
}

fn domain(rest: &str) -> Result<[f32; 3], GradeError> {
    let mut parts = rest.split_whitespace();
    let values = [
        finite(parts.next())?,
        finite(parts.next())?,
        finite(parts.next())?,
    ];
    if parts.next().is_some() {
        return Err(GradeError::Rejected("cube-invalid"));
    }
    Ok(values)
}

fn finite(token: Option<&str>) -> Result<f32, GradeError> {
    let value = token
        .ok_or(GradeError::Rejected("cube-invalid"))?
        .parse::<f32>()
        .map_err(|_| GradeError::Rejected("cube-invalid"))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(GradeError::Rejected("cube-invalid"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lattice(edge: u32) -> String {
        let mut text = format!("LUT_3D_SIZE {edge}\n");
        let n = edge.pow(3);
        for _ in 0..n {
            text.push_str("0 0 0\n");
        }
        text
    }

    #[test]
    fn accepts_an_edge_outside_the_old_whitelist() {
        let parsed = parse_cube(lattice(2).as_bytes()).unwrap();
        assert_eq!(parsed.size, 2);
        let parsed = parse_cube(lattice(32).as_bytes()).unwrap();
        assert_eq!(parsed.size, 32);
    }

    #[test]
    fn rejects_edges_outside_2_to_256() {
        assert!(matches!(
            parse_cube(b"LUT_3D_SIZE 1\n0 0 0\n"),
            Err(GradeError::Rejected("cube-edge"))
        ));
        assert!(matches!(
            parse_cube(b"LUT_3D_SIZE 257\n"),
            Err(GradeError::Rejected("cube-edge"))
        ));
    }

    #[test]
    fn rejects_a_file_over_256_mib() {
        assert_eq!(CUBE_MAX_BYTES, 256 * 1024 * 1024);
        assert!(matches!(
            check_cube_len(CUBE_MAX_BYTES + 1),
            Err(GradeError::Rejected("cube-size"))
        ));
        assert!(check_cube_len(CUBE_MAX_BYTES).is_ok());
    }

    #[test]
    fn a_solid_red_cube_samples_red() {
        let mut text = "LUT_3D_SIZE 2\n".to_string();
        for _ in 0..8 {
            text.push_str("1 0 0\n");
        }
        let lut = parse_cube(text.as_bytes()).unwrap();
        let [r, g, b] = lut.sample(0.2, 0.4, 0.8);
        assert!((r - 1.0).abs() < 1e-5);
        assert!(g.abs() < 1e-5 && b.abs() < 1e-5);
    }

    #[test]
    fn tetrahedral_identity_stays_on_the_input() {
        let mut text = "LUT_3D_SIZE 2\n".to_string();
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    text.push_str(&format!("{r} {g} {b}\n"));
                }
            }
        }
        let lut = parse_cube(text.as_bytes()).unwrap();
        let [r, g, b] = lut.sample(0.3, 0.55, 0.8);
        assert!((r - 0.3).abs() < 1e-5, "{r}");
        assert!((g - 0.55).abs() < 1e-5, "{g}");
        assert!((b - 0.8).abs() < 1e-5, "{b}");
    }
}
