use burn::backend::Flex;
use burn::prelude::*;
#[cfg(test)]
use half::f16;
use image::imageops::FilterType;
#[cfg(test)]
use std::path::Path;
use std::sync::OnceLock;

#[cfg(test)]
use crate::backend::album::decoder::{
    DecodeOptions, DecoderCreateOptions, create_decoder, decode_bytes_with_path,
};

pub const IMAGE_EMBEDDING_DIM: usize = 128;
pub const VISUAL_EMBED_REVISION: &str = "visual-hash-v2-burn";
pub const TEXT_HASH_REVISION: &str = "text-hash-v1";

/// Default interim embedding size (DCT / text-hash). Real models use manifest dims.
pub const INTERIM_EMBEDDING_DIM: usize = IMAGE_EMBEDDING_DIM;
pub const SIGLIP2_EMBEDDING_DIM: usize = 768;
pub const ARCFACE_EMBEDDING_DIM: usize = 512;

const SAMPLE_SIZE: usize = 32;
const DCT_SIZE: usize = 8;
const AC_COEFFS: usize = DCT_SIZE * DCT_SIZE - 1; // skip DC
const PIXEL_COUNT: usize = SAMPLE_SIZE * SAMPLE_SIZE;

type EmbedBackend = Flex;

/// Lowercase latin letters; leave CJK and other scripts unchanged.
pub fn normalize_search_text(text: &str) -> String {
    text.trim()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphabetic() {
                ch.to_ascii_lowercase()
            } else {
                ch
            }
        })
        .collect()
}

