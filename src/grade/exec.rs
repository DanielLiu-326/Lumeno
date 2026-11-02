//! Preview executor. A node edit keeps the frame that enters the edited node.
//! The next edit of that same node reruns from there. Only that one frame is kept.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};

use image::{DynamicImage, ImageBuffer, Rgb};
use rayon::prelude::*;

use rayon::ThreadPoolBuilder;

use super::buffer::{self, DisplayBuffer, ImageSize, LinearBuffer, WorkingSpace};
use super::color::{self, encode_u8, luma, rec2020_to_linear_srgb, srgb_to_linear};
use super::document::{self, Document};
use super::error::GradeError;
use super::pipeline::Pipeline;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Space {
    /// Linear Rec.2020 working space.
    Linear,
    /// Linear camera RGB. Input Color Profile turns this into `Linear`.
    Camera,
    Display,
}

/// Rotation stacked by geometry nodes. Degrees are counterclockwise in the view, same as the rotate node.
#[derive(Clone, Copy, Debug)]
pub struct ViewTransform {
    pub degrees: f32,
}

impl Default for ViewTransform {
    fn default() -> Self {
        Self { degrees: 0.0 }
    }
}

/// Axis-aligned region of the transformed canvas, normalized to that canvas.
#[derive(Clone, Copy, Debug)]
pub struct Roi {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Default for Roi {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: 1.0,
            h: 1.0,
        }
    }
}

pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub(crate) space: Space,
    /// Shared pixel buffer. Geometry nodes clone the `Arc` and only change the view.
    pub(crate) rgb: Arc<Vec<f32>>,
    pub(crate) transform: ViewTransform,
    pub(crate) roi: Roi,
}

pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub struct CancelFlag {
    flag: AtomicBool,
}

impl CancelFlag {
    pub fn new() -> Self {
        Self {
            flag: AtomicBool::new(false),
        }
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
}

pub enum ExecTarget {
    Preview { max_edge: u32 },
    Export,
}

pub struct ExecCtx<'a> {
    pub target: ExecTarget,
    pub cancel: &'a CancelFlag,
}

pub struct Render {
    pub preview: Rgba,
    pub degrees: f32,
    pub roi: [f32; 4],
    pub elapsed_ms: u128,
    pub reran: Vec<String>,
}

pub fn frame_from_image(image: &DynamicImage, max_edge: u32) -> Frame {
    let mut image = image.to_rgb8();
    let (w, h) = image.dimensions();
    let long = w.max(h);
    if max_edge > 0 && long > max_edge {
        let scale = max_edge as f32 / long as f32;
        let nw = ((w as f32) * scale).round().max(1.0) as u32;
        let nh = ((h as f32) * scale).round().max(1.0) as u32;
        image = image::imageops::resize(&image, nw, nh, image::imageops::FilterType::Triangle);
    }
    let (width, height) = image.dimensions();
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for pixel in image.pixels() {
        let r = srgb_to_linear(pixel[0] as f32 / 255.0);
        let g = srgb_to_linear(pixel[1] as f32 / 255.0);
        let b = srgb_to_linear(pixel[2] as f32 / 255.0);
        let [r, g, b] = color::linear_srgb_to_rec2020(r, g, b);
        rgb.extend_from_slice(&[r, g, b]);
    }
    Frame {
        width,
        height,
        space: Space::Linear,
        rgb: Arc::new(rgb),
        transform: ViewTransform::default(),
        roi: Roi::default(),
    }
}

