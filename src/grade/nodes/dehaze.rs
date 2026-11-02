use std::sync::Arc;

use serde_json::{Value, json};

use super::{Node, NodeParam, clamp_f, object, req_f};
use crate::grade::buffer::{InputDesc, OutputDesc};
use crate::grade::exec::{self, Frame};
use crate::grade::field::{self, FieldSpec};

#[derive(Clone)]
pub struct Dehaze {
    strength: f32,
    omega: f32,
    t_min: f32,
    radius: f32,
}

impl Dehaze {
    pub(crate) fn new() -> Self {
        Self {
            strength: 0.0,
            omega: 0.95,
            t_min: 0.1,
            radius: 0.02,
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        Ok(Self {
            strength: clamp_f(req_f(obj, "strength")?, 0.0, 1.0),
            omega: clamp_f(req_f(obj, "omega")?, 0.5, 1.0),
            t_min: clamp_f(req_f(obj, "t_min")?, 0.0, 0.5),
            radius: clamp_f(req_f(obj, "radius")?, 0.0, 0.08),
        })
    }
}

impl NodeParam for Dehaze {
    fn node_type(&self) -> &'static str {
        "dehaze"
    }

    fn to_json(&self) -> Value {
        json!({
            "strength": self.strength,
            "omega": self.omega,
            "t_min": self.t_min,
            "radius": self.radius,
        })
    }

    fn fields(&self) -> Vec<FieldSpec> {
        self.ui_fields()
    }

    fn from_json(value: &Value) -> Result<Self, String>
    where
        Self: Sized,
    {
        Self::parse(value)
    }

    fn set_json(&mut self, value: &Value) -> Result<(), String> {
        *self = Self::from_json(value)?;
        Ok(())
    }
}

impl Node for Dehaze {
    fn params(&self) -> &dyn NodeParam {
        self
    }

    fn set_param(&mut self, value: &Value) -> Result<(), String> {
        NodeParam::set_json(self, value)
    }

    fn inputs(_param: &dyn NodeParam) -> Vec<InputDesc>
    where
        Self: Sized,
    {
        super::linear_mask_in()
    }

    fn outputs(_param: &dyn NodeParam) -> Vec<OutputDesc>
    where
        Self: Sized,
    {
        super::linear_image_out()
    }

    fn render(param: &dyn NodeParam, frame: &Frame) -> Frame
    where
        Self: Sized,
    {
        super::param_as::<Self>(param).render_frame(frame)
    }

    fn run(
        &self,
        inputs: &std::collections::BTreeMap<String, std::sync::Arc<crate::grade::buffer::Value>>,
        _ctx: &crate::grade::exec::ExecCtx,
    ) -> Result<
        std::collections::BTreeMap<String, crate::grade::buffer::Value>,
        crate::grade::error::GradeError,
    > {
        super::run_rendered::<Self>(self.params(), inputs)
    }

    fn defaults(&self) -> Box<dyn Node> {
        Box::new(Self::new())
    }
}

impl Dehaze {
    pub(super) fn ui_fields(&self) -> Vec<FieldSpec> {
        vec![
            field::slider(
                "strength",
                "grade.field.strength",
                self.strength,
                0.0,
                1.0,
                format!("{:.2}", self.strength),
            ),
            field::slider(
                "omega",
                "Omega",
                self.omega,
                0.5,
                1.0,
                format!("{:.2}", self.omega),
            ),
            field::slider(
                "t_min",
                "grade.field.floor",
                self.t_min,
                0.0,
                0.5,
                format!("{:.2}", self.t_min),
            ),
            field::slider(
                "radius",
                "grade.field.radius",
                self.radius,
                0.0,
                0.08,
                format!("{:.3}", self.radius),
            ),
        ]
    }

