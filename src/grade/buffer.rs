//! The four pipeline buffers, plus the input and output descriptors that name them.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkingSpace {
    LinearRec2020,
    /// Linear camera RGB, before Input Color Profile.
    Camera,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataType {
    Mosaic,
    LinearRgb,
    DisplayRgb,
    Mask,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputDesc {
    pub name: &'static str,
    /// Kinds this one input accepts. A run carries only one of them.
    pub types: &'static [DataType],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputDesc {
    pub name: &'static str,
    /// Kinds this one output might write. A run writes only one of them.
    pub types: &'static [DataType],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CfaPattern {
    Bayer,
    XTrans,
}

#[derive(Clone, Debug)]
pub struct MosaicBuffer {
    pub size: ImageSize,
    pub cfa: CfaPattern,
    pub black_level: f32,
    pub white_level: f32,
    pub camera_to_xyz_d50: [f32; 9],
    pub as_shot_gains: [f32; 3],
    pub linear: bool,
    pub samples_code: Vec<u16>,
    pub samples_linear: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct LinearBuffer {
    pub size: ImageSize,
    pub space: WorkingSpace,
    pub rgb: std::sync::Arc<Vec<f32>>,
    pub degrees: f32,
    pub roi: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct DisplayBuffer {
    pub size: ImageSize,
    pub rgb: std::sync::Arc<Vec<f32>>,
    pub degrees: f32,
    pub roi: [f32; 4],
}

#[derive(Clone, Debug)]
pub struct MaskBuffer {
    pub size: ImageSize,
    pub samples: Vec<f32>,
}

#[derive(Clone, Debug)]
pub enum Value {
    Mosaic(MosaicBuffer),
    Linear(LinearBuffer),
    Display(DisplayBuffer),
    Mask(MaskBuffer),
}

pub(crate) static MOSAIC: &[DataType] = &[DataType::Mosaic];
pub(crate) static LINEAR_RGB: &[DataType] = &[DataType::LinearRgb];
pub(crate) static MOSAIC_OR_LINEAR: &[DataType] = &[DataType::Mosaic, DataType::LinearRgb];
pub(crate) static DISPLAY_RGB: &[DataType] = &[DataType::DisplayRgb];
pub(crate) static MASK: &[DataType] = &[DataType::Mask];

pub(crate) fn image_in(types: &'static [DataType]) -> InputDesc {
    InputDesc {
        name: "image",
        types,
    }
}

pub(crate) fn image_out(types: &'static [DataType]) -> OutputDesc {
    OutputDesc {
        name: "image",
        types,
    }
}

pub(crate) fn mask_in() -> InputDesc {
    InputDesc {
        name: "mask",
        types: MASK,
    }
}

pub(crate) fn mask_out() -> OutputDesc {
    OutputDesc {
        name: "mask",
        types: MASK,
    }
}
