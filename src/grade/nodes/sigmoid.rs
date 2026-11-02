use serde_json::{Value, json};

use super::{Node, NodeParam, clamp_f, object, req_f};
use crate::grade::buffer::{self, InputDesc, OutputDesc};
use crate::grade::color::rec2020_to_linear_srgb;
use crate::grade::exec::{Frame, Roi, Space, ViewTransform, map_rgb};
use crate::grade::field::{self, FieldSpec};

/// Scene grey that darktable's sigmoid pins to the same display value.
/// Contrast only changes the slope there; it does not move this point.
const MIDDLE_GREY: f32 = 0.1845;

#[derive(Clone, Copy)]
struct LogLogistic {
    white: f32,
    paper_exposure: f32,
    film_fog: f32,
    film_power: f32,
    paper_power: f32,
}

#[derive(Clone)]
pub struct Sigmoid {
    /// Slope control at middle grey. Same meaning as darktable's contrast.
    contrast: f32,
    skew: f32,
    /// Display white, percent. 100 is display linear 1, the asymptote at scene infinity.
    white: f32,
    /// Display black, percent. 0.0152 is the darktable default.
    black: f32,
}

impl Sigmoid {
    pub(crate) fn new() -> Self {
        Self {
            contrast: 1.5,
            skew: 0.0,
            white: 100.0,
            black: 0.0152,
        }
    }

    fn parse(params: &Value) -> Result<Self, String> {
        let obj = object(params)?;
        Ok(Self {
            contrast: clamp_f(req_f(obj, "contrast")?, 0.1, 10.0),
            skew: clamp_f(req_f(obj, "skew")?, -1.0, 1.0),
            white: clamp_f(req_f(obj, "white")?, 20.0, 1600.0),
            black: clamp_f(req_f(obj, "black")?, 0.0, 15.0),
        })
    }
}

