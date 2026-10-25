use serde_json::{Value, json};

use super::{Node, NodeParam, object, req_bool, req_f, text, wrap_degrees};
use crate::grade::buffer::{InputDesc, OutputDesc};
use crate::grade::exec::{Frame, Roi};
use crate::grade::field::{self, FieldSpec};

#[derive(Clone)]
pub struct Rotate {
    degrees: f32,
    expand: bool,
    /// Crop to the largest axis-aligned rectangle that keeps the source aspect and has no empty corners.
    inscribed: bool,
}

impl Rotate {
    pub(crate) fn new() -> Self {
        Self {
            degrees: 0.0,
            expand: true,
            inscribed: true,
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        Ok(Self {
            degrees: wrap_degrees(req_f(obj, "degrees")?),
            expand: req_bool(obj, "expand")?,
            inscribed: obj
                .get("inscribed")
                .and_then(Value::as_bool)
                .unwrap_or(true),
        })
    }
}

impl NodeParam for Rotate {
    fn node_type(&self) -> &'static str {
        "rotate"
    }

    fn to_json(&self) -> Value {
        json!({
            "degrees": self.degrees,
            "expand": self.expand,
            "inscribed": self.inscribed,
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

impl Node for Rotate {
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

impl Rotate {
    pub(super) fn ui_fields(&self, locale: &str) -> Vec<FieldSpec> {
        let zh = locale.starts_with("zh");
        vec![
            field::slider(
                "degrees",
                text(zh, "Angle", "角度"),
                self.degrees,
                -180.0,
                180.0,
                format!("{:.0}°", self.degrees),
            ),
            field::toggle(
                "expand",
                text(zh, "Expand canvas", "扩展画布"),
                self.expand,
                text(zh, "On", "开"),
                text(zh, "Off", "关"),
            ),
            field::toggle(
                "inscribed",
                text(zh, "Largest fit", "最大内接矩形"),
                self.inscribed,
                text(zh, "On", "开"),
                text(zh, "Off", "关"),
            ),
        ]
    }

    pub(super) fn render_frame(&self, input: &Frame) -> Frame {
        let mut frame = input.clone_frame();
        frame.transform.degrees += self.degrees;
        let degrees = frame.transform.degrees;
        let [x, y, w, h] = if self.inscribed {
            inscribed_norm(input.width as f32, input.height as f32, degrees)
        } else if self.expand {
            [0.0, 0.0, 1.0, 1.0]
        } else {
            source_window_norm(input.width as f32, input.height as f32, degrees)
        };
        frame.roi = Roi { x, y, w, h };
        frame
    }
}

fn source_window_norm(source_w: f32, source_h: f32, degrees: f32) -> [f32; 4] {
    let rad = degrees.to_radians();
    let (sin_a, cos_a) = (rad.sin().abs(), rad.cos().abs());
    if sin_a < 1e-4 || cos_a < 1e-4 {
        return [0.0, 0.0, 1.0, 1.0];
    }
    let canvas_w = (source_w * cos_a + source_h * sin_a).round().max(1.0);
    let canvas_h = (source_w * sin_a + source_h * cos_a).round().max(1.0);
    let ow = source_w.round().clamp(1.0, canvas_w);
    let oh = source_h.round().clamp(1.0, canvas_h);
    [
        (canvas_w - ow) * 0.5 / canvas_w,
        (canvas_h - oh) * 0.5 / canvas_h,
        ow / canvas_w,
        oh / canvas_h,
    ]
}

pub(crate) fn inscribed_norm(source_w: f32, source_h: f32, degrees: f32) -> [f32; 4] {
    let turns = (degrees / 90.0).round();
    if (degrees - turns * 90.0).abs() < 1e-2 || source_w <= 0.0 || source_h <= 0.0 {
        return [0.0, 0.0, 1.0, 1.0];
    }
    let rad = degrees.to_radians();
    let (sin_a, cos_a) = (rad.sin().abs(), rad.cos().abs());
    let canvas_w = (source_w * cos_a + source_h * sin_a).round().max(1.0);
    let canvas_h = (source_w * sin_a + source_h * cos_a).round().max(1.0);
    let (wr, hr) = largest_axis_aligned(source_w, source_h, degrees);
    let ow = wr.round().clamp(1.0, canvas_w);
    let oh = hr.round().clamp(1.0, canvas_h);
    [
        (canvas_w - ow) * 0.5 / canvas_w,
        (canvas_h - oh) * 0.5 / canvas_h,
        ow / canvas_w,
        oh / canvas_h,
    ]
}

/// Largest axis-aligned rectangle inside the rotated source that keeps `source_w / source_h`.
fn largest_axis_aligned(source_w: f32, source_h: f32, angle_deg: f32) -> (f32, f32) {
    if source_w <= 0.0 || source_h <= 0.0 {
        return (source_w.max(1.0), source_h.max(1.0));
    }
    let rad = angle_deg.to_radians();
    let (sin_a, cos_a) = (rad.sin().abs(), rad.cos().abs());
    if sin_a < 1e-4 {
        return (source_w, source_h);
    }
    if cos_a < 1e-4 {
        return (source_h, source_w);
    }
    // A centered rectangle of size k*(w, h) stays inside the rotated source when
    // its corners satisfy both edge half-planes.
    let k = (source_w / (source_w * cos_a + source_h * sin_a))
        .min(source_h / (source_w * sin_a + source_h * cos_a));
    (source_w * k, source_h * k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grade::exec::{Roi, Space, ViewTransform};

    fn solid(w: u32, h: u32) -> Frame {
        Frame {
            width: w,
            height: h,
            space: Space::Linear,
            rgb: std::sync::Arc::new(vec![1.0; (w * h * 3) as usize]),
            transform: ViewTransform::default(),
            roi: Roi::default(),
        }
    }

    fn node(degrees: f32, inscribed: bool) -> Rotate {
        Rotate {
            degrees,
            expand: true,
            inscribed,
        }
    }

    #[test]
    fn rotate_keeps_pixels_and_records_the_turn() {
        let input = solid(40, 30);
        let out = node(20.0, true).render_frame(&input);
        assert_eq!(out.rgb, input.rgb);
        assert!((out.transform.degrees - 20.0).abs() < 1e-4);
        assert!((out.roi.w / out.roi.h - 40.0 / 30.0).abs() < 0.05 || out.roi.w < 1.0);
        let (wr, hr) = largest_axis_aligned(40.0, 30.0, 20.0);
        assert!((wr / hr - 40.0 / 30.0).abs() < 1e-4);
    }

    #[test]
    fn zero_angle_keeps_the_source() {
        let (wr, hr) = largest_axis_aligned(400.0, 300.0, 0.0);
        assert!((wr - 400.0).abs() < 1e-3);
        assert!((hr - 300.0).abs() < 1e-3);
    }

    #[test]
    fn inscribed_box_is_centered_on_the_expanded_canvas() {
        let [x, y, w, h] = inscribed_norm(100.0, 100.0, 45.0);
        assert!((w - 0.5).abs() < 0.02, "{w}");
        assert!((h - 0.5).abs() < 0.02, "{h}");
        assert!((x - (1.0 - w) / 2.0).abs() < 1e-3);
        assert!((y - (1.0 - h) / 2.0).abs() < 1e-3);
        assert_eq!(inscribed_norm(400.0, 300.0, 0.0), [0.0, 0.0, 1.0, 1.0]);
    }
}
