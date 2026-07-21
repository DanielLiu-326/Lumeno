use image::{DynamicImage, GenericImageView, RgbImage};
use std::path::Path;

use crate::backend::album::decoder::{
    create_decoder, DecodeOptions, DecoderCreateOptions,
};

use super::embedding::{compute_visual_embedding_rgb, l2_normalize};

pub const FACE_DETECTOR_REVISION: &str = "face-heuristic-v1";
pub const MAX_FACES_PER_PHOTO: usize = 8;

#[derive(Debug, Clone)]
pub struct DetectedFace {
    /// Normalized `[x, y, w, h]` in `[0, 1]` relative to the source image.
    pub box_xywh: [f32; 4],
    /// Normalized landmarks in `[0, 1]` relative to the source image.
    pub landmarks: [[f32; 2]; 5],
    pub detection_score: f32,
    pub quality_score: f32,
    pub embedding: Vec<f32>,
}

/// Convert absolute pixel boxes (or already-normalized ones) to `[0, 1]` xywh.
pub fn normalize_box_xywh(box_xywh: [f32; 4], width: u32, height: u32) -> [f32; 4] {
    let [x, y, w, h] = box_xywh;
    if looks_normalized_box(box_xywh) {
        return [
            x.clamp(0.0, 1.0),
            y.clamp(0.0, 1.0),
            w.clamp(0.0, 1.0),
            h.clamp(0.0, 1.0),
        ];
    }
    let iw = width.max(1) as f32;
    let ih = height.max(1) as f32;
    [(x / iw).clamp(0.0, 1.0), (y / ih).clamp(0.0, 1.0), (w / iw).clamp(0.0, 1.0), (h / ih).clamp(0.0, 1.0)]
}

pub fn normalize_landmarks(
    landmarks: [[f32; 2]; 5],
    width: u32,
    height: u32,
) -> [[f32; 2]; 5] {
    if landmarks
        .iter()
        .all(|point| point[0] <= 1.5 && point[1] <= 1.5)
    {
        return landmarks.map(|[x, y]| [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)]);
    }
    let iw = width.max(1) as f32;
    let ih = height.max(1) as f32;
    landmarks.map(|[x, y]| [(x / iw).clamp(0.0, 1.0), (y / ih).clamp(0.0, 1.0)])
}

fn looks_normalized_box(box_xywh: [f32; 4]) -> bool {
    box_xywh.iter().all(|value| *value <= 1.5)
}

#[derive(Debug, Clone, Copy)]
struct CandidateBox {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    score: f32,
}

fn is_skin_pixel(r: u8, g: u8, b: u8) -> bool {
    let r = f32::from(r);
    let g = f32::from(g);
    let b = f32::from(b);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    r > 95.0
        && g > 40.0
        && b > 20.0
        && (max - min) > 15.0
        && (r - g).abs() > 15.0
        && r > g
        && r > b
}

fn downscale_for_scan(image: &DynamicImage, max_side: u32) -> RgbImage {
    let (width, height) = image.dimensions();
    if width <= max_side && height <= max_side {
        return image.to_rgb8();
    }
    let scale = max_side as f32 / width.max(height) as f32;
    let target_w = ((width as f32 * scale).round() as u32).max(1);
    let target_h = ((height as f32 * scale).round() as u32).max(1);
    image
        .resize_exact(target_w, target_h, image::imageops::FilterType::Triangle)
        .to_rgb8()
}

