use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, RgbImage};

/// OpenAI/CLIP-style mean/std used by many SigLIP checkpoints.
pub const SIGLIP_MEAN: [f32; 3] = [0.5, 0.5, 0.5];
pub const SIGLIP_STD: [f32; 3] = [0.5, 0.5, 0.5];

/// ArcFace / InsightFace face crop normalization.
pub const ARCFACE_MEAN: [f32; 3] = [0.5, 0.5, 0.5];
pub const ARCFACE_STD: [f32; 3] = [0.5, 0.5, 0.5];

pub fn resize_rgb(image: &DynamicImage, width: u32, height: u32) -> RgbImage {
    image
        .resize_exact(width, height, FilterType::Triangle)
        .to_rgb8()
}

pub fn normalize_rgb_chw(
    rgb: &RgbImage,
    mean: [f32; 3],
    std: [f32; 3],
) -> (usize, usize, Vec<f32>) {
    let width = rgb.width() as usize;
    let height = rgb.height() as usize;
    let mut out = vec![0.0_f32; 3 * width * height];
    for y in 0..height {
        for x in 0..width {
            let pixel = rgb.get_pixel(x as u32, y as u32);
            let idx = y * width + x;
            for c in 0..3 {
                let value = f32::from(pixel[c]) / 255.0;
                out[c * width * height + idx] = (value - mean[c]) / std[c];
            }
        }
    }
    (height, width, out)
}

pub fn prepare_siglip_image(image: &DynamicImage, size: u32) -> (usize, usize, Vec<f32>) {
    let rgb = resize_rgb(image, size, size);
    normalize_rgb_chw(&rgb, SIGLIP_MEAN, SIGLIP_STD)
}

/// InsightFace SCRFD preprocess: letterbox keep-aspect, mean 127.5 / std 128.
/// Returns `(chw, det_scale)` where boxes in network space ÷ `det_scale` → original pixels.
pub fn prepare_scrfd_image(
    image: &DynamicImage,
    input_size: u32,
) -> (Vec<f32>, f32, u32, u32) {
    let (orig_w, orig_h) = (image.width(), image.height());
    let input = input_size.max(32);
    let im_ratio = orig_h as f32 / orig_w as f32;
    let model_ratio = 1.0_f32;
    let (new_w, new_h) = if im_ratio > model_ratio {
        let new_h = input;
        let new_w = ((new_h as f32 / im_ratio).round() as u32).max(1);
        (new_w.min(input), new_h)
    } else {
        let new_w = input;
        let new_h = ((new_w as f32 * im_ratio).round() as u32).max(1);
        (new_w, new_h.min(input))
    };
    let det_scale = new_h as f32 / orig_h as f32;
    let resized = image
        .resize_exact(new_w, new_h, FilterType::Triangle)
        .to_rgb8();

    let size = input as usize;
    let mut out = vec![0.0_f32; 3 * size * size];
    // Pad is zeros → after normalize becomes (0 - 127.5) / 128.
    let pad = (0.0 - 127.5) / 128.0;
    out.fill(pad);
    for y in 0..new_h as usize {
        for x in 0..new_w as usize {
            let pixel = resized.get_pixel(x as u32, y as u32);
            let idx = y * size + x;
            for c in 0..3 {
                let value = f32::from(pixel[c]);
                out[c * size * size + idx] = (value - 127.5) / 128.0;
            }
        }
    }
    (out, det_scale, orig_w, orig_h)
}

pub fn prepare_arcface_crop(rgb: &RgbImage) -> (usize, usize, Vec<f32>) {
    normalize_rgb_chw(rgb, ARCFACE_MEAN, ARCFACE_STD)
}

/// Similarity-transform align face to 112x112 using 5 landmarks (InsightFace template).
pub fn align_face_112(image: &DynamicImage, landmarks: [[f32; 2]; 5]) -> Result<RgbImage, String> {
    const TEMPLATE: [[f32; 2]; 5] = [
        [38.2946, 51.6963],
        [73.5318, 51.5014],
        [56.0252, 71.7366],
        [41.5493, 92.3655],
        [70.7299, 92.2041],
    ];
    let rgb = image.to_rgb8();
    let (src_w, src_h) = (rgb.width() as i32, rgb.height() as i32);
    if src_w < 8 || src_h < 8 {
        return Err("image too small for face align".to_string());
    }

    let transform = similarity_transform(&landmarks, &TEMPLATE)?;
    let mut out = RgbImage::new(112, 112);
    for y in 0..112_u32 {
        for x in 0..112_u32 {
            let (sx, sy) = apply_inverse_similarity(transform, x as f32, y as f32);
            let px = sample_bilinear(&rgb, sx, sy);
            out.put_pixel(x, y, image::Rgb(px));
        }
    }
    Ok(out)
}

#[derive(Clone, Copy)]
struct Similarity2d {
    a: f32,
    b: f32,
    tx: f32,
    ty: f32,
}

