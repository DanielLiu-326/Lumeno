//! Each concrete type implements [`NodeParam`] and [`Node`]. The traits are
//! separate: a node exposes its parameters through [`Node::params`]. Construction
//! from a type name is the only match left in this module.

mod crop;
mod dehaze;
mod demosaic;
mod denoise;
mod export;
mod exposure;
mod input;
mod input_color_profile;
mod local_contrast;
mod lut;
mod rotate;
mod sharpen;
mod sigmoid;
mod velvia;
mod white_balance;

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Map, Value};

use super::buffer::{self, InputDesc, OutputDesc};
use super::error::GradeError;
use super::exec::{self, ExecCtx, Frame};
use super::field::FieldSpec;

/// Parameters of one node. Ports are not here: they are read from these
/// parameters by [`Node::inputs`] and [`Node::outputs`].
pub trait NodeParam: Any + Send + Sync {
    fn node_type(&self) -> &'static str;

    fn to_json(&self) -> Value;

    /// This node stays in the pipeline. The default can be switched off.
    fn always_on(&self) -> bool {
        false
    }

    fn fields(&self, _locale: &str) -> Vec<FieldSpec> {
        Vec::new()
    }

    /// Build a new parameter value. Not on the `dyn NodeParam` vtable.
    fn from_json(value: &Value) -> Result<Self, String>
    where
        Self: Sized;

    /// Write parameters into this value. The node object stays.
    fn set_json(&mut self, value: &Value) -> Result<(), String>;
}

/// One grade node. `inputs` and `outputs` are associated functions: only the
/// parameters decide the ports, and `where Self: Sized` keeps them off the
/// `dyn Node` vtable.
pub trait Node: Send + Sync {
    fn params(&self) -> &dyn NodeParam;

    fn inputs(param: &dyn NodeParam) -> Vec<InputDesc>
    where
        Self: Sized;

    fn outputs(param: &dyn NodeParam) -> Vec<OutputDesc>
    where
        Self: Sized;

    /// Pixel or geometry step. Input and Export do not use this.
    fn render(param: &dyn NodeParam, frame: &Frame) -> Frame
    where
        Self: Sized,
    {
        let _ = param;
        frame.clone_frame()
    }

    fn run(
        &self,
        inputs: &BTreeMap<String, Arc<buffer::Value>>,
        ctx: &ExecCtx,
    ) -> Result<BTreeMap<String, buffer::Value>, GradeError>;

    fn set_param(&mut self, value: &Value) -> Result<(), String>;

    fn defaults(&self) -> Box<dyn Node>;
}

pub(crate) fn input(raw: bool) -> Box<dyn Node> {
    Box::new(input::Input::new(raw))
}

pub(crate) fn white_balance() -> Box<dyn Node> {
    Box::new(white_balance::WhiteBalance::new())
}

pub(crate) fn demosaic() -> Box<dyn Node> {
    Box::new(demosaic::Demosaic::new())
}

pub(crate) fn input_color_profile(camera: Option<([f32; 9], String)>) -> Box<dyn Node> {
    Box::new(input_color_profile::InputColorProfile::new(camera))
}

pub(crate) fn exposure() -> Box<dyn Node> {
    Box::new(exposure::Exposure::new())
}

pub(crate) fn exposure_at(ev: f32) -> Box<dyn Node> {
    Box::new(exposure::Exposure::at(ev))
}

pub(crate) fn local_contrast() -> Box<dyn Node> {
    Box::new(local_contrast::LocalContrast::new())
}

pub(crate) fn dehaze() -> Box<dyn Node> {
    Box::new(dehaze::Dehaze::new())
}

pub(crate) fn velvia() -> Box<dyn Node> {
    Box::new(velvia::Velvia::new())
}

pub(crate) fn denoise() -> Box<dyn Node> {
    Box::new(denoise::Denoise::new())
}

pub(crate) fn sharpen() -> Box<dyn Node> {
    Box::new(sharpen::Sharpen::new())
}

pub(crate) fn rotate() -> Box<dyn Node> {
    Box::new(rotate::Rotate::new())
}

pub(crate) fn crop() -> Box<dyn Node> {
    Box::new(crop::Crop::new())
}

pub(crate) fn sigmoid() -> Box<dyn Node> {
    Box::new(sigmoid::Sigmoid::new())
}

pub(crate) fn lut() -> Box<dyn Node> {
    Box::new(lut::Lut::new())
}

pub(crate) fn export() -> Box<dyn Node> {
    Box::new(export::Export::new())
}

