use serde_json::{Value, json};

use super::{Node, NodeParam, object, req_str, text};
use crate::grade::buffer::{self, InputDesc, OutputDesc};
use crate::grade::color::apply_matrix;
use crate::grade::exec::{Frame, Roi, Space, ViewTransform, map_rgb};
use crate::grade::field::{self, FieldSpec};

const IDENTITY: [f32; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Embedded,
    Imported,
}

#[derive(Clone)]
pub struct InputColorProfile {
    source: Source,
    /// This frame is linear camera RGB, so the matrix should run.
    camera: bool,
    embedded_name: String,
    embedded: [f32; 9],
    imported_name: String,
    imported: [f32; 9],
}

impl InputColorProfile {
    pub(crate) fn new(camera: Option<([f32; 9], String)>) -> Self {
        let (matrix, name, camera) = match camera {
            Some((matrix, name)) => (matrix, name, true),
            None => (IDENTITY, String::new(), false),
        };
        Self {
            source: Source::Embedded,
            camera,
            embedded_name: name.clone(),
            embedded: matrix,
            imported_name: name,
            imported: matrix,
        }
    }

    pub(super) fn parse_new(params: &Value) -> Result<Self, String> {
        Self::parse(params, None)
    }

    pub(super) fn parse_keeping(current: &Self, params: &Value) -> Result<Self, String> {
        Self::parse(params, Some(current))
    }

    fn parse(params: &Value, current: Option<&Self>) -> Result<Self, String> {
        let obj = object(params)?;
        let source = match req_str(obj, "source")? {
            "embedded" => Source::Embedded,
            "imported" => Source::Imported,
            other => return Err(format!("unknown input color profile source {other}")),
        };
        let imported_name = obj
            .get("imported_name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let imported = match obj.get("imported") {
            Some(value) => matrix_from(value)?,
            None => current.map(|node| node.imported).unwrap_or(IDENTITY),
        };
        let (camera, embedded_name, embedded) = match current {
            Some(node) => (node.camera, node.embedded_name.clone(), node.embedded),
            None => (false, String::new(), IDENTITY),
        };
        Ok(Self {
            source,
            camera,
            embedded_name,
            embedded,
            imported_name,
            imported,
        })
    }

    fn active_matrix(&self) -> [f32; 9] {
        match self.source {
            Source::Embedded => self.embedded,
            Source::Imported => self.imported,
        }
    }

    fn active_name(&self) -> &str {
        match self.source {
            Source::Embedded => &self.embedded_name,
            Source::Imported => &self.imported_name,
        }
    }

    pub(super) fn restored(&self) -> Self {
        let mut next = self.clone();
        next.source = Source::Embedded;
        next.imported = self.embedded;
        next.imported_name = self.embedded_name.clone();
        next
    }
}

impl NodeParam for InputColorProfile {
    fn node_type(&self) -> &'static str {
        "input-color-profile"
    }

    fn always_on(&self) -> bool {
        true
    }

    fn to_json(&self) -> Value {
        json!({
            "source": match self.source {
                Source::Embedded => "embedded",
                Source::Imported => "imported",
            },
            "camera": self.camera,
            "name": self.active_name(),
            "embedded_name": self.embedded_name,
            "embedded": self.embedded.to_vec(),
            "imported_name": self.imported_name,
            "imported": self.imported.to_vec(),
        })
    }

    fn fields(&self, locale: &str) -> Vec<FieldSpec> {
        self.ui_fields(locale)
    }

    fn from_json(value: &Value) -> Result<Self, String>
    where
        Self: Sized,
    {
        Self::parse_new(value)
    }

    fn set_json(&mut self, value: &Value) -> Result<(), String> {
        *self = Self::parse_keeping(self, value)?;
        Ok(())
    }
}