/// `samples` is packed RGB, linear, full range `0..=65535`.
fn frame_from_linear_u16(
    width: u32,
    height: u32,
    samples: Vec<u16>,
    max_edge: u32,
    space: Space,
) -> Frame {
    let Some(mut image) = ImageBuffer::<Rgb<u16>, Vec<u16>>::from_raw(width, height, samples)
    else {
        return Frame {
            width: 0,
            height: 0,
            space,
            rgb: Arc::new(Vec::new()),
            transform: ViewTransform::default(),
            roi: Roi::default(),
        };
    };
    let long = width.max(height);
    if max_edge > 0 && long > max_edge {
        let scale = max_edge as f32 / long as f32;
        let nw = ((width as f32) * scale).round().max(1.0) as u32;
        let nh = ((height as f32) * scale).round().max(1.0) as u32;
        image = image::imageops::resize(&image, nw, nh, image::imageops::FilterType::Triangle);
    }
    let (width, height) = image.dimensions();
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for pixel in image.pixels() {
        rgb.extend_from_slice(&[
            pixel[0] as f32 / 65535.0,
            pixel[1] as f32 / 65535.0,
            pixel[2] as f32 / 65535.0,
        ]);
    }
    Frame {
        width,
        height,
        space,
        rgb: Arc::new(rgb),
        transform: ViewTransform::default(),
        roi: Roi::default(),
    }
}

/// `samples` is packed RGB, already linear Rec.2020, full range `0..=65535`.
pub fn frame_from_linear_rec2020(
    width: u32,
    height: u32,
    samples: Vec<u16>,
    max_edge: u32,
) -> Frame {
    frame_from_linear_u16(width, height, samples, max_edge, Space::Linear)
}

/// `samples` is packed linear camera RGB. The profile node converts it.
pub fn frame_from_camera_rgb(width: u32, height: u32, samples: Vec<u16>, max_edge: u32) -> Frame {
    frame_from_linear_u16(width, height, samples, max_edge, Space::Camera)
}

pub fn render(
    doc: &Document,
    source: &Frame,
    cache: &mut HashMap<String, Arc<Frame>>,
    dirty: &HashSet<String>,
) -> Render {
    render_cancellable(doc, source, cache, dirty, &CancelFlag::new())
}

/// Same as [`render`], but `cancel` is shared with the host. The host sets it
/// when a newer request on this open should stop the current one. The flag is
/// read between nodes.
pub fn render_cancellable(
    doc: &Document,
    source: &Frame,
    cache: &mut HashMap<String, Arc<Frame>>,
    dirty: &HashSet<String>,
    cancel: &CancelFlag,
) -> Render {
    render_pipeline(doc, source, cache, dirty, cancel).0
}

fn render_pipeline(
    doc: &Document,
    source: &Frame,
    cache: &mut HashMap<String, Arc<Frame>>,
    dirty: &HashSet<String>,
    cancel: &CancelFlag,
) -> (Render, Frame) {
    let started = std::time::Instant::now();
    let ctx = ExecCtx {
        target: ExecTarget::Preview { max_edge: 1440 },
        cancel,
    };
    let mut reran = Vec::new();
    let order = Pipeline::compile(doc).order().to_vec();
    // Opening a photo dirties every node, including input, so nothing is kept.
    // An edit dirties one node and its downstream. Keep the frame that enters
    // that node, so the next drag does not rerun its upstream.
    let checkpoint = dirty.iter().find(|id| match document::upstream(doc, id) {
        Some(prev) => !dirty.contains(prev),
        None => false,
    });
    let resume_at = order
        .iter()
        .rposition(|id| !dirty.contains(id) && cache.contains_key(id));
    let resume = resume_at.map(|index| Arc::clone(&cache[&order[index]]));
    cache.clear();
    let mut current = if let Some(frame) = resume.clone() {
        frame
    } else {
        Arc::new(source.clone_linear())
    };
    let start = resume_at.map(|index| index + 1).unwrap_or(0);
    for id in order.iter().skip(start) {
        if ctx.cancel.is_cancelled() {
            break;
        }
        if checkpoint.map(String::as_str) == Some(id.as_str()) {
            if let Some(prev) = document::upstream(doc, id) {
                cache.insert(prev.to_string(), Arc::clone(&current));
            }
        }
        if id != "input" {
            let Some(node) = doc.nodes.iter().find(|node| node.id == *id) else {
                continue;
            };
            if node.enabled || node.always_on() {
                reran.push(id.clone());
                let carried = Arc::clone(&current);
                let mut named = BTreeMap::new();
                named.insert(
                    "image".to_string(),
                    Arc::new(value_from_frame(current.clone_frame())),
                );
                let mut scratch = node.clone();
                match scratch.run(&named, &ctx) {
                    Err(GradeError::Cancelled) => break,
                    other => {
                        current = other
                            .ok()
                            .as_ref()
                            .and_then(|outputs| outputs.get("image"))
                            .and_then(frame_from_value)
                            .map(Arc::new)
                            .unwrap_or(carried);
                    }
                }
            }
        }
    }
    if cache.is_empty() {
        if let Some(index) = resume_at {
            if let Some(frame) = resume {
                cache.insert(order[index].clone(), frame);
            }
        }
    }
    let frame = current.clone_frame();
    let preview = if cancel.is_cancelled() {
        Rgba {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        }
    } else {
        present(&current)
    };
    (
        Render {
            preview,
            degrees: current.transform.degrees,
            roi: [current.roi.x, current.roi.y, current.roi.w, current.roi.h],
            elapsed_ms: started.elapsed().as_millis(),
            reran,
        },
        frame,
    )
}

