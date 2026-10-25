use serde_json::{Value, json};

use super::{Node, NodeParam, object, req_str, text};
use crate::grade::buffer::{self, InputDesc, OutputDesc};
use crate::grade::exec::{Frame, map_rgb};
use crate::grade::field::{self, FieldSpec};

#[derive(Clone, Copy)]
enum Method {
    Bilinear,
    Rcd,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Highlight {
    Clip,
    Reconstruct,
}

#[derive(Clone)]
pub struct Demosaic {
    method: Method,
    highlight: Highlight,
}

impl Demosaic {
    pub(crate) fn new() -> Self {
        Self {
            method: Method::Bilinear,
            highlight: Highlight::Clip,
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        Ok(Self {
            method: match req_str(obj, "method")? {
                "bilinear" => Method::Bilinear,
                "rcd" => Method::Rcd,
                other => return Err(format!("unknown demosaic method {other}")),
            },
            highlight: match req_str(obj, "highlight")? {
                "clip" => Highlight::Clip,
                "reconstruct" => Highlight::Reconstruct,
                other => return Err(format!("unknown highlight mode {other}")),
            },
        })
    }
}

impl NodeParam for Demosaic {
    fn node_type(&self) -> &'static str {
        "demosaic"
    }

    fn always_on(&self) -> bool {
        true
    }

    fn to_json(&self) -> Value {
        json!({
            "method": match self.method {
                Method::Bilinear => "bilinear",
                Method::Rcd => "rcd",
            },
            "highlight": match self.highlight {
                Highlight::Clip => "clip",
                Highlight::Reconstruct => "reconstruct",
            },
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

impl Node for Demosaic {
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
        vec![buffer::image_out(buffer::LINEAR_RGB)]
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

impl Demosaic {
    pub(super) fn ui_fields(&self, locale: &str) -> Vec<FieldSpec> {
        let zh = locale.starts_with("zh");
        vec![
            field::choice(
                "method",
                text(zh, "Method", "方法"),
                match self.method {
                    Method::Bilinear => 0,
                    Method::Rcd => 1,
                },
                &["bilinear", "rcd"],
                zh,
                &["双线性", "RCD"],
            ),
            field::choice(
                "highlight",
                text(zh, "Highlights", "高光"),
                match self.highlight {
                    Highlight::Clip => 0,
                    Highlight::Reconstruct => 1,
                },
                &["clip", "reconstruct"],
                zh,
                &["裁切", "重建"],
            ),
        ]
    }

    pub(super) fn render_frame(&self, input: &Frame) -> Frame {
        let _ = self.method;
        if self.highlight == Highlight::Reconstruct {
            return input.clone_frame();
        }
        map_rgb(input, input.space, |r, g, b| {
            [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)]
        })
    }
}
