//! Display-referred 3D LUT. The cube lives in the resource library. The node samples the
//! table the host has already loaded for this id; it does not open a file path.

use serde_json::{Value, json};

use super::{Node, NodeParam, clamp_f, object, req_f, req_str};
use crate::grade::buffer::{InputDesc, OutputDesc};
use crate::grade::color::{
    self, adobe_to_linear, linear_to_adobe, linear_to_rec709, linear_to_srgb, rec709_to_linear,
    srgb_to_linear,
};
use crate::grade::cube::lookup_lut;
use crate::grade::exec::Frame;
use crate::grade::exec::Space;
use crate::grade::field::{self, FieldSpec};

#[derive(Clone)]
pub struct Lut {
    /// `lut:<hash>`, or empty when none is chosen.
    lut: String,
    strength: f32,
    /// Application color space. The cube is sampled there, then converted back.
    encoding: String,
}

impl Lut {
    pub(crate) fn new() -> Self {
        Self {
            lut: String::new(),
            strength: 1.0,
            encoding: "srgb".to_string(),
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        let encoding = req_str(obj, "encoding")?;
        if !is_apply_space(encoding) {
            return Err("bad encoding".into());
        }
        Ok(Self {
            lut: req_str(obj, "lut")?.to_string(),
            strength: clamp_f(req_f(obj, "strength")?, 0.0, 1.0),
            encoding: encoding.to_string(),
        })
    }
}

impl NodeParam for Lut {
    fn node_type(&self) -> &'static str {
        "lut"
    }