pub fn original_preview(source: &Frame) -> Rgba {
    present(source)
}

fn present(frame: &Frame) -> Rgba {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let mut pixels = vec![0u8; w * h * 4];
    if w == 0 || h == 0 {
        return Rgba {
            width: frame.width,
            height: frame.height,
            pixels,
        };
    }
    let encode_row = |y: usize, dst: &mut [u8]| {
        let src = &frame.rgb[y * w * 3..(y + 1) * w * 3];
        for (px, out) in src.chunks_exact(3).zip(dst.chunks_exact_mut(4)) {
            let [r, g, b] = if frame.space == Space::Display || frame.space == Space::Camera {
                [px[0], px[1], px[2]]
            } else {
                let [r, g, b] = rec2020_to_linear_srgb(px[0], px[1], px[2]);
                [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)]
            };
            out[0] = encode_u8(r);
            out[1] = encode_u8(g);
            out[2] = encode_u8(b);
            out[3] = 255;
        }
    };
    for_rows_u8(h, w * 4, &mut pixels, encode_row);
    Rgba {
        width: frame.width,
        height: frame.height,
        pixels,
    }
}

/// Bake the view transform (stacked rotation + normalized crop) into a display-referred image.
/// This is the pixel-space equivalent of how the host preview applies `degrees` and `roi` with
/// CSS: the source is centered on a canvas large enough for the rotated bounds, rotated clockwise
/// by `degrees` (screen convention, positive is clockwise), then cropped to `roi`. A full roi with
/// no rotation returns the image unchanged.
pub fn bake_view(rgba: &Rgba, degrees: f32, roi: [f32; 4]) -> Rgba {
    let [rx, ry, rw, rh] = roi;
    let no_geometry = degrees.abs() < 1e-3
        && rx <= 1e-4
        && ry <= 1e-4
        && (rw - 1.0).abs() < 1e-3
        && (rh - 1.0).abs() < 1e-3;
    if no_geometry {
        return Rgba {
            width: rgba.width,
            height: rgba.height,
            pixels: rgba.pixels.clone(),
        };
    }
    let (w, h) = (rgba.width, rgba.height);
    if w == 0 || h == 0 {
        return Rgba {
            width: 0,
            height: 0,
            pixels: Vec::new(),
        };
    }
    let wf = w as f32;
    let hf = h as f32;
    let rad = degrees.to_radians();
    let (sin_a, cos_a) = (rad.sin(), rad.cos());
    let canvas_w = (wf * cos_a.abs() + hf * sin_a.abs()).round().max(1.0);
    let canvas_h = (wf * sin_a.abs() + hf * cos_a.abs()).round().max(1.0);
    let rw = rw.clamp(1e-3, 1.0);
    let rh = rh.clamp(1e-3, 1.0);
    let out_w = (rw * canvas_w).round().max(1.0) as u32;
    let out_h = (rh * canvas_h).round().max(1.0) as u32;
    let src = &rgba.pixels;
    let src_w = w as usize;
    let src_h = h as usize;
    let mut out = vec![0u8; (out_w as usize) * (out_h as usize) * 4];
    let row_len = (out_w as usize) * 4;
    for_rows_u8(out_h as usize, row_len, &mut out, |oy, row| {
        let cy = ry * canvas_h + (oy as f32 + 0.5) * rh * canvas_h / out_h as f32;
        let dy = cy - canvas_h * 0.5;
        for ox in 0..out_w as usize {
            let cx = rx * canvas_w + (ox as f32 + 0.5) * rw * canvas_w / out_w as f32;
            let dx = cx - canvas_w * 0.5;
            // Inverse of the forward rotation: image-local coords relative to the image center.
            let lx = dx * cos_a + dy * sin_a;
            let ly = -dx * sin_a + dy * cos_a;
            let px = bilinear_rgba(
                src,
                src_w,
                src_h,
                lx + (wf - 1.0) * 0.5,
                ly + (hf - 1.0) * 0.5,
            );
            let dst = ox * 4;
            row[dst..dst + 4].copy_from_slice(&px);
        }
    });
    Rgba {
        width: out_w,
        height: out_h,
        pixels: out,
    }
}