pub fn tokenize_search_text(text: &str) -> Vec<String> {
    let normalized = normalize_search_text(text);
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in normalized.chars() {
        if ch.is_whitespace() || matches!(ch, '/' | '\\' | '-' | '_' | '.' | ',') {
            if !current.is_empty() {
                tokens.push(current.clone());
                current.clear();
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    if tokens.is_empty() && !normalized.is_empty() {
        tokens.push(normalized);
    }
    tokens
}

/// Interim text vector in the same 128-dim space as image embeddings (hashing trick).
pub fn compute_text_hash_embedding(text: &str) -> Result<Vec<f32>, String> {
    let normalized = normalize_search_text(text);
    if normalized.is_empty() {
        return Err("search text is empty".to_string());
    }
    let mut vector = vec![0_f32; IMAGE_EMBEDDING_DIM];
    let chars: Vec<char> = normalized.chars().collect();
    let windows = if chars.len() >= 2 {
        chars
            .windows(2)
            .map(|w| w.iter().collect::<String>())
            .collect::<Vec<_>>()
    } else {
        vec![normalized.clone()]
    };
    for bigram in windows {
        let hash = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            use std::hash::{Hash, Hasher};
            bigram.hash(&mut hasher);
            hasher.finish()
        };
        let idx = (hash as usize) % IMAGE_EMBEDDING_DIM;
        let sign = if hash & 1 == 0 { 1.0_f32 } else { -1.0_f32 };
        vector[idx] += sign;
    }
    l2_normalize(&mut vector)?;
    Ok(vector)
}

pub fn l2_normalize(vector: &mut [f32]) -> Result<(), String> {
    let mut sum_sq = 0_f32;
    for value in vector.iter() {
        if !value.is_finite() {
            return Err("embedding contains non-finite values".to_string());
        }
        sum_sq += *value * *value;
    }
    if sum_sq <= f32::EPSILON {
        return Err("embedding norm is too small".to_string());
    }
    let inv_norm = sum_sq.sqrt().recip();
    for value in vector.iter_mut() {
        *value *= inv_norm;
    }
    Ok(())
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(a, b)| a * b)
        .sum::<f32>()
}

/// SigLIP2 text↔image cosines for true matches are typically ~0.04–0.15 (not CLIP-scale).
/// Stretch into `[0, 1]` so blended search scores and UI thresholds stay usable.
pub const SIGLIP_TEXT_IMAGE_COSINE_SCALE: f32 = 0.12;

pub fn calibrate_siglip_text_image_score(cosine: f32) -> f32 {
    (cosine / SIGLIP_TEXT_IMAGE_COSINE_SCALE).clamp(0.0, 1.0)
}

#[cfg(test)]
pub fn encode_f16_vector(vector: &[f32]) -> Result<Vec<u8>, String> {
    encode_f16_vector_dim(vector, vector.len())
}

#[cfg(test)]
pub fn encode_f16_vector_dim(vector: &[f32], dim: usize) -> Result<Vec<u8>, String> {
    if dim == 0 {
        return Err("embedding dimension must be > 0".to_string());
    }
    if vector.len() != dim {
        return Err(format!(
            "unexpected embedding dimension: {} (expected {dim})",
            vector.len()
        ));
    }
    let mut bytes = Vec::with_capacity(dim * 2);
    for value in vector {
        if !value.is_finite() {
            return Err("embedding contains non-finite values".to_string());
        }
        bytes.extend_from_slice(&f16::from_f32(*value).to_le_bytes());
    }
    Ok(bytes)
}

#[cfg(test)]
pub fn decode_f16_vector(bytes: &[u8]) -> Result<Vec<f32>, String> {
    if !bytes.len().is_multiple_of(2) || bytes.is_empty() {
        return Err(format!(
            "unexpected vector byte length: {} (expected even non-zero)",
            bytes.len()
        ));
    }
    decode_f16_vector_dim(bytes, bytes.len() / 2)
}

#[cfg(test)]
pub fn decode_f16_vector_dim(bytes: &[u8], dim: usize) -> Result<Vec<f32>, String> {
    if dim == 0 {
        return Err("embedding dimension must be > 0".to_string());
    }
    if bytes.len() != dim * 2 {
        return Err(format!(
            "unexpected vector byte length: {} (expected {})",
            bytes.len(),
            dim * 2
        ));
    }
    let mut vector = Vec::with_capacity(dim);
    for chunk in bytes.chunks_exact(2) {
        let bits = u16::from_le_bytes([chunk[0], chunk[1]]);
        let value = f16::from_bits(bits).to_f32();
        if !value.is_finite() {
            return Err("decoded embedding contains non-finite values".to_string());
        }
        vector.push(value);
    }
    Ok(vector)
}

struct DctEngine {
    /// Precomputed DCT-II AC basis as [PIXEL_COUNT, AC_COEFFS], owned by Burn.
    basis: Tensor<EmbedBackend, 2>,
}

impl DctEngine {
    fn new() -> Self {
        let device = Default::default();
        let mut basis_host = vec![0_f32; PIXEL_COUNT * AC_COEFFS];
        let mut coeff_index = 0usize;
        for frequency_y in 0..DCT_SIZE {
            for frequency_x in 0..DCT_SIZE {
                if frequency_y == 0 && frequency_x == 0 {
                    continue;
                }
                for sample_y in 0..SAMPLE_SIZE {
                    let cosine_y =
                        ((2 * sample_y + 1) as f32 * frequency_y as f32 * std::f32::consts::PI
                            / (2 * SAMPLE_SIZE) as f32)
                            .cos();
                    for sample_x in 0..SAMPLE_SIZE {
                        let cosine_x =
                            ((2 * sample_x + 1) as f32 * frequency_x as f32 * std::f32::consts::PI
                                / (2 * SAMPLE_SIZE) as f32)
                                .cos();
                        let row = sample_y * SAMPLE_SIZE + sample_x;
                        basis_host[row * AC_COEFFS + coeff_index] =
                            cosine_x * cosine_y / PIXEL_COUNT as f32;
                    }
                }
                coeff_index += 1;
            }
        }
        let basis = Tensor::<EmbedBackend, 1>::from_floats(basis_host.as_slice(), &device)
            .reshape([PIXEL_COUNT, AC_COEFFS]);
        Self { basis }
    }

    fn dct_ac_batch(&self, luma_batch: &[f32], batch_size: usize) -> Result<Vec<f32>, String> {
        if luma_batch.len() != batch_size * PIXEL_COUNT {
            return Err(format!(
                "unexpected luma batch length: {} (expected {})",
                luma_batch.len(),
                batch_size * PIXEL_COUNT
            ));
        }
        if batch_size == 0 {
            return Ok(Vec::new());
        }
        let device = Default::default();
        let pixels = Tensor::<EmbedBackend, 1>::from_floats(luma_batch, &device)
            .reshape([batch_size, PIXEL_COUNT]);
        let coeffs = pixels.matmul(self.basis.clone()); // [B, AC]
        let data = coeffs.into_data();
        data.to_vec::<f32>()
            .map_err(|error| format!("read dct coefficients failed: {error}"))
    }
}

fn dct_engine() -> &'static DctEngine {
    static ENGINE: OnceLock<DctEngine> = OnceLock::new();
    ENGINE.get_or_init(DctEngine::new)
}

fn prepare_sample_rgb(rgb: &image::RgbImage) -> image::RgbImage {
    image::DynamicImage::ImageRgb8(rgb.clone())
        .resize_exact(SAMPLE_SIZE as u32, SAMPLE_SIZE as u32, FilterType::Triangle)
        .to_rgb8()
}