impl Node for InputColorProfile {
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
        vec![buffer::image_in(buffer::LINEAR_RGB)]
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

impl InputColorProfile {
    pub(super) fn ui_fields(&self, locale: &str) -> Vec<FieldSpec> {
        let zh = locale.starts_with("zh");
        if !self.camera {
            return vec![field::readonly(
                "name",
                text(zh, "Profile", "配置"),
                text(zh, "Already linear Rec.2020", "已是线性 Rec.2020"),
            )];
        }
        let name = self.active_name();
        let display = if name.is_empty() {
            text(zh, "Camera", "相机")
        } else {
            name.to_string()
        };
        vec![
            field::choice(
                "source",
                text(zh, "Profile", "配置"),
                match self.source {
                    Source::Embedded => 0,
                    Source::Imported => 1,
                },
                &["Default|embedded", "Imported|imported"],
                zh,
                &["默认", "导入"],
            ),
            field::readonly("name", text(zh, "Name", "名称"), display),
        ]
    }

    pub(super) fn render_frame(&self, input: &Frame) -> Frame {
        if !self.camera || input.space != Space::Camera {
            return input.clone_frame();
        }
        let matrix = self.active_matrix();
        map_rgb(input, Space::Linear, |r, g, b| {
            apply_matrix(&matrix, r, g, b)
        })
    }
}

fn matrix_from(value: &Value) -> Result<[f32; 9], String> {
    let items = value
        .as_array()
        .ok_or_else(|| "matrix must have 9 numbers".to_string())?;
    if items.len() != 9 {
        return Err("matrix must have 9 numbers".to_string());
    }
    let mut out = [0f32; 9];
    for (slot, item) in out.iter_mut().zip(items) {
        let number = item
            .as_f64()
            .ok_or_else(|| "matrix must have 9 numbers".to_string())? as f32;
        if !number.is_finite() || number.abs() > 16.0 {
            return Err("matrix is out of range".to_string());
        }
        *slot = number;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera_frame(rgb: Vec<f32>) -> Frame {
        Frame {
            width: 1,
            height: 1,
            space: Space::Camera,
            rgb: std::sync::Arc::new(rgb),
            transform: ViewTransform::default(),
            roi: Roi::default(),
        }
    }

    #[test]
    fn matrix_runs_on_camera_rgb_only() {
        let node = InputColorProfile::new(Some((
            [2.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 3.0],
            "Body".into(),
        )));
        let out = node.render_frame(&camera_frame(vec![1.0, 0.0, 0.0]));
        assert_eq!(out.space, Space::Linear);
        assert!((out.rgb[0] - 2.0).abs() < 1e-5);
        assert!((out.rgb[2] - 0.0).abs() < 1e-5);
        let already = Frame {
            width: 1,
            height: 1,
            space: Space::Linear,
            rgb: std::sync::Arc::new(vec![1.0, 0.0, 0.0]),
            transform: ViewTransform::default(),
            roi: Roi::default(),
        };
        let same = node.render_frame(&already);
        assert!((same.rgb[0] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn patch_keeps_the_file_matrix() {
        let current = InputColorProfile::new(Some((
            [0.2, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            "Body".into(),
        )));
        let mut params = current.to_json();
        params["embedded"] = json!([9.0, 0.0, 0.0, 0.0, 9.0, 0.0, 0.0, 0.0, 9.0]);
        params["source"] = json!("imported");
        params["imported_name"] = json!("Sheet");
        params["imported"] = json!([0.5, 0.0, 0.0, 0.0, 0.5, 0.0, 0.0, 0.0, 0.5]);
        let next = InputColorProfile::parse_keeping(&current, &params).unwrap();
        let kept = next.to_json();
        assert!((kept["embedded"][0].as_f64().unwrap() - 0.2).abs() < 1e-5);
        assert_eq!(kept["source"], "imported");
        assert_eq!(kept["imported_name"], "Sheet");
        let rendered = next.render_frame(&camera_frame(vec![1.0, 0.0, 0.0]));
        assert!((rendered.rgb[0] - 0.5).abs() < 1e-5);
    }
}