pub(crate) fn param_from_json(node_type: &str, value: &Value) -> Result<Box<dyn Node>, GradeError> {
    let parsed = match node_type {
        "input" => input::Input::from_json(value).map(|node| Box::new(node) as Box<dyn Node>),
        "white-balance" => white_balance::WhiteBalance::from_json(value)
            .map(|node| Box::new(node) as Box<dyn Node>),
        "demosaic" => {
            demosaic::Demosaic::from_json(value).map(|node| Box::new(node) as Box<dyn Node>)
        }
        "input-color-profile" => input_color_profile::InputColorProfile::from_json(value)
            .map(|node| Box::new(node) as Box<dyn Node>),
        "exposure" => {
            exposure::Exposure::from_json(value).map(|node| Box::new(node) as Box<dyn Node>)
        }
        "local-contrast" => local_contrast::LocalContrast::from_json(value)
            .map(|node| Box::new(node) as Box<dyn Node>),
        "dehaze" => dehaze::Dehaze::from_json(value).map(|node| Box::new(node) as Box<dyn Node>),
        "velvia" => velvia::Velvia::from_json(value).map(|node| Box::new(node) as Box<dyn Node>),
        "denoise" => denoise::Denoise::from_json(value).map(|node| Box::new(node) as Box<dyn Node>),
        "sharpen" => sharpen::Sharpen::from_json(value).map(|node| Box::new(node) as Box<dyn Node>),
        "rotate" => rotate::Rotate::from_json(value).map(|node| Box::new(node) as Box<dyn Node>),
        "crop" => crop::Crop::from_json(value).map(|node| Box::new(node) as Box<dyn Node>),
        "sigmoid" => sigmoid::Sigmoid::from_json(value).map(|node| Box::new(node) as Box<dyn Node>),
        "lut" => lut::Lut::from_json(value).map(|node| Box::new(node) as Box<dyn Node>),
        "export" => export::Export::from_json(value).map(|node| Box::new(node) as Box<dyn Node>),
        other => {
            return Err(GradeError::InvalidDocument(format!(
                "unknown node type {other}"
            )));
        }
    };
    parsed.map_err(GradeError::InvalidDocument)
}

fn image_frame(inputs: &BTreeMap<String, Arc<super::buffer::Value>>) -> Option<Frame> {
    inputs
        .get("image")
        .and_then(|value| exec::frame_from_value(value))
}

pub(super) fn param_as<T: NodeParam + 'static>(param: &dyn NodeParam) -> &T {
    (param as &dyn Any)
        .downcast_ref::<T>()
        .expect("grade node parameter")
}

pub(super) fn run_rendered<N: Node>(
    param: &dyn NodeParam,
    inputs: &BTreeMap<String, Arc<buffer::Value>>,
) -> Result<BTreeMap<String, buffer::Value>, GradeError> {
    let Some(frame) = image_frame(inputs) else {
        return Ok(BTreeMap::new());
    };
    let mut outputs = BTreeMap::new();
    outputs.insert(
        "image".to_string(),
        exec::value_from_frame(N::render(param, &frame)),
    );
    Ok(outputs)
}

pub(super) fn linear_mask_in() -> Vec<InputDesc> {
    vec![buffer::image_in(buffer::LINEAR_RGB), buffer::mask_in()]
}

pub(super) fn linear_image_out() -> Vec<OutputDesc> {
    vec![buffer::image_out(buffer::LINEAR_RGB)]
}

pub(super) fn geometry_out() -> Vec<OutputDesc> {
    vec![buffer::image_out(buffer::LINEAR_RGB), buffer::mask_out()]
}

pub(super) fn object(params: &Value) -> Result<&Map<String, Value>, String> {
    params
        .as_object()
        .ok_or_else(|| "params must be an object".to_string())
}

pub(super) fn req_f(obj: &Map<String, Value>, key: &str) -> Result<f32, String> {
    obj.get(key)
        .and_then(Value::as_f64)
        .map(|n| n as f32)
        .or_else(|| obj.get(key).and_then(Value::as_i64).map(|n| n as f32))
        .or_else(|| obj.get(key).and_then(Value::as_u64).map(|n| n as f32))
        .ok_or_else(|| format!("missing {key}"))
}

pub(super) fn req_str<'a>(obj: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    obj.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing {key}"))
}

pub(super) fn req_bool(obj: &Map<String, Value>, key: &str) -> Result<bool, String> {
    obj.get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("missing {key}"))
}

pub(super) fn clamp_f(value: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        min
    }
}

pub(super) fn wrap_degrees(value: f32) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }
    let mut d = value % 360.0;
    if d > 180.0 {
        d -= 360.0;
    }
    if d < -180.0 {
        d += 360.0;
    }
    d.clamp(-180.0, 180.0)
}

pub(super) fn text(zh: bool, en: &str, cn: &str) -> String {
    (if zh { cn } else { en }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grade::buffer::DataType;

    #[test]
    fn ports_follow_the_parameters() {
        let exposure = exposure::Exposure::new();
        let inputs = <exposure::Exposure as Node>::inputs(&exposure);
        assert!(
            inputs
                .iter()
                .any(|port| port.name == "image" && port.types.contains(&DataType::LinearRgb))
        );
        assert!(
            inputs
                .iter()
                .any(|port| port.name == "mask" && port.types.contains(&DataType::Mask))
        );
        let outputs = <exposure::Exposure as Node>::outputs(&exposure);
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].name, "image");
        let sigmoid = sigmoid::Sigmoid::new();
        assert!(
            <sigmoid::Sigmoid as Node>::outputs(&sigmoid)[0]
                .types
                .contains(&DataType::DisplayRgb)
        );
        let export = export::Export::new();
        assert!(<export::Export as Node>::outputs(&export).is_empty());
        assert!(
            <export::Export as Node>::inputs(&export)[0]
                .types
                .contains(&DataType::DisplayRgb)
        );
        let input = input::Input::new(false);
        assert!(<input::Input as Node>::inputs(&input).is_empty());
        let produced = <input::Input as Node>::outputs(&input);
        assert!(produced[0].types.contains(&DataType::Mosaic));
        assert!(produced[0].types.contains(&DataType::LinearRgb));
    }
}
