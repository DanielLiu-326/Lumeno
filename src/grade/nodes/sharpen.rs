use std::sync::Arc;

use serde_json::{Value, json};

use super::{Node, NodeParam, clamp_f, object, req_f, text};
use crate::grade::buffer::{InputDesc, OutputDesc};
use crate::grade::exec::{self, Frame};
use crate::grade::field::{self, FieldSpec};

#[derive(Clone)]
pub struct Sharpen {
    amount: f32,
    radius: f32,
    threshold: f32,
}

impl Sharpen {
    pub(crate) fn new() -> Self {
        Self {
            amount: 0.0,
            radius: 0.003,
            threshold: 0.0,
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        Ok(Self {
            amount: clamp_f(req_f(obj, "amount")?, 0.0, 3.0),
            radius: clamp_f(req_f(obj, "radius")?, 0.001, 0.02),
            threshold: clamp_f(req_f(obj, "threshold")?, 0.0, 0.2),
        })
    }
}

impl NodeParam for Sharpen {
    fn node_type(&self) -> &'static str {
        "sharpen"
    }

    fn to_json(&self) -> Value {
        json!({
            "amount": self.amount,
            "radius": self.radius,
            "threshold": self.threshold,
        })
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

impl Node for Sharpen {
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

impl Sharpen {
    pub(super) fn ui_fields(&self, locale: &str) -> Vec<FieldSpec> {
        let zh = locale.starts_with("zh");
        vec![
            field::slider(
                "amount",
                text(zh, "Amount", "数量"),
                self.amount,
                0.0,
                3.0,
                format!("{:.2}", self.amount),
            ),
            field::slider(
                "radius",
                text(zh, "Radius", "半径"),
                self.radius,
                0.001,
                0.02,
                format!("{:.3}", self.radius),
            ),
            field::slider(
                "threshold",
                text(zh, "Threshold", "阈值"),
                self.threshold,
                0.0,
                0.2,
                format!("{:.3}", self.threshold),
            ),
        ]
    }

    pub(super) fn render_frame(&self, input: &Frame) -> Frame {
        if self.amount <= 0.0 {
            return input.clone_frame();
        }
        let (w, h) = (input.width as usize, input.height as usize);
        let y = exec::log_luma(input);
        let sigma = (self.radius * w.min(h) as f32).max(0.5);
        let blur = exec::gaussian(&y, w, h, sigma);
        let mut rgb = vec![0f32; input.rgb.len()];
        let amount = self.amount;
        let threshold = self.threshold;
        exec::for_rows(h, w * 3, &mut rgb, |row_i, row| {
            for x in 0..w {
                let i = row_i * w + x;
                let px = &input.rgb[i * 3..i * 3 + 3];
                let mut detail = y[i] - blur[i];
                if detail.abs() < threshold {
                    detail = 0.0;
                }
                let scale = 2f32.powf(amount * detail);
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