    pub(super) fn render_frame(&self, input: &Frame) -> Frame {
        if self.strength <= 0.0 {
            return input.clone_frame();
        }
        let (w, h) = (input.width as usize, input.height as usize);
        let r = ((self.radius * w.min(h) as f32).round() as usize).min(48);
        // Haze is slow. A sliding minimum on the sharp image beats against
        // any repeating detail and that beat is moiré. Estimate on a low
        // pass and put the original detail back unchanged.
        let sigma = if r == 0 {
            0.0
        } else {
            (r as f32 / 3.0).max(1.0)
        };
        let low = if sigma == 0.0 {
            input.rgb.clone()
        } else {
            Arc::new(blur_rgb(&input.rgb[..], w, h, sigma))
        };
        let mut dark = vec![0f32; w * h];
        for (i, px) in low.chunks_exact(3).enumerate() {
            dark[i] = px[0].min(px[1]).min(px[2]).max(0.0);
        }
        let dark = exec::min_filter(&dark, w, h, r);
        let mut guide = vec![0f32; w * h];
        for (i, px) in low.chunks_exact(3).enumerate() {
            guide[i] = (px[0] + px[1] + px[2]) * (1.0 / 3.0);
        }
        let mut air = 0f32;
        let mut count = 0f32;
        let mut ranked: Vec<(usize, f32)> = dark.iter().copied().enumerate().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let take = ((ranked.len() as f32) * 0.001).ceil().max(1.0) as usize;
        for (idx, _) in ranked.into_iter().take(take) {
            let px = &low[idx * 3..idx * 3 + 3];
            air += px[0].max(px[1]).max(px[2]);
            count += 1.0;
        }
        let air = (air / count.max(1.0)).clamp(0.05, 4.0);
        let air_luma = air;
        let t = {
            let mut t = vec![0f32; w * h];
            for i in 0..w * h {
                t[i] = (1.0 - self.omega * dark[i] / air_luma.max(1e-4)).clamp(self.t_min, 1.0);
            }
            if r == 0 {
                t
            } else {
                exec::guided_by(&guide, &t, w, h, r, r, 1e-2)
            }
        };
        let mut rgb = vec![0f32; input.rgb.len()];
        exec::for_rows(h, w * 3, &mut rgb, |y, row| {
            for x in 0..w {
                let i = y * w + x;
                let base = &low[i * 3..i * 3 + 3];
                let px = &input.rgb[i * 3..i * 3 + 3];
                let ti = t[i].max(self.t_min).max(1e-4);
                for c in 0..3 {
                    let j = (base[c] - air) / ti + air;
                    let corrected = base[c] + self.strength * (j - base[c]);
                    row[x * 3 + c] = corrected + (px[c] - base[c]);
                }
            }
        });
        Frame {
            width: input.width,
            height: input.height,
            space: input.space,
            rgb: Arc::new(rgb),
            transform: input.transform,
            roi: input.roi,
        }
    }
}

fn blur_rgb(rgb: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let n = w * h;
    let mut ch = [vec![0f32; n], vec![0f32; n], vec![0f32; n]];
    for (i, px) in rgb.chunks_exact(3).enumerate() {
        ch[0][i] = px[0];
        ch[1][i] = px[1];
        ch[2][i] = px[2];
    }
    let planes: Vec<Vec<f32>> = if exec::uses_pool() {
        exec::in_parallel(|| {
            use rayon::prelude::*;
            ch.into_par_iter()
                .map(|plane| exec::gaussian(&plane, w, h, sigma))
                .collect()
        })
    } else {
        ch.into_iter()
            .map(|plane| exec::gaussian(&plane, w, h, sigma))
            .collect()
    };
    let mut out = vec![0f32; n * 3];
    for i in 0..n {
        out[i * 3] = planes[0][i];
        out[i * 3 + 1] = planes[1][i];
        out[i * 3 + 2] = planes[2][i];
    }
    out
}
mod tests {
    use super::*;
    use crate::grade::exec::{Roi, Space, ViewTransform};

