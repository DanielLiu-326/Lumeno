use std::sync::Arc;

use serde_json::{Value, json};

use super::{Node, NodeParam, clamp_f, object, req_f, text};
use crate::grade::buffer::{InputDesc, OutputDesc};
use crate::grade::exec::{self, Frame};
use crate::grade::field::{self, FieldSpec};

#[derive(Clone)]
pub struct LocalContrast {
    strength: f32,
    radius: f32,
    eps: f32,
}

impl LocalContrast {
    pub(crate) fn new() -> Self {
        Self {
            strength: 0.0,
            radius: 0.04,
            eps: 0.02,
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        Ok(Self {
            strength: clamp_f(req_f(obj, "strength")?, 0.0, 2.0),
            radius: clamp_f(req_f(obj, "radius")?, 0.005, 0.2),
            eps: clamp_f(req_f(obj, "eps")?, 1e-3, 1.0),
        })
    }
}

impl NodeParam for LocalContrast {
    fn node_type(&self) -> &'static str {
        "local-contrast"
    }

    fn to_json(&self) -> Value {
        json!({ "strength": self.strength, "radius": self.radius, "eps": self.eps })
    }

    fn fields(&self, locale: &str) -> Vec<FieldSpec> {
        self.ui_fields(locale)
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

impl Node for LocalContrast {
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

impl LocalContrast {
    pub(super) fn ui_fields(&self, locale: &str) -> Vec<FieldSpec> {
        let zh = locale.starts_with("zh");
        vec![
            field::slider(
                "strength",
                text(zh, "Strength", "强度"),
                self.strength,
                0.0,
                2.0,
                format!("{:.2}", self.strength),
            ),
            field::slider(
                "radius",
                text(zh, "Radius", "半径"),
                self.radius,
                0.005,
                0.2,
                format!("{:.3}", self.radius),
            ),
            field::slider(
                "eps",
                "Epsilon",
                self.eps,
                1e-3,
                1.0,
                format!("{:.3}", self.eps),
            ),
        ]
    }

    pub(super) fn render_frame(&self, input: &Frame) -> Frame {
        if self.strength <= 0.0 {
            return input.clone_frame();
        }
        let (w, h) = (input.width as usize, input.height as usize);
        let y = exec::log_luma(input);
        let r = exec::radius_px(self.radius, w, h);
        let base = exec::guided(&y, w, h, r, self.eps);
        let mut rgb = vec![0f32; input.rgb.len()];
        let strength = self.strength;
        exec::for_rows(h, w * 3, &mut rgb, |row_i, row| {
            for x in 0..w {
                let i = row_i * w + x;
                let px = &input.rgb[i * 3..i * 3 + 3];
                let detail = y[i] - base[i];
                let scale = 2f32.powf(strength * detail);
                row[x * 3] = px[0] * scale;
                row[x * 3 + 1] = px[1] * scale;
                row[x * 3 + 2] = px[2] * scale;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grade::exec::{Roi, Space, ViewTransform};

    #[test]
    fn strength_lifts_a_small_bright_patch() {
        let (w, h) = (32u32, 32u32);
        let mut rgb = vec![0.2f32; (w * h * 3) as usize];
        for y in 14..18 {
            for x in 14..18 {
                let i = ((y * w + x) * 3) as usize;
                rgb[i] = 0.55;
                rgb[i + 1] = 0.55;
                rgb[i + 2] = 0.55;
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
        let out = LocalContrast {
            strength: 1.0,
            radius: 0.2,
            eps: 0.02,
        }
        .render_frame(&frame);
        let center = out.rgb[((16 * w + 16) * 3) as usize];
        let corner = out.rgb[0];
        assert!(center > 0.55, "patch should brighten, got {center}");
        assert!(
            (corner - 0.2).abs() < 0.05,
            "flat surround should stay near 0.2, got {corner}"
        );
    }
}
