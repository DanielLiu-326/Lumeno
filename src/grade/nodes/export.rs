use serde_json::{Value, json};

use super::{Node, NodeParam, clamp_f, object, req_f, req_str, text};
use crate::grade::buffer::{self, InputDesc, OutputDesc};
use crate::grade::field::{self, FieldSpec};

#[derive(Clone, Copy)]
enum Format {
    Jpeg,
    Webp,
    Png,
    Tiff,
}

#[derive(Clone)]
pub struct Export {
    format: Format,
    quality: u8,
    long_edge: u32,
}

impl Export {
    pub(crate) fn new() -> Self {
        Self {
            format: Format::Jpeg,
            quality: 90,
            long_edge: 0,
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        Ok(Self {
            format: match req_str(obj, "format")? {
                "jpeg" => Format::Jpeg,
                "webp" => Format::Webp,
                "png" => Format::Png,
                "tiff" => Format::Tiff,
                other => return Err(format!("unknown export format {other}")),
            },
            quality: clamp_f(req_f(obj, "quality")?, 1.0, 100.0).round() as u8,
            long_edge: clamp_f(req_f(obj, "long_edge")?, 0.0, 4096.0).round() as u32,
        })
    }
}

impl NodeParam for Export {
    fn node_type(&self) -> &'static str {
        "export"
    }

    fn always_on(&self) -> bool {
        true
    }

    fn to_json(&self) -> Value {
        json!({
            "format": match self.format {
                Format::Jpeg => "jpeg",
                Format::Webp => "webp",
                Format::Png => "png",
                Format::Tiff => "tiff",
            },
            "quality": self.quality,
            "long_edge": self.long_edge,
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

impl Node for Export {
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
        vec![buffer::image_in(buffer::DISPLAY_RGB)]
    }

    fn outputs(_param: &dyn NodeParam) -> Vec<OutputDesc>
    where
        Self: Sized,
    {
        Vec::new()
    }

    fn run(
        &self,
        _inputs: &std::collections::BTreeMap<String, std::sync::Arc<crate::grade::buffer::Value>>,
        _ctx: &crate::grade::exec::ExecCtx,
    ) -> Result<
        std::collections::BTreeMap<String, crate::grade::buffer::Value>,
        crate::grade::error::GradeError,
    > {
        Ok(std::collections::BTreeMap::new())
    }

    fn defaults(&self) -> Box<dyn Node> {
        Box::new(Self::new())
    }
}

impl Export {
    pub(super) fn ui_fields(&self, locale: &str) -> Vec<FieldSpec> {
        let zh = locale.starts_with("zh");
        vec![
            field::choice(
                "format",
                text(zh, "Format", "格式"),
                match self.format {
                    Format::Jpeg => 0,
                    Format::Webp => 1,
                    Format::Png => 2,
                    Format::Tiff => 3,
                },
                &["jpeg", "webp", "png", "tiff"],
                zh,
                &["JPEG", "WebP", "PNG", "TIFF"],
            ),
            field::slider(
                "quality",
                text(zh, "Quality", "质量"),
                self.quality as f32,
                1.0,
                100.0,
                format!("{}", self.quality),
            ),
            field::slider(
                "long_edge",
                text(zh, "Long edge", "长边"),
                self.long_edge as f32,
                0.0,
                4096.0,
                if self.long_edge == 0 {
                    text(zh, "Full", "原始")
                } else {
                    format!("{}px", self.long_edge)
                },
            ),
        ]
    }
}