fn similarity_transform(
    src: &[[f32; 2]; 5],
    dst: &[[f32; 2]; 5],
) -> Result<Similarity2d, String> {
    let mut src_mean = [0.0_f32; 2];
    let mut dst_mean = [0.0_f32; 2];
    for i in 0..5 {
        src_mean[0] += src[i][0];
        src_mean[1] += src[i][1];
        dst_mean[0] += dst[i][0];
        dst_mean[1] += dst[i][1];
    }
    src_mean[0] /= 5.0;
    src_mean[1] /= 5.0;
    dst_mean[0] /= 5.0;
    dst_mean[1] /= 5.0;

    let mut src_centered = [[0.0_f32; 2]; 5];
    let mut dst_centered = [[0.0_f32; 2]; 5];
    let mut src_var = 0.0_f32;
    for i in 0..5 {
        src_centered[i][0] = src[i][0] - src_mean[0];
        src_centered[i][1] = src[i][1] - src_mean[1];
        dst_centered[i][0] = dst[i][0] - dst_mean[0];
        dst_centered[i][1] = dst[i][1] - dst_mean[1];
        src_var += src_centered[i][0] * src_centered[i][0] + src_centered[i][1] * src_centered[i][1];
    }
    if src_var < 1e-6 {
        return Err("degenerate face landmarks".to_string());
    }

    let mut cov00 = 0.0_f32;
    let mut cov01 = 0.0_f32;
    let mut cov10 = 0.0_f32;
    let mut cov11 = 0.0_f32;
    for i in 0..5 {
        cov00 += dst_centered[i][0] * src_centered[i][0];
        cov01 += dst_centered[i][0] * src_centered[i][1];
        cov10 += dst_centered[i][1] * src_centered[i][0];
        cov11 += dst_centered[i][1] * src_centered[i][1];
    }

    // 2x2 SVD-free similarity: [a -b; b a]
    let a = (cov00 + cov11) / src_var;
    let b = (cov10 - cov01) / src_var;
    let tx = dst_mean[0] - (a * src_mean[0] - b * src_mean[1]);
    let ty = dst_mean[1] - (b * src_mean[0] + a * src_mean[1]);
    Ok(Similarity2d { a, b, tx, ty })
}

fn apply_inverse_similarity(t: Similarity2d, x: f32, y: f32) -> (f32, f32) {
    // Forward: [x'; y'] = [[a,-b],[b,a]] * [x,y] + [tx,ty]
    // Inverse of scale-rotation: (1/(a^2+b^2)) * [[a,b],[-b,a]]
    let denom = t.a * t.a + t.b * t.b;
    if denom < 1e-12 {
        return (x, y);
    }
    let dx = x - t.tx;
    let dy = y - t.ty;
    let sx = (t.a * dx + t.b * dy) / denom;
    let sy = (-t.b * dx + t.a * dy) / denom;
    (sx, sy)
}

fn sample_bilinear(rgb: &RgbImage, x: f32, y: f32) -> [u8; 3] {
    let w = rgb.width() as f32;
    let h = rgb.height() as f32;
    if x < 0.0 || y < 0.0 || x >= w - 1.0 || y >= h - 1.0 {
        return [0, 0, 0];
    }
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(rgb.width() - 1);
    let y1 = (y0 + 1).min(rgb.height() - 1);
    let dx = x - x0 as f32;
    let dy = y - y0 as f32;
    let mut out = [0_u8; 3];
    for c in 0..3 {
        let p00 = f32::from(rgb.get_pixel(x0, y0)[c]);
        let p10 = f32::from(rgb.get_pixel(x1, y0)[c]);
        let p01 = f32::from(rgb.get_pixel(x0, y1)[c]);
        let p11 = f32::from(rgb.get_pixel(x1, y1)[c]);
        let top = p00 * (1.0 - dx) + p10 * dx;
        let bottom = p01 * (1.0 - dx) + p11 * dx;
        out[c] = (top * (1.0 - dy) + bottom * dy).round().clamp(0.0, 255.0) as u8;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgb, RgbImage};

    #[test]
    fn normalize_produces_chw_layout() {
        let mut rgb = RgbImage::new(2, 2);
        rgb.put_pixel(0, 0, Rgb([255, 0, 0]));
        let (h, w, data) = normalize_rgb_chw(&rgb, SIGLIP_MEAN, SIGLIP_STD);
        assert_eq!((h, w), (2, 2));
        assert_eq!(data.len(), 12);
        assert!((data[0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn align_face_returns_112() {
        let rgb = RgbImage::from_pixel(200, 200, Rgb([120, 90, 70]));
        let image = DynamicImage::ImageRgb8(rgb);
        let landmarks = [
            [70.0, 80.0],
            [130.0, 80.0],
            [100.0, 110.0],
            [80.0, 140.0],
            [120.0, 140.0],
        ];
        let aligned = align_face_112(&image, landmarks).unwrap();
        assert_eq!(aligned.dimensions(), (112, 112));
    }
}
