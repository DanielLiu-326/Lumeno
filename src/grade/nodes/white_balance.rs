use serde_json::{Value, json};

use super::{Node, NodeParam, clamp_f, object, req_f, req_str};
use crate::grade::buffer::{self, InputDesc, OutputDesc};
use crate::grade::color;
use crate::grade::exec::{Frame, map_rgb};
use crate::grade::field::{self, FieldSpec};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    AsShot,
    Temperature,
    Gains,
}

#[derive(Clone)]
pub struct WhiteBalance {
    mode: Mode,
    temperature: f32,
    tint: f32,
    gains: [f32; 3],
}

impl WhiteBalance {
    pub(crate) fn new() -> Self {
        Self {
            mode: Mode::AsShot,
            temperature: 5500.0,
            tint: 0.0,
            gains: [1.0, 1.0, 1.0],
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        Ok(Self {
            mode: match req_str(obj, "mode")? {
                "as_shot" => Mode::AsShot,
                "temperature" => Mode::Temperature,
                "gains" => Mode::Gains,
                other => return Err(format!("unknown white balance mode {other}")),
            },
            temperature: clamp_f(req_f(obj, "temperature")?, 2000.0, 12000.0),
            tint: clamp_f(req_f(obj, "tint")?, -100.0, 100.0),
            gains: [
                clamp_f(req_f(obj, "gain_r")?, 0.05, 20.0),
                clamp_f(req_f(obj, "gain_g")?, 0.05, 20.0),
                clamp_f(req_f(obj, "gain_b")?, 0.05, 20.0),
            ],
        })
    }
}

impl NodeParam for WhiteBalance {
    fn node_type(&self) -> &'static str {
        "white-balance"
    }

    fn to_json(&self) -> Value {
        json!({
            "mode": match self.mode {
                Mode::AsShot => "as_shot",
                Mode::Temperature => "temperature",
                Mode::Gains => "gains",
            },
            "temperature": self.temperature,
            "tint": self.tint,
            "gain_r": self.gains[0],
            "gain_g": self.gains[1],
            "gain_b": self.gains[2],
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

impl Node for WhiteBalance {
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
        vec![buffer::image_in(buffer::MOSAIC)]
    }

    fn outputs(_param: &dyn NodeParam) -> Vec<OutputDesc>
    where
        Self: Sized,
    {
        vec![buffer::image_out(buffer::MOSAIC)]
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

impl WhiteBalance {
    pub(super) fn ui_fields(&self) -> Vec<FieldSpec> {
        let mut fields = vec![field::choice(
            "mode",
            "grade.field.mode",
            match self.mode {
                Mode::AsShot => 0,
                Mode::Temperature => 1,
                Mode::Gains => 2,
            },
            &["as shot|as_shot", "temperature|temperature", "gains|gains"],
            &[
                "grade.option.as_shot",
                "grade.option.temperature",
                "grade.option.gains",
            ],
        )];
        if self.mode == Mode::Temperature {
            fields.push(field::slider(
                "temperature",
                "grade.field.temperature",
                self.temperature,
                2000.0,
                12000.0,
                format!("{:.0} K", self.temperature),
            ));
            fields.push(field::slider(
                "tint",
                "grade.field.tint",
                self.tint,
                -100.0,
                100.0,
                format!("{:.0}", self.tint),
            ));
        }
        if self.mode == Mode::Gains {
            fields.push(field::slider(
                "gain_r",
                "R",
                self.gains[0],
                0.05,
                8.0,
                format!("{:.2}", self.gains[0]),
            ));
            fields.push(field::slider(
                "gain_g",
                "G",
                self.gains[1],
                0.05,
                8.0,
                format!("{:.2}", self.gains[1]),
            ));
            fields.push(field::slider(
                "gain_b",
                "B",
                self.gains[2],
                0.05,
                8.0,
                format!("{:.2}", self.gains[2]),
            ));
        }
        fields
    }

    pub(super) fn render_frame(&self, input: &Frame) -> Frame {
        let gains = match self.mode {
            Mode::AsShot => [1.0, 1.0, 1.0],
            Mode::Temperature => color::kelvin_gains(self.temperature, self.tint),
            Mode::Gains => self.gains,
        };
        map_rgb(input, input.space, |r, g, b| {
            [r * gains[0], g * gains[1], b * gains[2]]
        })
    }
}
