use serde_json::{Value, json};

use super::{Node, NodeParam, object, req_str};
use crate::grade::buffer::{self, InputDesc, OutputDesc};
use crate::grade::field::{self, FieldSpec};

#[derive(Clone)]
pub struct Input {
    source: &'static str,
}

impl Input {
    pub(crate) fn new(raw: bool) -> Self {
        Self {
            source: if raw { "raw" } else { "raster" },
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        let source = match req_str(obj, "source")? {
            "raw" => "raw",
            "raster" => "raster",
            other => return Err(format!("unknown input source {other}")),
        };
        Ok(Self { source })
    }
}

impl NodeParam for Input {
    fn node_type(&self) -> &'static str {
        "input"
    }

    fn to_json(&self) -> Value {
        json!({ "source": self.source })
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
        object(value)?;
        Ok(())
    }
}

impl Node for Input {
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
        Vec::new()
    }

    fn outputs(_param: &dyn NodeParam) -> Vec<OutputDesc>
    where
        Self: Sized,
    {
        vec![buffer::image_out(buffer::MOSAIC_OR_LINEAR)]
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
        Box::new(self.clone())
    }
}

impl Input {
    pub(super) fn ui_fields(&self) -> Vec<FieldSpec> {
        vec![field::readonly("source", "grade.field.source", self.source)]
    }
}