fn bilinear_rgba(src: &[u8], w: usize, h: usize, sx: f32, sy: f32) -> [u8; 4] {
    // Small slack absorbs float noise so an exactly-on-edge sample (e.g. cos(90°))
    // is not rejected as outside the image.
    const EPS: f32 = 1e-3;
    if sx < -EPS || sy < -EPS || sx >= w as f32 + EPS || sy >= h as f32 + EPS {
        return [0, 0, 0, 0];
    }
    let x = sx.clamp(0.0, w as f32 - 1.0);
    let y = sy.clamp(0.0, h as f32 - 1.0);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let i00 = (y0 * w + x0) * 4;
    let i01 = (y0 * w + x1) * 4;
    let i10 = (y1 * w + x0) * 4;
    let i11 = (y1 * w + x1) * 4;
    let mut out = [0u8; 4];
    for c in 0..4 {
        let v00 = src[i00 + c] as f32;
        let v01 = src[i01 + c] as f32;
        let v10 = src[i10 + c] as f32;
        let v11 = src[i11 + c] as f32;
        let top = v00 + (v01 - v00) * fx;
        let bottom = v10 + (v11 - v10) * fx;
        out[c] = (top + (bottom - top) * fy).round().clamp(0.0, 255.0) as u8;
    }
    out
}

pub(crate) fn value_from_frame(frame: Frame) -> buffer::Value {
    let size = ImageSize {
        width: frame.width,
        height: frame.height,
    };
    let degrees = frame.transform.degrees;
    let roi = [frame.roi.x, frame.roi.y, frame.roi.w, frame.roi.h];
    if frame.space == Space::Display {
        buffer::Value::Display(DisplayBuffer {
            size,
            rgb: frame.rgb,
            degrees,
            roi,
        })
    } else {
        buffer::Value::Linear(LinearBuffer {
            size,
            space: if frame.space == Space::Camera {
                WorkingSpace::Camera
            } else {
                WorkingSpace::LinearRec2020
            },
            rgb: frame.rgb,
            degrees,
            roi,
        })
    }
}

pub(crate) fn frame_from_value(value: &buffer::Value) -> Option<Frame> {
    match value {
        buffer::Value::Linear(buf) => Some(Frame {
            width: buf.size.width,
            height: buf.size.height,
            space: match buf.space {
                WorkingSpace::Camera => Space::Camera,
                WorkingSpace::LinearRec2020 => Space::Linear,
            },
            rgb: buf.rgb.clone(),
            transform: ViewTransform {
                degrees: buf.degrees,
            },
            roi: Roi {
                x: buf.roi[0],
                y: buf.roi[1],
                w: buf.roi[2],
                h: buf.roi[3],
            },
        }),
        buffer::Value::Display(buf) => Some(Frame {
            width: buf.size.width,
            height: buf.size.height,
            space: Space::Display,
            rgb: buf.rgb.clone(),
            transform: ViewTransform {
                degrees: buf.degrees,
            },
            roi: Roi {
                x: buf.roi[0],
                y: buf.roi[1],
                w: buf.roi[2],
                h: buf.roi[3],
            },
        }),
        _ => None,
    }
}

