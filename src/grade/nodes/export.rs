use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::{Value, json};

use super::{Node, NodeParam, object};
use crate::grade::buffer::{self, InputDesc, OutputDesc, Value as BufferValue};
use crate::grade::error::GradeError;
use crate::grade::exec::ExecCtx;
use crate::grade::field::{self, FieldSpec};

/// Terminal marker for the grade graph. It owns the output color space and
/// transfer choice; format, quality and size remain export-request settings.
#[derive(Clone, Copy)]
pub struct Export {
    rec2020: bool,
    transfer: Transfer,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Transfer {
    Srgb,
    Pq,
    Hlg,
}

impl Transfer {
    fn as_str(self) -> &'static str {
        match self {
            Self::Srgb => "srgb",
            Self::Pq => "pq",
            Self::Hlg => "hlg",
        }
    }
}

impl Export {
    pub(crate) fn new() -> Self {
        Self {
            rec2020: false,
            transfer: Transfer::Srgb,
        }
    }

    fn parse(value: &Value) -> Result<Self, String> {
        let obj = object(value)?;
        let rec2020 = match obj.get("color_space").and_then(Value::as_str) {
            Some("rec2020") => true,
            Some("srgb") | None => false,
            Some(other) => return Err(format!("unknown export color space {other}")),
        };
        let transfer = match obj.get("transfer").and_then(Value::as_str) {
            Some("pq") if rec2020 => Transfer::Pq,
            Some("hlg") if rec2020 => Transfer::Hlg,
            Some("srgb") | None => Transfer::Srgb,
            Some("pq") | Some("hlg") => Transfer::Srgb,
            Some(other) => return Err(format!("unknown export transfer {other}")),
        };
        Ok(Self {
            rec2020,
            transfer: if rec2020 && transfer == Transfer::Srgb {
                Transfer::Pq
            } else {
                transfer
            },
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
            "color_space": if self.rec2020 { "rec2020" } else { "srgb" },
            "transfer": self.transfer.as_str(),
        })
    }

    fn fields(&self) -> Vec<FieldSpec> {
        let mut out = vec![field::choice(
            "color_space",
            "grade.field.color_space",
            if self.rec2020 { 1 } else { 0 },
            &["srgb", "rec2020"],
            &["grade.option.srgb", "grade.option.rec2020"],
        )];
        if self.rec2020 {
            out.push(field::choice(
                "transfer",
                "grade.field.transfer",
                match self.transfer {
                    Transfer::Pq => 0,
                    Transfer::Hlg => 1,
                    Transfer::Srgb => 0,
                },
                &["pq", "hlg"],
                &["grade.option.pq", "grade.option.hlg"],
            ));
        } else {
            out.push(field::choice(
                "transfer",
                "grade.field.transfer",
                0,
                &["srgb"],
                &["grade.option.srgb"],
            ));
        }
        out
    }

    fn from_json(value: &Value) -> Result<Self, String>
    where
        Self: Sized,
    {
        // Older sidecars carried format/quality/long_edge here. They are now
        // owned by the export request, so accept and discard them while
        // preserving the new output profile fields.
        Self::parse(value)
    }

    fn set_json(&mut self, value: &Value) -> Result<(), String> {
        *self = Self::parse(value)?;
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
        _inputs: &BTreeMap<String, Arc<BufferValue>>,
        _ctx: &ExecCtx,
    ) -> Result<BTreeMap<String, BufferValue>, GradeError> {
        Ok(BTreeMap::new())
    }

    fn defaults(&self) -> Box<dyn Node> {
        Box::new(Self::new())
    }
}

#[cfg(test)]
mod tests {
    use super::Export;
    use crate::grade::nodes::NodeParam;
    use serde_json::json;

    #[test]
    fn export_keeps_only_color_space_and_transfer() {
        let node = Export::from_json(&json!({
            "format": "png",
            "quality": 12,
            "long_edge": 800,
            "color_space": "rec2020",
            "transfer": "hlg"
        }))
        .unwrap();
        assert_eq!(
            node.to_json(),
            json!({ "color_space": "rec2020", "transfer": "hlg" })
        );
    }

    #[test]
    fn srgb_transfer_is_valid_for_srgb_only() {
        let node = Export::from_json(&json!({
            "color_space": "rec2020",
            "transfer": "srgb"
        }))
        .unwrap();
        assert_eq!(
            node.to_json()["transfer"].as_str(),
            Some("pq"),
            "rec2020 cannot use the sRGB transfer"
        );
    }
}