fn fill_luma_and_color(rgb: &image::RgbImage, luma_out: &mut [f32], color_out: &mut [f32]) {
    debug_assert_eq!(luma_out.len(), PIXEL_COUNT);
    debug_assert!(color_out.len() >= 48);

    for (index, pixel) in rgb.pixels().enumerate() {
        luma_out[index] =
            0.299 * f32::from(pixel[0]) + 0.587 * f32::from(pixel[1]) + 0.114 * f32::from(pixel[2]);
    }

    const GRID: usize = 4;
    let cell = SAMPLE_SIZE / GRID;
    let mut write_at = 0usize;
    for cell_y in 0..GRID {
        for cell_x in 0..GRID {
            let mut sum_r = 0_f32;
            let mut sum_g = 0_f32;
            let mut sum_b = 0_f32;
            let mut count = 0_f32;
            for sample_y in cell_y * cell..(cell_y + 1) * cell {
                for sample_x in cell_x * cell..(cell_x + 1) * cell {
                    let pixel = rgb.get_pixel(sample_x as u32, sample_y as u32);
                    sum_r += f32::from(pixel[0]);
                    sum_g += f32::from(pixel[1]);
                    sum_b += f32::from(pixel[2]);
                    count += 1.0;
                }
            }
            color_out[write_at] = (sum_r / count) / 255.0;
            color_out[write_at + 1] = (sum_g / count) / 255.0;
            color_out[write_at + 2] = (sum_b / count) / 255.0;
            write_at += 3;
        }
    }
}

fn assemble_embedding(ac_coeffs: &[f32], color: &[f32]) -> Result<Vec<f32>, String> {
    if ac_coeffs.len() != AC_COEFFS {
        return Err(format!(
            "unexpected AC coeff count: {} (expected {AC_COEFFS})",
            ac_coeffs.len()
        ));
    }
    let mut vector = vec![0_f32; IMAGE_EMBEDDING_DIM];
    vector[..AC_COEFFS].copy_from_slice(ac_coeffs);
    let color_len = 48.min(color.len());
    vector[AC_COEFFS..AC_COEFFS + color_len].copy_from_slice(&color[..color_len]);
    l2_normalize(&mut vector)?;
    Ok(vector)
}

/// Deterministic visual embedding accelerated with Burn Flex matmul.
/// Combines low-frequency DCT AC coefficients with spatial color averages.
pub fn compute_visual_embedding_rgb(rgb: &image::RgbImage) -> Result<Vec<f32>, String> {
    let sample = prepare_sample_rgb(rgb);
    let mut luma = [0_f32; PIXEL_COUNT];
    let mut color = [0_f32; 48];
    fill_luma_and_color(&sample, &mut luma, &mut color);
    let ac = dct_engine().dct_ac_batch(&luma, 1)?;
    assemble_embedding(&ac, &color)
}

#[cfg(test)]
pub fn compute_visual_embedding(path: &Path) -> Result<Vec<f32>, String> {
    let decoder = create_decoder(DecoderCreateOptions::auto())
        .map_err(|error| format!("create decoder failed: {error}"))?;
    let bytes = std::fs::read(path).map_err(|error| format!("read photo failed: {error}"))?;
    let image = decode_bytes_with_path(decoder.as_ref(), path, &bytes, DecodeOptions::new())
        .map_err(|error| format!("decode photo for visual embedding failed: {error}"))?;
    compute_visual_embedding_rgb(&image.to_rgb8())
}