impl Frame {
    pub fn duplicate(&self) -> Frame {
        self.clone_frame()
    }

    pub(crate) fn clone_frame(&self) -> Frame {
        Frame {
            width: self.width,
            height: self.height,
            space: self.space,
            rgb: self.rgb.clone(),
            transform: self.transform,
            roi: self.roi,
        }
    }

    pub(crate) fn clone_linear(&self) -> Frame {
        let mut frame = self.clone_frame();
        if frame.space == Space::Display {
            frame.space = Space::Linear;
        }
        frame
    }
}

pub(crate) fn map_rgb(
    input: &Frame,
    space: Space,
    f: impl Fn(f32, f32, f32) -> [f32; 3] + Sync,
) -> Frame {
    let w = input.width as usize;
    let h = input.height as usize;
    let mut rgb = vec![0f32; w * h * 3];
    if w == 0 || h == 0 {
        return Frame {
            width: input.width,
            height: input.height,
            space,
            rgb: Arc::new(rgb),
            transform: input.transform,
            roi: input.roi,
        };
    }
    for_rows(h, w * 3, &mut rgb, |y, dst| {
        let src = &input.rgb[y * w * 3..(y + 1) * w * 3];
        for (px, out) in src.chunks_exact(3).zip(dst.chunks_exact_mut(3)) {
            let [r, g, b] = f(px[0], px[1], px[2]);
            out[0] = r;
            out[1] = g;
            out[2] = b;
        }
    });
    Frame {
        width: input.width,
        height: input.height,
        space,
        rgb: Arc::new(rgb),
        transform: input.transform,
        roi: input.roi,
    }
}

struct GradePool {
    width: usize,
    pool: Option<Arc<rayon::ThreadPool>>,
}

fn logical_cpus() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .max(1)
}

fn grade_pool() -> &'static RwLock<GradePool> {
    static POOL: OnceLock<RwLock<GradePool>> = OnceLock::new();
    POOL.get_or_init(|| {
        let width = logical_cpus();
        let pool = if width <= 1 {
            None
        } else {
            ThreadPoolBuilder::new()
                .num_threads(width)
                .thread_name(|index| format!("grade-{index}"))
                .build()
                .ok()
                .map(Arc::new)
        };
        RwLock::new(GradePool { width, pool })
    })
}

pub(crate) fn uses_pool() -> bool {
    grade_pool()
        .read()
        .unwrap_or_else(|err| err.into_inner())
        .pool
        .is_some()
}

pub(crate) fn in_parallel<R: Send>(work: impl FnOnce() -> R + Send) -> R {
    let pool = grade_pool()
        .read()
        .unwrap_or_else(|err| err.into_inner())
        .pool
        .clone();
    match pool {
        Some(pool) => pool.install(work),
        None => work(),
    }
}

pub(crate) fn log_luma(input: &Frame) -> Vec<f32> {
    let n = input.rgb.len() / 3;
    let mut out = vec![0f32; n];
    if n < 48 * 48 {
        for (i, px) in input.rgb.chunks_exact(3).enumerate() {
            out[i] = luma(px[0], px[1], px[2]).max(1e-6).log2();
        }
        return out;
    }
    if !uses_pool() {
        for (i, px) in input.rgb.chunks_exact(3).enumerate() {
            out[i] = luma(px[0], px[1], px[2]).max(1e-6).log2();
        }
        return out;
    }
    in_parallel(|| {
        out.par_iter_mut().enumerate().for_each(|(i, slot)| {
            let px = &input.rgb[i * 3..i * 3 + 3];
            *slot = luma(px[0], px[1], px[2]).max(1e-6).log2();
        });
    });
    out
}

