//! Scene metering for the initial exposure. The grade library has no copy and
//! does not read files; this only turns a linear frame into an EV.

use super::color::{MIDDLE_GREY, apply_matrix, luma};
use super::exec::{Frame, Space};

const SCENE_WHITE: f32 = 1.0;
/// The 99th percentile may go at most this far past the white point when the
/// meter is brightening. A frame with a small bright window should not let the
/// room drag the window two stops into clipping; a frame with no highlights can
/// still be lifted to middle grey.
const HIGHLIGHT_HEADROOM_EV: f32 = 0.5;
/// Stats only need a shape, not every pixel. A 24 MP export samples one in
/// roughly 120 pixels, which is still enough for a stable median.
const SAMPLE_LIMIT: usize = 200_000;

/// Exposure offset that puts the median scene luminance near middle grey.
///
/// `camera_to_working` is used only when the frame is still linear camera RGB,
/// i.e. RAW pixels the decoder has not yet moved into the working space.
/// Returns `None` for a frame with no usable pixels; the caller then keeps the
/// camera baseline or the raster default.
pub fn auto_exposure_ev(frame: &Frame, camera_to_working: Option<[f32; 9]>) -> Option<f32> {
    let pixels = frame.rgb.len() / 3;
    if pixels == 0 {
        return None;
    }
    let stride = pixels.div_ceil(SAMPLE_LIMIT).max(1);
    let mut samples = Vec::with_capacity(pixels.div_ceil(stride));
    let camera = frame.space == Space::Camera;
    for pixel in frame.rgb.chunks_exact(3).step_by(stride) {
        let rgb = if camera {
            match &camera_to_working {
                Some(matrix) => apply_matrix(matrix, pixel[0], pixel[1], pixel[2]),
                None => [pixel[0], pixel[1], pixel[2]],
            }
        } else {
            [pixel[0], pixel[1], pixel[2]]
        };
        let y = luma(rgb[0], rgb[1], rgb[2]);
        // Pure black is usually a letterbox or a dead pixel, not a shadow
        // measurement. Keeping it would pull the median down and overexpose.
        if y.is_finite() && y > 1e-6 {
            samples.push(y);
        }
    }
    if samples.is_empty() {
        return None;
    }
    samples.sort_by(|left, right| left.total_cmp(right));
    let median = percentile(&samples, 0.50);
    let highlight = percentile(&samples, 0.99);
    let ev_mid = (MIDDLE_GREY / median).log2();
    let ev_white = (SCENE_WHITE / highlight).log2();
    let ev = ev_mid.min(ev_white + HIGHLIGHT_HEADROOM_EV);
    Some((ev.clamp(-5.0, 5.0) * 100.0).round() / 100.0)
}

fn percentile(sorted: &[f32], quantile: f32) -> f32 {
    let last = sorted.len() - 1;
    let index = ((last as f32) * quantile.clamp(0.0, 1.0)).round() as usize;
    sorted[index.min(last)]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::grade::exec::{Roi, ViewTransform};

    fn frame(rgb: Vec<f32>, space: Space) -> Frame {
        let pixels = rgb.len() / 3;
        Frame {
            width: pixels.max(1) as u32,
            height: 1,
            space,
            rgb: Arc::new(rgb),
            transform: ViewTransform::default(),
            roi: Roi::default(),
        }
    }

    fn flat(value: f32) -> Frame {
        frame(vec![value; 3 * 1000], Space::Linear)
    }

    #[test]
    fn middle_grey_needs_no_correction() {
        let ev = auto_exposure_ev(&flat(MIDDLE_GREY), None).unwrap();
        assert!(ev.abs() < 0.01, "ev={ev}");
    }

    #[test]
    fn dark_and_bright_frames_move_toward_middle_grey() {
        let dark = auto_exposure_ev(&flat(MIDDLE_GREY / 8.0), None).unwrap();
        assert!((dark - 3.0).abs() < 0.02, "dark ev={dark}");
        let bright = auto_exposure_ev(&flat(MIDDLE_GREY * 4.0), None).unwrap();
        assert!((bright + 2.0).abs() < 0.02, "bright ev={bright}");
    }

    #[test]
    fn a_bright_highlight_caps_positive_exposure() {
        // Ten percent at scene white, the rest almost black. The median asks
        // for +4.2 EV, the 99th percentile only allows +0.5 EV.
        let mut rgb = vec![0.01; 3 * 10_000];
        for pixel in rgb.chunks_exact_mut(3).take(1_000) {
            pixel.fill(1.0);
        }
        let ev = auto_exposure_ev(&frame(rgb, Space::Linear), None).unwrap();
        assert!((ev - HIGHLIGHT_HEADROOM_EV).abs() < 0.02, "ev={ev}");
    }

    #[test]
    fn exposure_is_clamped_to_the_node_range() {
        let ev = auto_exposure_ev(&flat(1e-5), None).unwrap();
        assert!((ev - 5.0).abs() < 0.001, "ev={ev}");
    }

    #[test]
    fn an_empty_or_black_frame_has_no_measurement() {
        assert!(auto_exposure_ev(&frame(Vec::new(), Space::Linear), None).is_none());
        assert!(auto_exposure_ev(&flat(0.0), None).is_none());
    }

    #[test]
    fn camera_pixels_go_through_the_working_matrix() {
        let matrix = [2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 2.0];
        let camera = frame(vec![0.1; 3 * 1000], Space::Camera);
        let metered = auto_exposure_ev(&camera, Some(matrix)).unwrap();
        let skipped = auto_exposure_ev(&camera, None).unwrap();
        assert!(metered < skipped, "metered={metered} skipped={skipped}");
        assert!((metered - (MIDDLE_GREY / 0.2).log2()).abs() < 0.02);
    }
}
