use std::sync::Arc;

use serde_json::{Value, json};

use super::{Node, NodeParam, clamp_f, object, req_f};
use crate::grade::buffer::{InputDesc, OutputDesc};
use crate::grade::color::luma;
use crate::grade::exec::{self, Frame};
use crate::grade::field::{self, FieldSpec};

#[derive(Clone)]
pub struct Denoise {
    luma: f32,
    chroma: f32,
    radius: f32,
    eps: f32,
}

impl Denoise {
    pub(crate) fn new() -> Self {
        Self {
            luma: 0.0,
            chroma: 0.0,
            radius: 0.008,
            eps: 1e-3,
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        Ok(Self {
            luma: clamp_f(req_f(obj, "luma")?, 0.0, 1.0),
            chroma: clamp_f(req_f(obj, "chroma")?, 0.0, 1.0),
            radius: clamp_f(req_f(obj, "radius")?, 0.002, 0.05),
            eps: clamp_f(req_f(obj, "eps")?, 1e-4, 1e-1),
        })
    }
}

impl NodeParam for Denoise {
    fn node_type(&self) -> &'static str {
        "denoise"
    }

    fn to_json(&self) -> Value {
        json!({
            "luma": self.luma,
            "chroma": self.chroma,
            "radius": self.radius,
            "eps": self.eps,
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

impl Node for Denoise {
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

impl Denoise {
    pub(super) fn ui_fields(&self) -> Vec<FieldSpec> {
        vec![
            field::slider(
                "luma",
                "grade.field.luma",
                self.luma,
                0.0,
                1.0,
                format!("{:.2}", self.luma),
            ),
            field::slider(
                "chroma",
                "grade.field.chroma",
                self.chroma,
                0.0,
                1.0,
                format!("{:.2}", self.chroma),
            ),
            field::slider(
                "radius",
                "grade.field.radius",
                self.radius,
                0.002,
                0.05,
                format!("{:.3}", self.radius),
            ),
            field::slider(
                "eps",
                "Epsilon",
                self.eps,
                1e-4,
                1e-1,
                format!("{:.4}", self.eps),
            ),
        ]
    }

    pub(super) fn render_frame(&self, input: &Frame) -> Frame {
        if self.luma <= 0.0 && self.chroma <= 0.0 {
            return input.clone_frame();
        }
        let (w, h) = (input.width as usize, input.height as usize);
        let y = exec::log_luma(input);
        let r = exec::radius_px(self.radius, w, h);
        let base = if self.luma > 0.0 {
            exec::guided(&y, w, h, r, self.eps)
        } else {
            y.clone()
        };
        let mut cr = vec![1f32; w * h];
        let mut cg = vec![1f32; w * h];
        let mut cb = vec![1f32; w * h];
        let mut yy = vec![0f32; w * h];
        for (i, px) in input.rgb.chunks_exact(3).enumerate() {
            let yv = luma(px[0], px[1], px[2]).max(0.0);
            yy[i] = yv;
            if yv > 1e-6 {
                cr[i] = px[0] / yv;
                cg[i] = px[1] / yv;
                cb[i] = px[2] / yv;
            }
        }
        let (cr, cg, cb) = if self.chroma > 0.0 {
            (
                exec::box_blur(&cr, w, h, r),
                exec::box_blur(&cg, w, h, r),
                exec::box_blur(&cb, w, h, r),
            )
        } else {
            (cr, cg, cb)
        };
        let mut rgb = vec![0f32; input.rgb.len()];
        let luma_mix = self.luma;
        let chroma = self.chroma;
        exec::for_rows(h, w * 3, &mut rgb, |row_i, row| {
            for x in 0..w {
                let i = row_i * w + x;
                let px = &input.rgb[i * 3..i * 3 + 3];
                if yy[i] < 1e-6 {
                    row[x * 3..x * 3 + 3].copy_from_slice(px);
                    continue;
                }
                let lp = base[i] + (y[i] - base[i]) * (1.0 - luma_mix);
                let y_out = 2f32.powf(lp);
                let c0 = px[0] / yy[i];
                let c1 = px[1] / yy[i];
                let c2 = px[2] / yy[i];
                row[x * 3] = y_out * (c0 + (cr[i] - c0) * chroma);
                row[x * 3 + 1] = y_out * (c1 + (cg[i] - c1) * chroma);
                row[x * 3 + 2] = y_out * (c2 + (cb[i] - c2) * chroma);
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