/// Run `f` once per row. Tall images split rows across the shared grade pool.
pub(crate) fn for_rows(
    height: usize,
    row_len: usize,
    dst: &mut [f32],
    f: impl Fn(usize, &mut [f32]) + Sync,
) {
    if height == 0 || row_len == 0 {
        return;
    }
    if height < 48 || !uses_pool() {
        for (y, row) in dst.chunks_mut(row_len).take(height).enumerate() {
            f(y, row);
        }
        return;
    }
    in_parallel(|| {
        dst.par_chunks_mut(row_len)
            .take(height)
            .enumerate()
            .for_each(|(y, row)| f(y, row));
    });
}

fn for_rows_u8(height: usize, row_len: usize, dst: &mut [u8], f: impl Fn(usize, &mut [u8]) + Sync) {
    if height == 0 || row_len == 0 {
        return;
    }
    if height < 48 || !uses_pool() {
        for (y, row) in dst.chunks_mut(row_len).take(height).enumerate() {
            f(y, row);
        }
        return;
    }
    in_parallel(|| {
        dst.par_chunks_mut(row_len)
            .take(height)
            .enumerate()
            .for_each(|(y, row)| f(y, row));
    });
}

pub(crate) fn radius_px(radius: f32, w: usize, h: usize) -> usize {
    (radius * w.min(h) as f32).round().max(1.0) as usize
}

pub(crate) fn guided(src: &[f32], w: usize, h: usize, radius: usize, eps: f32) -> Vec<f32> {
    guided_by(src, src, w, h, radius, radius, eps)
}

/// Guided filter of `src`. `radius` estimates the local fit. `coeff_radius`
/// averages that fit. Dehaze uses the same radius for both: the dark-channel
/// tiles are the size of the patch, and only averaging the fit removes them.
/// A zero `coeff_radius` leaves those tiles, which alias into moiré.
pub(crate) fn guided_by(
    guide: &[f32],
    src: &[f32],
    w: usize,
    h: usize,
    radius: usize,
    coeff_radius: usize,
    eps: f32,
) -> Vec<f32> {
    let mean_g = box_blur(guide, w, h, radius);
    let mean_s = box_blur(src, w, h, radius);
    let mut gg = vec![0f32; src.len()];
    let mut gs = vec![0f32; src.len()];
    for_rows(h, w, &mut gg, |y, row| {
        let g = &guide[y * w..(y + 1) * w];
        for x in 0..w {
            row[x] = g[x] * g[x];
        }
    });
    for_rows(h, w, &mut gs, |y, row| {
        let g = &guide[y * w..(y + 1) * w];
        let s = &src[y * w..(y + 1) * w];
        for x in 0..w {
            row[x] = g[x] * s[x];
        }
    });
    let mean_gg = box_blur(&gg, w, h, radius);
    let mean_gs = box_blur(&gs, w, h, radius);
    let mut a = vec![0f32; src.len()];
    let mut b = vec![0f32; src.len()];
    for_rows(h, w, &mut a, |y, row| {
        for x in 0..w {
            let i = y * w + x;
            let var = (mean_gg[i] - mean_g[i] * mean_g[i]).max(0.0);
            let cov = mean_gs[i] - mean_g[i] * mean_s[i];
            row[x] = cov / (var + eps);
        }
    });
    for_rows(h, w, &mut b, |y, row| {
        for x in 0..w {
            let i = y * w + x;
            row[x] = mean_s[i] - a[i] * mean_g[i];
        }
    });
    let mean_a = box_blur(&a, w, h, coeff_radius);
    let mean_b = box_blur(&b, w, h, coeff_radius);
    let mut out = vec![0f32; src.len()];
    for_rows(h, w, &mut out, |y, row| {
        for x in 0..w {
            let i = y * w + x;
            row[x] = mean_a[i] * guide[i] + mean_b[i];
        }
    });
    out
}