    fn to_json(&self) -> Value {
        json!({
            "lut": self.lut,
            "strength": self.strength,
            "encoding": self.encoding,
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

impl Node for Lut {
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
        vec![
            crate::grade::buffer::image_in(crate::grade::buffer::DISPLAY_RGB),
            crate::grade::buffer::mask_in(),
        ]
    }

    fn outputs(_param: &dyn NodeParam) -> Vec<OutputDesc>
    where
        Self: Sized,
    {
        vec![crate::grade::buffer::image_out(
            crate::grade::buffer::DISPLAY_RGB,
        )]
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

impl Lut {
    pub(super) fn ui_fields(&self) -> Vec<FieldSpec> {
        let name = if self.lut.is_empty() {
            "grade.value.none".to_string()
        } else {
            self.lut.clone()
        };
        vec![
            field::readonly("lut", "grade.field.lut", name),
            field::slider(
                "strength",
                "grade.field.strength",
                self.strength,
                0.0,
                1.0,
                format!("{:.2}", self.strength),
            ),
            field::choice(
                "encoding",
                "grade.field.color_space",
                space_index(&self.encoding),
                &[
                    "sRGB|srgb",
                    "Adobe RGB|adobe-rgb",
                    "gamma Rec709 RGB|gamma-rec709",
                    "linear Rec709 RGB|linear-rec709",
                    "linear Rec2020 RGB|linear-rec2020",
                    "linear ProPhoto RGB|linear-prophoto",
                ],
                &[
                    "grade.option.srgb",
                    "grade.option.adobe_rgb",
                    "grade.option.gamma_rec709",
                    "grade.option.linear_rec709",
                    "grade.option.linear_rec2020",
                    "grade.option.linear_prophoto",
                ],
            ),
        ]
    }

    pub(super) fn render_frame(&self, input: &Frame) -> Frame {
        if self.strength <= 0.0 || self.lut.is_empty() {
            return input.clone_frame();
        }
        let Some(table) = lookup_lut(&self.lut) else {
            return input.clone_frame();
        };
        let strength = self.strength;
        let encoding = self.encoding.clone();
        let buffer = input.space;
        crate::grade::exec::map_rgb(input, input.space, move |r, g, b| {
            let sample_in = into_apply_space(&encoding, buffer, r, g, b);
            let [lr, lg, lb] = table.sample(sample_in[0], sample_in[1], sample_in[2]);
            let [dr, dg, db] = from_apply_space(&encoding, buffer, lr, lg, lb);
            [
                r + strength * (dr - r),
                g + strength * (dg - g),
                b + strength * (db - b),
            ]
        })
    }
}

fn is_apply_space(encoding: &str) -> bool {
    matches!(
        encoding,
        "srgb"
            | "adobe-rgb"
            | "gamma-rec709"
            | "linear-rec709"
            | "linear-rec2020"
            | "linear-prophoto"
            | "linear"
    )
}

fn space_index(encoding: &str) -> i32 {
    match encoding {
        "adobe-rgb" => 1,
        "gamma-rec709" => 2,
        "linear-rec709" | "linear" => 3,
        "linear-rec2020" => 4,
        "linear-prophoto" => 5,
        _ => 0,
    }
}

fn working_rec2020(buffer: Space, r: f32, g: f32, b: f32) -> [f32; 3] {
    if buffer == Space::Display {
        color::linear_srgb_to_rec2020(r, g, b)
    } else {
        [r, g, b]
    }
}

fn from_working(buffer: Space, r: f32, g: f32, b: f32) -> [f32; 3] {
    if buffer == Space::Display {
        color::rec2020_to_linear_srgb(r, g, b)
    } else {
        [r, g, b]
    }
}

fn into_apply_space(encoding: &str, buffer: Space, r: f32, g: f32, b: f32) -> [f32; 3] {
    if encoding == "linear" {
        return [r, g, b];
    }
    let [r, g, b] = working_rec2020(buffer, r, g, b);
    match encoding {
        "srgb" => {
            let [r, g, b] = color::rec2020_to_linear_srgb(r, g, b);
            [linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b)]
        }
        "adobe-rgb" => {
            let [r, g, b] = color::apply_matrix(&color::REC2020_TO_ADOBE, r, g, b);
            [linear_to_adobe(r), linear_to_adobe(g), linear_to_adobe(b)]
        }
        "gamma-rec709" => {
            let [r, g, b] = color::rec2020_to_linear_srgb(r, g, b);
            [
                linear_to_rec709(r),
                linear_to_rec709(g),
                linear_to_rec709(b),
            ]
        }
        "linear-rec709" => color::rec2020_to_linear_srgb(r, g, b),
        "linear-prophoto" => color::apply_matrix(&color::REC2020_TO_PROPHOTO, r, g, b),
        _ => [r, g, b],
    }
}

fn from_apply_space(encoding: &str, buffer: Space, r: f32, g: f32, b: f32) -> [f32; 3] {
    if encoding == "linear" {
        return [r, g, b];
    }
    let working = match encoding {
        "srgb" => {
            let [r, g, b] = [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b)];
            color::linear_srgb_to_rec2020(r, g, b)
        }
        "adobe-rgb" => {
            let [r, g, b] = [adobe_to_linear(r), adobe_to_linear(g), adobe_to_linear(b)];
            color::apply_matrix(&color::ADOBE_TO_REC2020, r, g, b)
        }
        "gamma-rec709" => {
            let [r, g, b] = [
                rec709_to_linear(r),
                rec709_to_linear(g),
                rec709_to_linear(b),
            ];
            color::linear_srgb_to_rec2020(r, g, b)
        }
        "linear-rec709" => color::linear_srgb_to_rec2020(r, g, b),
        "linear-prophoto" => color::apply_matrix(&color::PROPHOTO_TO_REC2020, r, g, b),
        _ => [r, g, b],
    };
    from_working(buffer, working[0], working[1], working[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grade::cube::{parse_cube, remember_lut};
    use crate::grade::exec::{Frame, Roi, Space, ViewTransform};
    use std::sync::Arc;

    #[test]
    fn a_loaded_cube_recolors_the_pixel() {
        let mut text = "LUT_3D_SIZE 2\n".to_string();
        for _ in 0..8 {
            text.push_str("1 0 0\n");
        }
        remember_lut(
            "lut:red-test",
            Arc::new(parse_cube(text.as_bytes()).unwrap()),
        );
        let node = Lut {
            lut: "lut:red-test".into(),
            strength: 1.0,
            encoding: "linear".into(),
        };
        let frame = Frame {
            width: 1,
            height: 1,
            space: Space::Display,
            rgb: Arc::new(vec![0.2, 0.3, 0.4]),
            transform: ViewTransform::default(),
            roi: Roi::default(),
        };
        let out = node.render_frame(&frame);
        assert!(out.rgb[0] > 0.9, "{}", out.rgb[0]);
        assert!(out.rgb[1] < 0.05 && out.rgb[2] < 0.05);
    }
}