/// Batch visual embeddings with one Burn matmul for the DCT stage.
#[cfg(test)]
pub fn compute_visual_embeddings_rgb_batch(
    images: &[&image::RgbImage],
) -> Result<Vec<Vec<f32>>, String> {
    if images.is_empty() {
        return Ok(Vec::new());
    }
    let batch_size = images.len();
    let mut luma_batch = vec![0_f32; batch_size * PIXEL_COUNT];
    let mut colors = vec![[0_f32; 48]; batch_size];
    for (batch_index, rgb) in images.iter().enumerate() {
        let sample = prepare_sample_rgb(rgb);
        let luma_slice =
            &mut luma_batch[batch_index * PIXEL_COUNT..(batch_index + 1) * PIXEL_COUNT];
        fill_luma_and_color(&sample, luma_slice, &mut colors[batch_index]);
    }
    let ac_batch = dct_engine().dct_ac_batch(&luma_batch, batch_size)?;
    let mut out = Vec::with_capacity(batch_size);
    for batch_index in 0..batch_size {
        let start = batch_index * AC_COEFFS;
        let end = start + AC_COEFFS;
        out.push(assemble_embedding(
            &ac_batch[start..end],
            &colors[batch_index],
        )?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    #[test]
    fn f16_roundtrip_variable_dimensions() {
        for dim in [128usize, 512, 768] {
            let mut vector = vec![0_f32; dim];
            for (index, value) in vector.iter_mut().enumerate() {
                *value = (index as f32 + 1.0).sin();
            }
            l2_normalize(&mut vector).unwrap();
            let encoded = encode_f16_vector_dim(&vector, dim).unwrap();
            assert_eq!(encoded.len(), dim * 2);
            let decoded = decode_f16_vector_dim(&encoded, dim).unwrap();
            assert_eq!(decoded.len(), dim);
            for (left, right) in vector.iter().zip(decoded.iter()) {
                assert!((left - right).abs() < 0.002);
            }
        }
    }

    #[test]
    fn f16_roundtrip_preserves_normalized_vector() {
        let mut vector = vec![0_f32; IMAGE_EMBEDDING_DIM];
        for (index, value) in vector.iter_mut().enumerate() {
            *value = (index as f32 + 1.0).sin();
        }
        l2_normalize(&mut vector).unwrap();
        let encoded = encode_f16_vector(&vector).unwrap();
        let decoded = decode_f16_vector(&encoded).unwrap();
        for (left, right) in vector.iter().zip(decoded.iter()) {
            assert!((left - right).abs() < 0.002);
        }
    }

    #[test]
    fn visual_embedding_is_stable_and_normalized() {
        let dir = std::env::temp_dir().join(format!(
            "lumeno-ai-embed-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.png");
        let mut image = RgbImage::new(64, 64);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = Rgb([(x * 3) as u8, (y * 5) as u8, ((x + y) * 2) as u8]);
        }
        image.save(&path).unwrap();

        let first = compute_visual_embedding(&path).unwrap();
        let second = compute_visual_embedding(&path).unwrap();
        assert_eq!(first.len(), IMAGE_EMBEDDING_DIM);
        assert!((cosine_similarity(&first, &second) - 1.0).abs() < 1e-5);
        let norm = first.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn batch_matches_single() {
        let mut left = RgbImage::new(48, 48);
        let mut right = RgbImage::new(48, 48);
        for (x, y, pixel) in left.enumerate_pixels_mut() {
            *pixel = Rgb([x as u8, y as u8, 40]);
        }
        for (x, y, pixel) in right.enumerate_pixels_mut() {
            *pixel = Rgb([200u8.saturating_sub(x as u8), 80, y as u8]);
        }
        let singles = [
            compute_visual_embedding_rgb(&left).unwrap(),
            compute_visual_embedding_rgb(&right).unwrap(),
        ];
        let batch = compute_visual_embeddings_rgb_batch(&[&left, &right]).unwrap();
        assert_eq!(batch.len(), 2);
        assert!((cosine_similarity(&singles[0], &batch[0]) - 1.0).abs() < 1e-4);
        assert!((cosine_similarity(&singles[1], &batch[1]) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn siglip_text_image_score_stretches_low_cosine() {
        assert!((calibrate_siglip_text_image_score(0.0) - 0.0).abs() < 1e-5);
        assert!((calibrate_siglip_text_image_score(0.06) - 0.5).abs() < 1e-5);
        assert!((calibrate_siglip_text_image_score(0.12) - 1.0).abs() < 1e-5);
        assert!((calibrate_siglip_text_image_score(0.20) - 1.0).abs() < 1e-5);
        assert!((calibrate_siglip_text_image_score(-0.1) - 0.0).abs() < 1e-5);
        // Raw cosine 0.06 would fail CLIP-era 0.25 threshold; calibrated blend passes.
        let blended = 0.65 * calibrate_siglip_text_image_score(0.06);
        assert!(blended > 0.25);
    }

    #[test]
    fn text_hash_embedding_is_normalized() {
        let vector = compute_text_hash_embedding("sunset beach").unwrap();
        assert_eq!(vector.len(), IMAGE_EMBEDDING_DIM);
        let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4);
    }

    #[test]
    fn different_images_are_not_identical() {
        let dir = std::env::temp_dir().join(format!(
            "lumeno-ai-embed-diff-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let left_path = dir.join("left.png");
        let right_path = dir.join("right.png");

        let mut left = RgbImage::new(64, 64);
        for (x, y, pixel) in left.enumerate_pixels_mut() {
            *pixel = if (x / 8 + y / 8) % 2 == 0 {
                Rgb([220, 40, 40])
            } else {
                Rgb([40, 180, 40])
            };
        }
        left.save(&left_path).unwrap();

        let mut right = RgbImage::new(64, 64);
        for (x, y, pixel) in right.enumerate_pixels_mut() {
            let t = ((x + y) as f32 / 10.0).sin();
            *pixel = Rgb([
                (40.0 + 180.0 * t.max(0.0)) as u8,
                40,
                (40.0 + 180.0 * (-t).max(0.0)) as u8,
            ]);
        }
        right.save(&right_path).unwrap();

        let left_vec = compute_visual_embedding(&left_path).unwrap();
        let right_vec = compute_visual_embedding(&right_path).unwrap();
        assert!(cosine_similarity(&left_vec, &right_vec) < 0.9);

        let _ = std::fs::remove_dir_all(dir);
    }
}