pub(crate) fn box_blur(src: &[f32], w: usize, h: usize, radius: usize) -> Vec<f32> {
    if radius == 0 || w == 0 || h == 0 {
        return src.to_vec();
    }
    let stride = w + 1;
    let mut integral = vec![0f32; stride * (h + 1)];
    for y in 0..h {
        let mut row = 0.0;
        for x in 0..w {
            row += src[y * w + x];
            integral[(y + 1) * stride + (x + 1)] = integral[y * stride + (x + 1)] + row;
        }
    }
    let mut out = vec![0f32; src.len()];
    for_rows(h, w, &mut out, |y, row| {
        let y0 = y.saturating_sub(radius);
        let y1 = (y + radius + 1).min(h);
        for x in 0..w {
            let x0 = x.saturating_sub(radius);
            let x1 = (x + radius + 1).min(w);
            let sum = integral[y1 * stride + x1]
                - integral[y0 * stride + x1]
                - integral[y1 * stride + x0]
                + integral[y0 * stride + x0];
            row[x] = sum / ((x1 - x0) * (y1 - y0)) as f32;
        }
    });
    out
}

pub(crate) fn min_filter(src: &[f32], w: usize, h: usize, radius: usize) -> Vec<f32> {
    if w == 0 || h == 0 {
        return Vec::new();
    }
    if radius == 0 {
        return src.to_vec();
    }
    let mut tmp = vec![0f32; src.len()];
    for_rows(h, w, &mut tmp, |y, row| {
        let src_row = &src[y * w..(y + 1) * w];
        for x in 0..w {
            let x0 = x.saturating_sub(radius);
            let x1 = (x + radius).min(w - 1);
            let mut m = f32::MAX;
            for k in x0..=x1 {
                m = m.min(src_row[k]);
            }
            row[x] = m;
        }
    });
    let mut out = vec![0f32; src.len()];
    for_rows(h, w, &mut out, |y, row| {
        let y0 = y.saturating_sub(radius);
        let y1 = (y + radius).min(h - 1);
        for x in 0..w {
            let mut m = f32::MAX;
            for k in y0..=y1 {
                m = m.min(tmp[k * w + x]);
            }
            row[x] = m;
        }
    });
    out
}