    #[test]
    fn zero_radius_does_not_paint_a_rim() {
        let (w, h) = (64u32, 64u32);
        let mut rgb = vec![0f32; (w * h * 3) as usize];
        for y in 0..h {
            for x in 0..w {
                let v = if y < 8 && x > 48 {
                    0.95
                } else if x < 32 {
                    0.08
                } else {
                    0.45
                };
                let i = ((y * w + x) * 3) as usize;
                rgb[i] = v;
                rgb[i + 1] = v;
                rgb[i + 2] = v;
            }
        }
        let out = Dehaze {
            strength: 1.0,
            omega: 0.95,
            t_min: 0.0,
            radius: 0.0,
        }
        .render_frame(&Frame {
            width: w,
            height: h,
            space: Space::Linear,
            rgb: Arc::new(rgb),
            transform: ViewTransform::default(),
            roi: Roi::default(),
        });
        let at = |x: u32, y: u32| out.rgb[((y * w + x) * 3) as usize];
        let beside = at(32, 40);
        let interior = at(60, 40);
        assert!(
            (beside - interior).abs() < 1e-4,
            "pixel on the edge {beside} should match the open gray {interior}"
        );
    }

    #[test]
    fn patch_radius_does_not_saturate_each_pixel() {
        let (w, h) = (48u32, 48u32);
        let mut rgb = vec![0f32; (w * h * 3) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 3) as usize;
                let blue = if (x + y) % 2 == 0 { 0.15 } else { 0.45 };
                rgb[i] = 0.5;
                rgb[i + 1] = 0.5;
                rgb[i + 2] = blue;
            }
        }
        // A corner of sky so airlight is not the checker itself.
        for y in 0..6 {
            for x in 40..w {
                let i = ((y * w + x) * 3) as usize;
                rgb[i] = 0.95;
                rgb[i + 1] = 0.95;
                rgb[i + 2] = 0.95;
            }
        }
        let frame = Frame {
            width: w,
            height: h,
            space: Space::Linear,
            rgb: Arc::new(rgb),
            transform: ViewTransform::default(),
            roi: Roi::default(),
        };
        let point = Dehaze {
            strength: 1.0,
            omega: 0.95,
            t_min: 0.1,
            radius: 0.0,
        }
        .render_frame(&frame);
        let patch = Dehaze {
            strength: 1.0,
            omega: 0.95,
            t_min: 0.1,
            radius: 0.12,
        }
        .render_frame(&frame);
        let sat = |buf: &Frame, x: u32, y: u32| {
            let i = ((y * w + x) * 3) as usize;
            let px = &buf.rgb[i..i + 3];
            let max = px[0].max(px[1]).max(px[2]);
            let min = px[0].min(px[1]).min(px[2]);
            if max.abs() < 1e-4 {
                0.0
            } else {
                (max - min) / max.abs()
            }
        };
        // A pixel whose own blue is high, so per-pixel dark channel treats it as haze.
        let x = 21;
        let y = 24;
        assert!(
            sat(&patch, x, y) < sat(&point, x, y) - 0.05,
            "patch sat {} should stay under per-pixel sat {}",
            sat(&patch, x, y),
            sat(&point, x, y)
        );
    }

    #[test]
    fn fine_detail_is_not_gained_by_the_patch() {
        let (w, h) = (64u32, 64u32);
        let mut rgb = vec![0f32; (w * h * 3) as usize];
        for y in 0..h {
            for x in 0..w {
                let v = if (x + y) % 2 == 0 { 0.25 } else { 0.55 };
                let i = ((y * w + x) * 3) as usize;
                rgb[i] = v;
                rgb[i + 1] = v;
                rgb[i + 2] = v;
            }
        }
        let frame = Frame {
            width: w,
            height: h,
            space: Space::Linear,
            rgb: Arc::new(rgb.clone()),
            transform: ViewTransform::default(),
            roi: Roi::default(),
        };
        let out = Dehaze {
            strength: 1.0,
            omega: 0.95,
            t_min: 0.1,
            radius: 0.16,
        }
        .render_frame(&frame);
        let delta = |x: u32, y: u32| {
            let i = ((y * w + x) * 3) as usize;
            out.rgb[i] - rgb[i]
        };
        let a = delta(30, 30);
        let b = delta(31, 30);
        assert!(
            (a - b).abs() < 0.02,
            "neighboring checker pixels should share a correction, got {a} and {b}"
        );
    }
}