fn find_skin_components(rgb: &RgbImage) -> Vec<CandidateBox> {
    let (width, height) = rgb.dimensions();
    if width == 0 || height == 0 {
        return Vec::new();
    }

    let mut visited = vec![false; (width * height) as usize];
    let mut components = Vec::new();
    let min_side = (width.min(height) as f32 * 0.04).max(6.0) as u32;

    for start_y in 0..height {
        for start_x in 0..width {
            let start_index = (start_y * width + start_x) as usize;
            if visited[start_index] {
                continue;
            }
            let pixel = rgb.get_pixel(start_x, start_y);
            if !is_skin_pixel(pixel[0], pixel[1], pixel[2]) {
                visited[start_index] = true;
                continue;
            }

            let mut queue = vec![(start_x, start_y)];
            visited[start_index] = true;
            let mut min_x = start_x;
            let mut max_x = start_x;
            let mut min_y = start_y;
            let mut max_y = start_y;
            let mut skin_pixels = 0usize;
            let mut area = 0usize;

            while let Some((x, y)) = queue.pop() {
                area += 1;
                let pixel = rgb.get_pixel(x, y);
                if is_skin_pixel(pixel[0], pixel[1], pixel[2]) {
                    skin_pixels += 1;
                }
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);

                for (nx, ny) in [
                    (x.saturating_sub(1), y),
                    (x + 1, y),
                    (x, y.saturating_sub(1)),
                    (x, y + 1),
                ] {
                    if nx >= width || ny >= height {
                        continue;
                    }
                    let index = (ny * width + nx) as usize;
                    if visited[index] {
                        continue;
                    }
                    visited[index] = true;
                    let neighbor = rgb.get_pixel(nx, ny);
                    if is_skin_pixel(neighbor[0], neighbor[1], neighbor[2]) {
                        queue.push((nx, ny));
                    }
                }
            }

            let w = max_x - min_x + 1;
            let h = max_y - min_y + 1;
            if w < min_side || h < min_side {
                continue;
            }
            let aspect = w as f32 / h as f32;
            if !(0.7..=1.4).contains(&aspect) {
                continue;
            }
            let density = skin_pixels as f32 / area as f32;
            if density < 0.35 {
                continue;
            }
            let size_score = ((w * h) as f32 / (width * height) as f32).min(0.35) / 0.35;
            let aspect_score = 1.0 - ((aspect - 1.0).abs() / 0.4).min(1.0);
            let score = (density * 0.55 + size_score * 0.25 + aspect_score * 0.20).clamp(0.0, 1.0);
            components.push(CandidateBox {
                x: min_x,
                y: min_y,
                w,
                h,
                score,
            });
        }
    }

    components.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| (left.y, left.x).cmp(&(right.y, right.x)))
    });
    components
}

fn suppress_overlapping(candidates: Vec<CandidateBox>, iou_threshold: f32) -> Vec<CandidateBox> {
    let mut kept: Vec<CandidateBox> = Vec::new();
    'outer: for candidate in candidates {
        for existing in &kept {
            let overlap_x = candidate
                .x
                .max(existing.x)
                .saturating_sub(candidate.x.min(existing.x + existing.w));
            let overlap_y = candidate
                .y
                .max(existing.y)
                .saturating_sub(candidate.y.min(existing.y + existing.h));
            if overlap_x == 0 || overlap_y == 0 {
                continue;
            }
            let intersection = overlap_x * overlap_y;
            let union = candidate.w * candidate.h + existing.w * existing.h - intersection;
            if union > 0 && intersection as f32 / union as f32 > iou_threshold {
                continue 'outer;
            }
        }
        kept.push(candidate);
        if kept.len() >= MAX_FACES_PER_PHOTO {
            break;
        }
    }
    kept
}

fn landmarks_for_box(box_xywh: [f32; 4]) -> [[f32; 2]; 5] {
    let [x, y, w, h] = box_xywh;
    [
        [x + w * 0.32, y + h * 0.35],
        [x + w * 0.68, y + h * 0.35],
        [x + w * 0.50, y + h * 0.55],
        [x + w * 0.36, y + h * 0.78],
        [x + w * 0.64, y + h * 0.78],
    ]
}

fn crop_face_rgb(image: &DynamicImage, box_xywh: [f32; 4]) -> Result<RgbImage, String> {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err("empty image".to_string());
    }
    let [x, y, w, h] = box_xywh;
    let mut left = (x * width as f32).floor() as i64;
    let mut top = (y * height as f32).floor() as i64;
    let mut right = ((x + w) * width as f32).ceil() as i64;
    let mut bottom = ((y + h) * height as f32).ceil() as i64;

    let pad_x = ((right - left) as f32 * 0.08) as i64;
    let pad_y = ((bottom - top) as f32 * 0.08) as i64;
    left = (left - pad_x).max(0);
    top = (top - pad_y).max(0);
    right = (right + pad_x).min(width as i64);
    bottom = (bottom + pad_y).min(height as i64);

    if right <= left || bottom <= top {
        return Err("invalid face crop".to_string());
    }

    Ok(image
        .crop_imm(left as u32, top as u32, (right - left) as u32, (bottom - top) as u32)
        .to_rgb8())
}

