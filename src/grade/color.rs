//! Color helpers for the preview. Matrices are D65 linear sRGB ↔ linear Rec.2020.

pub const LUMA: [f32; 3] = [0.2627, 0.6780, 0.0593];

const SRGB_TO_REC2020: [f32; 9] = [
    0.6274039, 0.3292830, 0.0433131, 0.0690973, 0.9195404, 0.0113623, 0.0163914, 0.0880133,
    0.8955953,
];

const REC2020_TO_SRGB: [f32; 9] = [
    1.6604910, -0.5876411, -0.0728499, -0.1245505, 1.1328999, -0.0083494, -0.0181508, -0.1005789,
    1.1187297,
];

pub fn luma(r: f32, g: f32, b: f32) -> f32 {
    LUMA[0] * r + LUMA[1] * g + LUMA[2] * b
}

pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

pub fn linear_to_srgb(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// ITU-R BT.709 camera OETF. Primaries match sRGB; the curve does not.
pub fn linear_to_rec709(c: f32) -> f32 {
    let c = c.clamp(0.0, 1.0);
    if c < 0.018 {
        4.5 * c
    } else {
        1.099 * c.powf(0.45) - 0.099
    }
}

pub fn rec709_to_linear(c: f32) -> f32 {
    if c < 0.081 {
        (c / 4.5).max(0.0)
    } else {
        ((c + 0.099) / 1.099).max(0.0).powf(1.0 / 0.45)
    }
}

/// Adobe RGB (1998) is a pure power, 563/256, with no linear toe.
const ADOBE_GAMMA: f32 = 2.19921875;

pub fn linear_to_adobe(c: f32) -> f32 {
    c.clamp(0.0, 1.0).powf(1.0 / ADOBE_GAMMA)
}

pub fn adobe_to_linear(c: f32) -> f32 {
    c.max(0.0).powf(ADOBE_GAMMA)
}

/// Linear Rec.2020 → linear Adobe RGB (D65). Row-major.
pub const REC2020_TO_ADOBE: [f32; 9] = [
    1.1520445, -0.0975668, -0.0544777, -0.1245503, 1.1328999, -0.0083494, -0.0225313, -0.0497960,
    1.0723273,
];

pub const ADOBE_TO_REC2020: [f32; 9] = [
    0.8772884, 0.0775388, 0.0451729, 0.0966174, 0.8915322, 0.0118502, 0.0229199, 0.0430295,
    0.9340505,
];

/// Linear Rec.2020 (D65) → linear ProPhoto (D50), Bradford in between.
pub const REC2020_TO_PROPHOTO: [f32; 9] = [
    0.8353116, 0.0487345, 0.1159538, 0.0540485, 0.9289024, 0.0170491, -0.0023303, 0.0363444,
    0.9659859,
];

pub const PROPHOTO_TO_REC2020: [f32; 9] = [
    1.2004718, -0.0573838, -0.1430879, -0.0699513, 1.0806271, -0.0106758, 0.0055278, -0.0407962,
    1.0352683,
];

fn mul(m: &[f32; 9], r: f32, g: f32, b: f32) -> [f32; 3] {
    [
        m[0] * r + m[1] * g + m[2] * b,
        m[3] * r + m[4] * g + m[5] * b,
        m[6] * r + m[7] * g + m[8] * b,
    ]
}

pub fn linear_srgb_to_rec2020(r: f32, g: f32, b: f32) -> [f32; 3] {
    mul(&SRGB_TO_REC2020, r, g, b)
}

pub fn rec2020_to_linear_srgb(r: f32, g: f32, b: f32) -> [f32; 3] {
    mul(&REC2020_TO_SRGB, r, g, b)
}

/// Bradford chromatic adaptation, XYZ D50 → XYZ D65.
const BRADFORD_D50_TO_D65: [f32; 9] = [
    0.9555766, -0.0230393, 0.0631636, -0.0282895, 1.0099416, 0.0210077, 0.0122982, -0.0204830,
    1.3299098,
];

/// XYZ D65 → linear Rec.2020. ITU-R BT.2020 primaries.
const XYZ_D65_TO_REC2020: [f32; 9] = [
    1.7166512, -0.3556708, -0.2533663, -0.6666844, 1.6164812, 0.0157685, 0.0176399, -0.0427706,
    0.9421031,
];

fn mul_mm(a: &[f32; 9], b: &[f32; 9]) -> [f32; 9] {
    let mut out = [0f32; 9];
    for row in 0..3 {
        for col in 0..3 {
            out[row * 3 + col] =
                a[row * 3] * b[col] + a[row * 3 + 1] * b[3 + col] + a[row * 3 + 2] * b[6 + col];
        }
    }
    out
}

/// LibRaw `LibRaw_constants::rec2020_rgb`, the matrix `convert_to_rgb` applies
/// for `output_color` Rec.2020. It is not the same numbers as `SRGB_TO_REC2020`.
const LIBRAW_REC2020: [f32; 9] = [
    0.627452, 0.329249, 0.043299, 0.069109, 0.919531, 0.011360, 0.016398, 0.088030, 0.895572,
];

/// LibRaw `rgb_cam` is camera RGB → linear sRGB. Direct Rec.2020 output multiplies
/// that by `rec2020_rgb`, so the default profile is that same product.
pub fn rec2020_from_rgb_cam(rgb_cam: [[f32; 4]; 3]) -> [f32; 9] {
    let mut cam = [0f32; 9];
    let mut energy = 0f32;
    for row in 0..3 {
        for col in 0..3 {
            let value = rgb_cam[row][col];
            cam[row * 3 + col] = value;
            energy += value.abs();
        }
    }
    if energy < 1e-4 {
        cam = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
    }
    mul_mm(&LIBRAW_REC2020, &cam)
}

/// `camera_to_xyz_d50` is row-major camera RGB → XYZ D50, the inverse of a DNG ColorMatrix tag.
pub fn rec2020_from_camera_xyz_d50(camera_to_xyz_d50: [f32; 9]) -> [f32; 9] {
    let xyz_d65 = mul_mm(&BRADFORD_D50_TO_D65, &camera_to_xyz_d50);
    mul_mm(&XYZ_D65_TO_REC2020, &xyz_d65)
}

pub fn apply_matrix(m: &[f32; 9], r: f32, g: f32, b: f32) -> [f32; 3] {
    mul(m, r, g, b)
}

/// Preview white balance when the host already decoded the RAW. Green stays 1.
pub fn kelvin_gains(temp: f32, tint: f32) -> [f32; 3] {
    let t = temp.clamp(2000.0, 12000.0) / 100.0;
    let r = if t <= 66.0 {
        255.0
    } else {
        (329.69873 * (t - 60.0).powf(-0.13320476)).clamp(0.0, 255.0)
    };
    let g = if t <= 66.0 {
        (99.4708 * t.ln() - 161.11957).clamp(1.0, 255.0)
    } else {
        (288.12216 * (t - 60.0).powf(-0.07551485)).clamp(1.0, 255.0)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        (138.51773 * (t - 10.0).ln() - 305.0448).clamp(0.0, 255.0)
    };
    let tint = tint.clamp(-100.0, 100.0) / 100.0;
    [
        (r / g) * (1.0 + tint * 0.2),
        1.0,
        (b / g) * (1.0 - tint * 0.2),
    ]
}

pub fn encode_u8(c: f32) -> u8 {
    (linear_to_srgb(c) * 255.0).round().clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_camera_matches_srgb_column() {
        let mut rgb_cam = [[0f32; 4]; 3];
        for channel in 0..3 {
            rgb_cam[channel][channel] = 1.0;
        }
        let matrix = rec2020_from_rgb_cam(rgb_cam);
        let red = apply_matrix(&matrix, 1.0, 0.0, 0.0);
        assert!((red[0] - LIBRAW_REC2020[0]).abs() < 1e-6);
        assert!((red[1] - LIBRAW_REC2020[3]).abs() < 1e-6);
        assert!((red[2] - LIBRAW_REC2020[6]).abs() < 1e-6);
    }

    #[test]
    fn bradford_moves_d50_white_toward_d65() {
        let d50 = apply_matrix(&BRADFORD_D50_TO_D65, 0.9642, 1.0, 0.8249);
        assert!((d50[0] - 0.9504).abs() < 0.01);
        assert!((d50[1] - 1.0).abs() < 0.01);
        assert!((d50[2] - 1.0888).abs() < 0.01);
    }
}