pub(crate) fn gaussian(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let radius = (sigma * 1.5).round().max(1.0) as usize;
    let mut once = box_blur(src, w, h, radius);
    once = box_blur(&once, w, h, radius);
    box_blur(&once, w, h, radius)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grade::document::{NodePatch, apply_node_params, default_document};
    use serde_json::json;
    use std::collections::HashSet;

    #[test]
    fn exposure_brightens_without_rerunning_input() {
        let mut doc = default_document(false);
        apply_node_params(
            &mut doc,
            &NodePatch {
                id: "exposure".into(),
                enabled: true,
                params: json!({ "ev": 1.0, "black": 0.0 }),
            },
        )
        .unwrap();
        let source = Frame {
            width: 1,
            height: 1,
            space: Space::Linear,
            rgb: Arc::new(vec![0.2, 0.2, 0.2]),
            transform: ViewTransform::default(),
            roi: Roi::default(),
        };
        let mut cache = HashMap::new();
        let dirty: HashSet<String> = doc.nodes.iter().map(|node| node.id.clone()).collect();
        let first = render(&doc, &source, &mut cache, &dirty);
        assert!(first.preview.pixels[0] > 0);
        let linear = frame_from_linear_rec2020(1, 1, vec![32768, 32768, 32768], 0);
        let mid = linear.rgb[0];
        assert!((mid - 32768.0 / 65535.0).abs() < 1e-5);
        cache.clear();
        let dirty = super::document::affected(&doc, "exposure");
        let again = render(&doc, &source, &mut cache, &dirty);
        assert!(again.preview.pixels[0] > 0);
        assert_eq!(cache.len(), 1);
        assert!(cache.contains_key("input"));
        let third = render(&doc, &source, &mut cache, &dirty);
        assert!(third.reran.contains(&"exposure".to_string()));
        assert!(!third.reran.iter().any(|id| id == "input"));
    }

    #[test]
    fn a_set_flag_stops_before_the_first_node() {
        let doc = default_document(false);
        let source = Frame {
            width: 1,
            height: 1,
            space: Space::Linear,
            rgb: Arc::new(vec![0.2, 0.2, 0.2]),
            transform: ViewTransform::default(),
            roi: Roi::default(),
        };
        let mut cache = HashMap::new();
        let dirty: HashSet<String> = doc.nodes.iter().map(|node| node.id.clone()).collect();
        let cancel = CancelFlag::new();
        cancel.cancel();
        let rendered = render_cancellable(&doc, &source, &mut cache, &dirty, &cancel);
        assert!(rendered.reran.is_empty());
        assert!(rendered.preview.pixels.is_empty());
    }

    #[test]
    fn bake_view_rotates_clockwise_like_css() {
        // 2x1 image, red left blue right. A 90deg clockwise turn puts red on top, blue below.
        let src = Rgba {
            width: 2,
            height: 1,
            pixels: vec![255, 0, 0, 255, 0, 0, 255, 255],
        };
        let out = bake_view(&src, 90.0, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!((out.width, out.height), (1, 2));
        assert_eq!(out.pixels[0..4], [255, 0, 0, 255]); // red on top
        assert_eq!(out.pixels[4..8], [0, 0, 255, 255]); // blue below
    }

    #[test]
    fn bake_view_passthrough_without_geometry() {
        let src = Rgba {
            width: 2,
            height: 2,
            pixels: vec![
                10, 20, 30, 255, 10, 20, 30, 255, 10, 20, 30, 255, 10, 20, 30, 255,
            ],
        };
        let out = bake_view(&src, 0.0, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(out.pixels, src.pixels);
    }

    /// Camera RGB above 1 is scene-referred, not an error. Clipping it before
    /// the camera matrix flips bright yellows to green; keep red above green.
    #[test]
    fn camera_bright_yellow_stays_warm() {
        let camera = crate::grade::document::CameraColor {
            name: "S5M2".to_string(),
            to_working: [
                0.8552234172821045,
                0.38491445779800415,
                -0.24013786017894745,
                -0.05828174576163292,
                1.4217835664749146,
                -0.3635018467903137,
                -0.01867656037211418,
                -0.2057872712612152,
                1.2244638204574585,
            ],
        };
        let doc = crate::grade::document::document_with_camera(true, 0.0, Some(camera));
        let render_camera = |rgb: [f32; 3]| {
            let frame = Frame {
                width: 1,
                height: 1,
                space: Space::Camera,
                rgb: Arc::new(vec![rgb[0], rgb[1], rgb[2]]),
                transform: ViewTransform::default(),
                roi: Roi::default(),
            };
            let mut cache = HashMap::new();
            let out = render(&doc, &frame, &mut cache, &Default::default());
            [
                out.preview.pixels[0],
                out.preview.pixels[1],
                out.preview.pixels[2],
            ]
        };
        let mid = render_camera([0.70, 0.52, 0.14]);
        let bright = render_camera([1.40, 1.05, 0.28]);
        assert!(
            bright[0] >= bright[1],
            "bright yellow went green: rgb={bright:?}"
        );
        let sum = |px: [u8; 3]| u32::from(px[0]) + u32::from(px[1]);
        assert!(
            sum(bright) > sum(mid),
            "brightening should raise the yellow, mid={mid:?} bright={bright:?}"
        );
    }

    #[test]
    fn bake_view_crops_the_roi() {
        // 4x4 solid, crop to the center half. No rotation.
        let mut pixels = Vec::new();
        for _ in 0..16 {
            pixels.extend_from_slice(&[1, 2, 3, 255]);
        }
        let src = Rgba {
            width: 4,
            height: 4,
            pixels,
        };
        let out = bake_view(&src, 0.0, [0.25, 0.25, 0.5, 0.5]);
        assert_eq!((out.width, out.height), (2, 2));
        assert_eq!(
            out.pixels,
            vec![1, 2, 3, 255, 1, 2, 3, 255, 1, 2, 3, 255, 1, 2, 3, 255]
        );
    }
}
