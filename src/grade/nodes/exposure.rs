use serde_json::{Value, json};

use super::{Node, NodeParam, clamp_f, object, req_f};
use crate::grade::buffer::{InputDesc, OutputDesc};
use crate::grade::exec::{Frame, map_rgb};
use crate::grade::field::{self, FieldSpec};

#[derive(Clone)]
pub struct Exposure {
    ev: f32,
    black: f32,
    /// EV restored by reset. For RAW this is the camera baseline, not zero.
    home_ev: f32,
}

impl Exposure {
    pub(crate) fn new() -> Self {
        Self::at(0.0)
    }

    pub(crate) fn at(ev: f32) -> Self {
        let ev = ev.clamp(-5.0, 5.0);
        Self {
            ev,
            black: 0.0,
            home_ev: ev,
        }
    }

    pub(super) fn home_ev(&self) -> f32 {
        self.home_ev
    }

    pub(super) fn restored(&self) -> Self {
        Self {
            ev: self.home_ev,
            black: 0.0,
            home_ev: self.home_ev,
        }
    }

    pub(super) fn retain_home(&mut self, home_ev: f32) {
        self.home_ev = home_ev.clamp(-5.0, 5.0);
    }

    pub(super) fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        let ev = clamp_f(req_f(obj, "ev")?, -5.0, 5.0);
        Ok(Self {
            ev,
            black: clamp_f(req_f(obj, "black")?, -0.1, 0.1),
            home_ev: ev,
        })
    }
}

impl NodeParam for Exposure {
    fn node_type(&self) -> &'static str {
        "exposure"
    }

    fn to_json(&self) -> Value {
        json!({ "ev": self.ev, "black": self.black })
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
        let home = self.home_ev();
        *self = Self::from_json(value)?;
        self.retain_home(home);
        Ok(())
    }
}

impl Node for Exposure {
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
        Box::new(self.restored())
    }
}

impl Exposure {
    pub(super) fn ui_fields(&self) -> Vec<FieldSpec> {
        vec![
            field::slider(
                "ev",
                "grade.field.exposure",
                self.ev,
                -5.0,
                5.0,
                format!("{:+.2} EV", self.ev),
            ),
            field::slider(
                "black",
                "grade.field.black",
                self.black,
                -0.1,
                0.1,
                format!("{:+.3}", self.black),
            ),
        ]
    }

    pub(super) fn render_frame(&self, input: &Frame) -> Frame {
        let ev = self.ev;
        let black = self.black;
        map_rgb(input, input.space, |r, g, b| {
            [
                r * 2f32.powf(ev) - black,
                g * 2f32.powf(ev) - black,
                b * 2f32.powf(ev) - black,
            ]
        })
    }
}
