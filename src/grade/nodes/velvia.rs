use serde_json::{Value, json};

use super::{Node, NodeParam, clamp_f, object, req_f, text};
use crate::grade::buffer::{InputDesc, OutputDesc};
use crate::grade::color::luma;
use crate::grade::exec::{Frame, map_rgb};
use crate::grade::field::{self, FieldSpec};

#[derive(Clone)]
pub struct Velvia {
    strength: f32,
    bias: f32,
}

impl Velvia {
    pub(crate) fn new() -> Self {
        Self {
            strength: 0.0,
            bias: 1.0,
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        Ok(Self {
            strength: clamp_f(req_f(obj, "strength")?, 0.0, 1.0),
            bias: clamp_f(req_f(obj, "bias")?, 0.25, 4.0),
        })
    }
}

impl NodeParam for Velvia {
    fn node_type(&self) -> &'static str {
        "velvia"
    }

    fn to_json(&self) -> Value {
        json!({ "strength": self.strength, "bias": self.bias })
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

impl Node for Velvia {
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

impl Velvia {
    pub(super) fn ui_fields(&self, locale: &str) -> Vec<FieldSpec> {
        let zh = locale.starts_with("zh");
        vec![
            field::slider(
                "strength",
                text(zh, "Strength", "强度"),
                self.strength,
                0.0,
                1.0,
                format!("{:.2}", self.strength),
            ),
            field::slider(
                "bias",
                text(zh, "Bias", "偏向"),
                self.bias,
                0.25,
                4.0,
                format!("{:.2}", self.bias),
            ),
        ]
    }

    pub(super) fn render_frame(&self, input: &Frame) -> Frame {
        if self.strength <= 0.0 {
            return input.clone_frame();
        }
        let strength = self.strength;
        let bias = self.bias;
        map_rgb(input, input.space, |r, g, b| {
            let y = luma(r, g, b);
            let mx = r.max(g).max(b);
            if mx < 1e-6 {
                return [r, g, b];
            }
            let mn = r.min(g).min(b);
            let sat = (mx - mn) / mx.max(1e-6);
            let weight = strength * (1.0 - sat).clamp(0.0, 1.0).powf(bias);
            [
                y + (r - y) * (1.0 + weight),
                y + (g - y) * (1.0 + weight),
                y + (b - y) * (1.0 + weight),
            ]
        })
    }
}
