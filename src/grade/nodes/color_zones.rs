use serde_json::{Value, json};

use super::{Node, NodeParam, clamp_f, object};
use crate::grade::buffer::{self, InputDesc, OutputDesc};
use crate::grade::color::{linear_srgb_to_rec2020, rec2020_to_linear_srgb};
use crate::grade::exec::{Frame, map_rgb};
use crate::grade::field::{self, FieldSpec};

/// One point of a hue curve: target hue in degrees, adjustment on the y axis.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    hue: f32,
    value: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProcessMode {
    Smooth,
    Strong,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Interpolation {
    Centripetal,
    Cubic,
    Linear,
}

#[derive(Clone)]
pub struct ColorZones {
    lightness: Vec<Point>,
    chroma: Vec<Point>,
    hue_shift: Vec<Point>,
    mix: f32,
    process_mode: ProcessMode,
    interpolation: Interpolation,
}

impl ColorZones {
    pub(crate) fn new() -> Self {
        Self {
            lightness: default_curve(),
            chroma: default_curve(),
            hue_shift: default_curve(),
            mix: 100.0,
            process_mode: ProcessMode::Smooth,
            interpolation: Interpolation::Centripetal,
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        let mix = clamp_f(
            obj.get("mix")
                .and_then(Value::as_f64)
                .map(|v| v as f32)
                .unwrap_or(100.0),
            0.0,
            100.0,
        );
        Ok(Self {
            lightness: parse_curve(obj.get("lightness"), -100.0, 100.0)?,
            chroma: parse_curve(obj.get("chroma"), -100.0, 100.0)?,
            hue_shift: parse_curve(obj.get("hue"), -180.0, 180.0)?,
            mix,
            process_mode: match obj.get("process_mode").and_then(Value::as_str) {
                Some("strong") => ProcessMode::Strong,
                _ => ProcessMode::Smooth,
            },
            interpolation: match obj.get("interpolation").and_then(Value::as_str) {
                Some("linear") => Interpolation::Linear,
                Some("cubic_spline") => Interpolation::Cubic,
                _ => Interpolation::Centripetal,
            },
        })
    }

    fn curve(&self, kind: &str) -> &[Point] {
        match kind {
            "chroma" => &self.chroma,
            "hue" => &self.hue_shift,
            _ => &self.lightness,
        }
    }

    pub(super) fn render_frame(&self, input: &Frame) -> Frame {
        if self.mix <= 0.0 {
            return input.clone_frame();
        }
        let mix = self.mix / 100.0;
        let strong = self.process_mode == ProcessMode::Strong;
        let lightness = &self.lightness;
        let chroma = &self.chroma;
        let hue_shift = &self.hue_shift;
        let interpolation = self.interpolation;
        map_rgb(input, input.space, |r, g, b| {
            let [sr, sg, sb] = rec2020_to_linear_srgb(r, g, b);
            let Some((hue, sat, val)) = rgb_to_hsv(sr, sg, sb) else {
                return [r, g, b];
            };
            let dl = shape(eval(lightness, hue, interpolation), 100.0, strong);
            let dc = shape(eval(chroma, hue, interpolation), 100.0, strong);
            let dh = shape(eval(hue_shift, hue, interpolation), 180.0, strong);
            let val2 = val * (1.0 + dl / 100.0);
            let sat2 = (sat * (1.0 + dc / 100.0)).clamp(0.0, 1.0);
            let hue2 = (hue + dh).rem_euclid(360.0);
            let [sr2, sg2, sb2] = hsv_to_rgb(hue2, sat2, val2.max(0.0));
            let [r2, g2, b2] = linear_srgb_to_rec2020(sr2, sg2, sb2);
            [r + (r2 - r) * mix, g + (g2 - g) * mix, b + (b2 - b) * mix]
        })
    }
}

fn default_curve() -> Vec<Point> {
    vec![
        Point {
            hue: 0.0,
            value: 0.0,
        },
        Point {
            hue: 360.0,
            value: 0.0,
        },
    ]
}

fn parse_curve(raw: Option<&Value>, min: f32, max: f32) -> Result<Vec<Point>, String> {
    let Some(items) = raw.and_then(Value::as_array) else {
        return Ok(default_curve());
    };
    if items.len() < 2 || items.len() > 64 {
        return Err("a hue curve needs 2 to 64 points".to_string());
    }
    let mut points = Vec::with_capacity(items.len());
    for item in items {
        let pair = item
            .as_array()
            .filter(|pair| pair.len() == 2)
            .ok_or_else(|| "curve points are [hue, value]".to_string())?;
        let hue = pair[0]
            .as_f64()
            .ok_or_else(|| "curve hue must be a number".to_string())? as f32;
        let value = pair[1]
            .as_f64()
            .ok_or_else(|| "curve value must be a number".to_string())? as f32;
        if !hue.is_finite() || !value.is_finite() {
            return Err("curve values must be finite".to_string());
        }
        points.push(Point {
            hue: hue.clamp(0.0, 360.0),
            value: value.clamp(min, max),
        });
    }
    points.sort_by(|a, b| a.hue.total_cmp(&b.hue));
    // Hue 0 and 360 are one place on the wheel, so they must agree; otherwise the
    // wrap segment becomes a step.
    let first_hue = points.first().map(|point| point.hue);
    let first_value = points.first().map(|point| point.value);
    if let (Some(first_hue), Some(first_value), Some(last)) =
        (first_hue, first_value, points.last_mut())
        && first_hue <= 1e-3
        && (last.hue - 360.0).abs() <= 1e-3
    {
        last.value = first_value;
    }
    Ok(points)
}

/// HSV keeps hue on the same 0–360 wheel the UI draws (red 0, yellow 60, cyan
/// 180, blue 240), and its value axis is unbounded, so camera highlights above
/// 1 survive the round trip.
fn rgb_to_hsv(r: f32, g: f32, b: f32) -> Option<(f32, f32, f32)> {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    if max <= 1e-6 {
        return None;
    }
    let delta = max - min;
    let hue = if delta <= 1e-6 {
        0.0
    } else if max == r {
        (60.0 * ((g - b) / delta)).rem_euclid(360.0)
    } else if max == g {
        60.0 * ((b - r) / delta) + 120.0
    } else {
        60.0 * ((r - g) / delta) + 240.0
    };
    Some((hue, delta / max, max))
}

fn hsv_to_rgb(hue: f32, sat: f32, val: f32) -> [f32; 3] {
    let h = hue.rem_euclid(360.0) / 60.0;
    let chroma = val * sat;
    let x = chroma * (1.0 - ((h % 2.0) - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let m = val - chroma;
    [r + m, g + m, b + m]
}

/// `strong` keeps the sign but squares the normalised adjustment, which narrows
/// the transition between a corrected and an untouched hue band.
fn shape(value: f32, scale: f32, strong: bool) -> f32 {
    if !strong {
        return value;
    }
    let norm = value / scale;
    norm * norm.abs() * scale
}

/// Periodic evaluation over hue: the segment that wraps past 360 is a real one.
fn eval(points: &[Point], hue: f32, interpolation: Interpolation) -> f32 {
    if points.is_empty() {
        return 0.0;
    }
    if points.len() == 1 {
        return points[0].value;
    }
    let hue = hue.rem_euclid(360.0);
    let n = points.len();
    let mut index = n - 1;
    for i in 0..n {
        let start = points[i].hue;
        let end = if i + 1 < n {
            points[i + 1].hue
        } else {
            points[0].hue + 360.0
        };
        if hue >= start && hue < end {
            index = i;
            break;
        }
    }
    let p0 = points[index];
    let p1 = points[(index + 1) % n];
    let x0 = p0.hue;
    let x1 = if index + 1 < n {
        p1.hue
    } else {
        p1.hue + 360.0
    };
    let span = (x1 - x0).max(1e-3);
    let t = ((hue - x0) / span).clamp(0.0, 1.0);
    match interpolation {
        Interpolation::Linear => p0.value + (p1.value - p0.value) * t,
        Interpolation::Cubic | Interpolation::Centripetal => {
            // One slope per point, shared by both neighbouring segments: that is
            // what keeps the curve differentiable at a control point. Computing
            // tangents per segment (or from a wrapped neighbour that sits on the
            // same x) bends the curve into a corner.
            let m0 = slope(points, index, interpolation);
            let m1 = slope(points, (index + 1) % n, interpolation);
            let t2 = t * t;
            let t3 = t2 * t;
            (2.0 * t3 - 3.0 * t2 + 1.0) * p0.value
                + (t3 - 2.0 * t2 + t) * span * m0
                + (-2.0 * t3 + 3.0 * t2) * p1.value
                + (t3 - t2) * span * m1
        }
    }
}

/// Slope at `index` in value-per-degree. `Cubic` is the uniform centred
/// difference; `Centripetal` weights the two secants by their x span, which is
/// the non-uniform variant (Barry–Goldman tangents reduced to 1-D).
fn slope(points: &[Point], index: usize, interpolation: Interpolation) -> f32 {
    let n = points.len();
    let first = points[0].hue;
    let y = |i: usize| points[i % n].value;
    let x_prev = if index == 0 {
        first - 360.0
    } else {
        points[index - 1].hue
    };
    let x_next = if index + 1 < n {
        points[index + 1].hue
    } else {
        first + 360.0
    };
    let h_prev = (points[index].hue - x_prev).max(1e-3);
    let h_next = (x_next - points[index].hue).max(1e-3);
    let prev = y(index + n - 1);
    let next = y(index + 1);
    let here = points[index].value;
    if interpolation == Interpolation::Cubic {
        return (next - prev) / (h_prev + h_next);
    }
    let s_prev = (here - prev) / h_prev;
    let s_next = (next - here) / h_next;
    (s_prev * h_next + s_next * h_prev) / (h_prev + h_next)
}

impl NodeParam for ColorZones {
    fn node_type(&self) -> &'static str {
        "color-zones"
    }

    fn to_json(&self) -> Value {
        json!({
            "lightness": curve_json(&self.lightness),
            "chroma": curve_json(&self.chroma),
            "hue": curve_json(&self.hue_shift),
            "mix": self.mix,
            "process_mode": match self.process_mode {
                ProcessMode::Smooth => "smooth",
                ProcessMode::Strong => "strong",
            },
            "interpolation": match self.interpolation {
                Interpolation::Centripetal => "centripetal_spline",
                Interpolation::Cubic => "cubic_spline",
                Interpolation::Linear => "linear",
            },
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
        *self = Self::parse(value)?;
        Ok(())
    }
}

fn curve_json(points: &[Point]) -> Value {
    Value::Array(
        points
            .iter()
            .map(|p| json!([p.hue as f64, p.value as f64]))
            .collect(),
    )
}

impl Node for ColorZones {
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
        Box::new(Self::new())
    }
}

impl ColorZones {
    pub(super) fn ui_fields(&self) -> Vec<FieldSpec> {
        vec![
            field::slider(
                "mix",
                "grade.field.mix",
                self.mix,
                0.0,
                100.0,
                format!("+{:.2}%", self.mix),
            ),
            field::choice(
                "process_mode",
                "grade.field.process_mode",
                match self.process_mode {
                    ProcessMode::Smooth => 0,
                    ProcessMode::Strong => 1,
                },
                &["smooth", "strong"],
                &["grade.option.smooth", "grade.option.strong"],
            ),
            field::choice(
                "interpolation",
                "grade.field.interpolation",
                match self.interpolation {
                    Interpolation::Centripetal => 0,
                    Interpolation::Cubic => 1,
                    Interpolation::Linear => 2,
                },
                &["centripetal_spline", "cubic_spline", "linear"],
                &[
                    "grade.option.centripetal",
                    "grade.option.cubic",
                    "grade.option.linear",
                ],
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::{ColorZones, Interpolation, NodeParam, Point, eval};

    #[test]
    fn curve_is_periodic_and_hits_its_points() {
        let points = vec![
            Point {
                hue: 0.0,
                value: 0.0,
            },
            Point {
                hue: 120.0,
                value: 50.0,
            },
            Point {
                hue: 240.0,
                value: -50.0,
            },
        ];
        for point in &points {
            let got = eval(&points, point.hue, Interpolation::Linear);
            assert!((got - point.value).abs() < 1e-3, "{point:?} -> {got}");
        }
        // The wrap segment is 240..360(0): 300 lies between them.
        let wrapped = eval(&points, 300.0, Interpolation::Linear);
        assert!(wrapped < 0.0 && wrapped > -50.0, "wrapped={wrapped}");
        assert!((eval(&points, 360.0, Interpolation::Linear) - 0.0).abs() < 1e-3);
    }

    #[test]
    fn curve_has_no_corner_at_a_control_point() {
        let points = vec![
            Point {
                hue: 0.0,
                value: 0.0,
            },
            Point {
                hue: 60.0,
                value: 50.0,
            },
            Point {
                hue: 240.0,
                value: -30.0,
            },
            Point {
                hue: 360.0,
                value: 0.0,
            },
        ];
        for mode in [Interpolation::Cubic, Interpolation::Centripetal] {
            for point in &points[1..points.len() - 1] {
                let eps = 0.05;
                let left =
                    (eval(&points, point.hue, mode) - eval(&points, point.hue - eps, mode)) / eps;
                let right =
                    (eval(&points, point.hue + eps, mode) - eval(&points, point.hue, mode)) / eps;
                assert!(
                    (left - right).abs() < 0.05,
                    "{mode:?} corner at {}: left={left} right={right}",
                    point.hue
                );
            }
        }
    }

    #[test]
    fn wrap_endpoints_share_one_value() {
        let node = ColorZones::from_json(&serde_json::json!({
            "lightness": [[0.0, 20.0], [120.0, 0.0], [360.0, -40.0]],
        }))
        .unwrap();
        let json = node.to_json();
        assert_eq!(json["lightness"][2][1], 20.0);
        assert!((eval(&node.lightness, 0.0, Interpolation::Cubic) - 20.0).abs() < 1e-3);
        assert!((eval(&node.lightness, 359.9, Interpolation::Cubic) - 20.0).abs() < 1.5);
    }

    #[test]
    fn centripetal_and_cubic_differ_on_uneven_spacing() {
        let points = vec![
            Point {
                hue: 0.0,
                value: 0.0,
            },
            Point {
                hue: 20.0,
                value: 60.0,
            },
            Point {
                hue: 300.0,
                value: -20.0,
            },
        ];
        let cubic = eval(&points, 10.0, Interpolation::Cubic);
        let centripetal = eval(&points, 10.0, Interpolation::Centripetal);
        assert!(
            (cubic - centripetal).abs() > 0.5,
            "cubic={cubic} centripetal={centripetal}"
        );
        for point in &points {
            assert!(
                (eval(&points, point.hue, Interpolation::Centripetal) - point.value).abs() < 1e-3
            );
        }
    }

    #[test]
    fn params_round_trip_and_clamp() {
        let raw = serde_json::json!({
            "lightness": [[0.0, -200.0], [180.0, 200.0]],
            "chroma": [[0.0, 0.0], [360.0, 0.0]],
            "hue": [[0.0, 0.0], [200.0, 30.0]],
            "mix": 50.0,
            "process_mode": "strong",
            "interpolation": "linear",
        });
        let node = ColorZones::from_json(&raw).unwrap();
        let json = node.to_json();
        assert_eq!(json["lightness"][0][1], -100.0);
        assert_eq!(json["lightness"][1][1], 100.0);
        assert_eq!(json["mix"], 50.0);
        assert_eq!(json["process_mode"], "strong");
        assert_eq!(json["interpolation"], "linear");
    }

    #[test]
    fn lightness_curve_brightens_its_band() {
        use crate::grade::exec::{Frame, Roi, Space};
        use std::sync::Arc;
        let node = ColorZones::from_json(&serde_json::json!({
            "lightness": [[0.0, 50.0], [360.0, 50.0]],
            "mix": 100.0,
        }))
        .unwrap();
        let frame = Frame {
            width: 1,
            height: 1,
            space: Space::Linear,
            rgb: Arc::new(vec![0.6, 0.5, 0.1]),
            transform: Default::default(),
            roi: Roi::default(),
        };
        let out = node.render_frame(&frame);
        println!("in={:?} out={:?}", frame.rgb, out.rgb);
        assert!(out.rgb[0] > frame.rgb[0], "should brighten");
        assert!(out.rgb[1] > frame.rgb[1], "should brighten");
    }

    #[test]
    fn zero_mix_is_a_no_op() {
        let node = ColorZones::from_json(&serde_json::json!({
            "lightness": [[0.0, 100.0], [360.0, 100.0]],
            "mix": 0.0
        }))
        .unwrap();
        let json = node.to_json();
        assert_eq!(json["mix"], 0.0);
    }
}