fn quality_score_for_crop(crop: &RgbImage, detection_score: f32) -> f32 {
    let (width, height) = crop.dimensions();
    if width == 0 || height == 0 {
        return 0.0;
    }
    let min_side = width.min(height) as f32;
    let size_score = (min_side / 128.0).clamp(0.0, 1.0);
    let mut luma_sum = 0_f32;
    let mut luma_sq = 0_f32;
    let mut count = 0_f32;
    for pixel in crop.pixels() {
        let luma = 0.299 * f32::from(pixel[0])
            + 0.587 * f32::from(pixel[1])
            + 0.114 * f32::from(pixel[2]);
        luma_sum += luma;
        luma_sq += luma * luma;
        count += 1.0;
    }
    let mean = luma_sum / count;
    let variance = (luma_sq / count - mean * mean).max(0.0);
    let contrast_score = (variance.sqrt() / 64.0).clamp(0.0, 1.0);
    (detection_score * 0.55 + size_score * 0.30 + contrast_score * 0.15).clamp(0.0, 1.0)
}

pub fn detect_and_embed_faces(path: &Path) -> Result<Vec<DetectedFace>, String> {
    let decoder = create_decoder(DecoderCreateOptions::auto())
        .map_err(|error| format!("create decoder failed: {error}"))?;
    let bytes = std::fs::read(path).map_err(|error| format!("read photo failed: {error}"))?;
    let image = decoder
        .decode_bytes(&bytes, DecodeOptions::new())
        .map_err(|error| format!("decode photo for face detection failed: {error}"))?;
    detect_and_embed_faces_from_image(&image)
}

pub fn detect_and_embed_faces_from_image(image: &DynamicImage) -> Result<Vec<DetectedFace>, String> {
    let (orig_w, orig_h) = image.dimensions();
    if orig_w == 0 || orig_h == 0 {
        return Ok(Vec::new());
    }

    let scan = downscale_for_scan(&image, 256);
    let (scan_w, scan_h) = scan.dimensions();
    let candidates = suppress_overlapping(find_skin_components(&scan), 0.35);
    let scale_x = orig_w as f32 / scan_w as f32;
    let scale_y = orig_h as f32 / scan_h as f32;

    let mut faces = Vec::new();
    for candidate in candidates {
        let x = (candidate.x as f32 * scale_x / orig_w as f32).clamp(0.0, 1.0);
        let y = (candidate.y as f32 * scale_y / orig_h as f32).clamp(0.0, 1.0);
        let w = (candidate.w as f32 * scale_x / orig_w as f32).clamp(0.01, 1.0 - x);
        let h = (candidate.h as f32 * scale_y / orig_h as f32).clamp(0.01, 1.0 - y);
        let box_xywh = [x, y, w, h];
        let crop = crop_face_rgb(&image, box_xywh)?;
        let quality_score = quality_score_for_crop(&crop, candidate.score);
        if quality_score < 0.15 {
            continue;
        }
        let mut embedding = compute_visual_embedding_rgb(&crop)?;
        l2_normalize(&mut embedding)?;
        faces.push(DetectedFace {
            box_xywh,
            landmarks: landmarks_for_box(box_xywh),
            detection_score: candidate.score,
            quality_score,
            embedding,
        });
    }

    faces.sort_by(|left, right| {
        right
            .quality_score
            .partial_cmp(&left.quality_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    faces.truncate(MAX_FACES_PER_PHOTO);
    Ok(faces)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;

    #[test]
    fn heuristic_detector_finds_skin_blob() {
        let mut image = RgbImage::new(160, 160);
        for y in 40..120 {
            for x in 50..110 {
                image.put_pixel(x, y, image::Rgb([210, 150, 120]));
            }
        }
        let path = std::env::temp_dir().join(format!(
            "lumeno-face-test-{}.png",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        image.save(&path).unwrap();
        let faces = detect_and_embed_faces(&path).unwrap();
        assert!(!faces.is_empty());
        assert_eq!(
            faces[0].embedding.len(),
            super::super::embedding::IMAGE_EMBEDDING_DIM
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn normalize_box_accepts_pixels_and_unit_coords() {
        let pixel = normalize_box_xywh([3000.0, 1000.0, 600.0, 800.0], 6000, 4000);
        assert!((pixel[0] - 0.5).abs() < 1e-5);
        assert!((pixel[1] - 0.25).abs() < 1e-5);
        assert!((pixel[2] - 0.1).abs() < 1e-5);
        assert!((pixel[3] - 0.2).abs() < 1e-5);

        let unit = normalize_box_xywh([0.2, 0.3, 0.1, 0.15], 6000, 4000);
        assert!((unit[0] - 0.2).abs() < 1e-5);
        assert!((unit[1] - 0.3).abs() < 1e-5);
    }
}
