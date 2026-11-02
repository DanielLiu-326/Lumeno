use serde_json::{Value, json};

use super::{Node, NodeParam, clamp_f, object, req_f};
use crate::grade::buffer::{InputDesc, OutputDesc};
use crate::grade::exec::Frame;
use crate::grade::field::{self, FieldSpec};

#[derive(Clone)]
pub struct Crop {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Crop {
    pub(crate) fn new() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        let x = clamp_f(req_f(obj, "x")?, 0.0, 1.0);
        let y = clamp_f(req_f(obj, "y")?, 0.0, 1.0);
        let w = clamp_f(req_f(obj, "w")?, 0.01, 1.0);
        let h = clamp_f(req_f(obj, "h")?, 0.01, 1.0);
        Ok(Self {
            x,
            y,
            w: w.min(1.0 - x).max(0.01),
            h: h.min(1.0 - y).max(0.01),
        })
    }
}

impl NodeParam for Crop {
    fn node_type(&self) -> &'static str {
        "crop"
    }

    fn to_json(&self) -> Value {
        json!({ "x": self.x, "y": self.y, "w": self.w, "h": self.h })
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

impl Node for Crop {
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
        super::geometry_out()
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

impl Crop {
    pub(super) fn ui_fields(&self) -> Vec<FieldSpec> {
        vec![
            field::slider(
                "x",
                "X",
                self.x,
                0.0,
                1.0,
                format!("{:.0}%", self.x * 100.0),
            ),
            field::slider(
                "y",
                "Y",
                self.y,
                0.0,
                1.0,
                format!("{:.0}%", self.y * 100.0),
            ),
            field::slider(
                "w",
                "grade.field.width",
                self.w,
                0.05,
                1.0,
                format!("{:.0}%", self.w * 100.0),
            ),
            field::slider(
                "h",
                "grade.field.height",
                self.h,
                0.05,
                1.0,
                format!("{:.0}%", self.h * 100.0),
            ),
        ]
    }

    pub(super) fn render_frame(&self, input: &Frame) -> Frame {
        let mut frame = input.clone_frame();
        frame.roi.x += self.x * input.roi.w;
        frame.roi.y += self.y * input.roi.h;
        frame.roi.w *= self.w;
        frame.roi.h *= self.h;
        frame
    }
}