impl NodeParam for Sigmoid {
    fn node_type(&self) -> &'static str {
        "sigmoid"
    }

    fn to_json(&self) -> Value {
        json!({
            "contrast": self.contrast,
            "skew": self.skew,
            "white": self.white,
            "black": self.black,
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

impl Node for Sigmoid {
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
        vec![buffer::image_out(buffer::DISPLAY_RGB)]
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

impl Sigmoid {
    pub(super) fn ui_fields(&self) -> Vec<FieldSpec> {
        vec![
            field::slider(
                "contrast",
                "grade.field.contrast",
                self.contrast,
                0.1,
                10.0,
                format!("{:.2}", self.contrast),
            ),
            field::slider(
                "skew",
                "grade.field.skew",
                self.skew,
                -1.0,
                1.0,
                format!("{:+.2}", self.skew),
            ),
            field::slider(
                "white",
                "grade.field.white",
                self.white,
                20.0,
                1600.0,
                format!("{:.0}", self.white),
            ),
            field::slider(
                "black",
                "grade.field.black",
                self.black,
                0.0,
                15.0,
                format!("{:.4}", self.black),
            ),
        ]
    }

    pub(super) fn render_frame(&self, input: &Frame) -> Frame {
        let curve = log_logistic(self.contrast, self.skew, self.white, self.black);
        map_rgb(input, Space::Display, |r, g, b| {
            let [r, g, b] = rec2020_to_linear_srgb(r, g, b);
            let [r, g, b] = desaturate_negatives(r, g, b);
            let y = (0.2126 * r + 0.7152 * g + 0.0722 * b).max(0.0);
            if y <= 1e-9 {
                let v = curve.eval(0.0);
                return [v, v, v];
            }
            let scale = curve.eval(y) / y;
            [
                (r * scale).max(0.0),
                (g * scale).max(0.0),
                (b * scale).max(0.0),
            ]
        })
    }
}

/// Darktable's film-plus-paper log-logistic. `white` and `black` are percents.
fn log_logistic(contrast: f32, skew: f32, white_percent: f32, black_percent: f32) -> LogLogistic {
    let white = white_percent * 0.01;
    let black = black_percent * 0.01;
    let paper_power = 5.0f32.powf(-skew);
    let ref_exposure = MIDDLE_GREY.powf(contrast) * ((1.0 / MIDDLE_GREY) - 1.0);
    let ref_slope = slope_at_grey(1.0, ref_exposure, 0.0, contrast, 1.0);
    let white_grey = (white / MIDDLE_GREY).powf(1.0 / paper_power) - 1.0;
    let temp_exposure = MIDDLE_GREY * white_grey;
    let temp_slope = slope_at_grey(white, temp_exposure, 0.0, 1.0, paper_power);
    let film_power = if temp_slope.abs() > 1e-8 {
        ref_slope / temp_slope
    } else {
        contrast
    };
    let (film_fog, paper_exposure) = if black <= 0.0 {
        (0.0, MIDDLE_GREY.powf(film_power) * white_grey)
    } else {
        let white_black = (black / white).powf(-1.0 / paper_power) - 1.0;
        let denom = white_black.powf(1.0 / film_power) - white_grey.powf(1.0 / film_power);
        let fog = MIDDLE_GREY * white_grey.powf(1.0 / film_power) / denom;
        if fog.is_finite() && fog >= 0.0 && denom.is_finite() {
            (fog, (fog + MIDDLE_GREY).powf(film_power) * white_grey)
        } else {
            (0.0, MIDDLE_GREY.powf(film_power) * white_grey)
        }
    };
    LogLogistic {
        white,
        paper_exposure,
        film_fog,
        film_power,
        paper_power,
    }
}

fn slope_at_grey(magnitude: f32, paper: f32, fog: f32, film: f32, paper_power: f32) -> f32 {
    let delta = 1e-6;
    let curve = LogLogistic {
        white: magnitude,
        paper_exposure: paper,
        film_fog: fog,
        film_power: film,
        paper_power,
    };
    (curve.eval(MIDDLE_GREY + delta) - curve.eval(MIDDLE_GREY - delta)) / (2.0 * delta)
}

impl LogLogistic {
    fn eval(self, value: f32) -> f32 {
        let film = (self.film_fog + value.max(0.0)).powf(self.film_power);
        let denom = self.paper_exposure + film;
        if denom <= 0.0 {
            return 0.0;
        }
        let paper = self.white * (film / denom).powf(self.paper_power);
        if paper.is_finite() { paper } else { self.white }
    }
}

fn desaturate_negatives(r: f32, g: f32, b: f32) -> [f32; 3] {
    let average = ((r + g + b) / 3.0).max(0.0);
    let min_value = r.min(g).min(b);
    if min_value >= 0.0 {
        return [r, g, b];
    }
    let gap = min_value - average;
    if gap.abs() < 1e-12 {
        return [average, average, average];
    }
    let saturation = -average / gap;
    [
        average + saturation * (r - average),
        average + saturation * (g - average),
        average + saturation * (b - average),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_grey_stays_on_middle_grey() {
        let curve = log_logistic(1.5, 0.0, 100.0, 0.0152);
        let y = curve.eval(MIDDLE_GREY);
        assert!((y - MIDDLE_GREY).abs() < 1e-3, "{y}");
    }

    #[test]
    fn diffuse_white_stays_on_the_shoulder() {
        let curve = log_logistic(1.5, 0.0, 100.0, 0.0152);
        let scene_white = curve.eval(1.0);
        let hot = curve.eval(4.0);
        assert!((scene_white - 0.739).abs() < 0.02, "{scene_white}");
        assert!((hot - 0.958).abs() < 0.02, "{hot}");
        assert!(hot > scene_white);
    }

    #[test]
    fn lower_contrast_keeps_grey_and_opens_the_shoulder() {
        let soft = log_logistic(1.0, 0.0, 100.0, 0.0152);
        let hard = log_logistic(1.5, 0.0, 100.0, 0.0152);
        assert!((soft.eval(MIDDLE_GREY) - MIDDLE_GREY).abs() < 1e-3);
        assert!(soft.eval(1.0) < hard.eval(1.0));
    }

    #[test]
    fn neutral_pixel_keeps_middle_grey() {
        let out = Sigmoid::new().render_frame(&Frame {
            width: 1,
            height: 1,
            space: Space::Linear,
            rgb: std::sync::Arc::new(vec![MIDDLE_GREY, MIDDLE_GREY, MIDDLE_GREY]),
            transform: ViewTransform::default(),
            roi: Roi::default(),
        });
        assert!((out.rgb[0] - MIDDLE_GREY).abs() < 1e-3, "{}", out.rgb[0]);
        assert_eq!(out.space, Space::Display);
    }
}
