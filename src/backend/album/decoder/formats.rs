//! Image format flags and detection helpers.

use std::path::Path;

use super::DecodeError;

/// Single format flag (powers of two). Combine with [`ImageFormatSet`].
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Unknown = 0,
    Jpeg = 1 << 0,
    Png = 1 << 1,
    Gif = 1 << 2,
    Bmp = 1 << 3,
    Tiff = 1 << 4,
    Webp = 1 << 5,
    Ico = 1 << 6,
    Heic = 1 << 7,
    Cr2 = 1 << 8,
    Cr3 = 1 << 9,
    Crw = 1 << 10,
    Nef = 1 << 11,
    Nrw = 1 << 12,
    Arw = 1 << 13,
    Srf = 1 << 14,
    Sr2 = 1 << 15,
    Raf = 1 << 16,
    Orf = 1 << 17,
    Rw2 = 1 << 18,
    Rwl = 1 << 19,
    Pef = 1 << 20,
    Srw = 1 << 21,
    X3f = 1 << 22,
    Mrw = 1 << 23,
    Dcr = 1 << 24,
    Kdc = 1 << 25,
    Iiq = 1 << 26,
    ThreeFr = 1 << 27,
    Fff = 1 << 28,
    Mef = 1 << 29,
    Mos = 1 << 30,
    Erf = 1 << 31,
    Dng = 1 << 32,
}

impl ImageFormat {
    pub const fn bit(self) -> u64 {
        self as u64
    }

    pub fn is_camera_raw(self) -> bool {
        ImageFormatSet::CAMERA_RAW.contains(self)
    }
}

/// Bitmask of [`ImageFormat`] flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ImageFormatSet(u64);

impl ImageFormatSet {
    pub const EMPTY: Self = Self(0);

    pub const RASTER: Self = Self(
        ImageFormat::Jpeg.bit()
            | ImageFormat::Png.bit()
            | ImageFormat::Gif.bit()
            | ImageFormat::Bmp.bit()
            | ImageFormat::Tiff.bit()
            | ImageFormat::Webp.bit()
            | ImageFormat::Ico.bit(),
    );

    pub const CAMERA_RAW: Self = Self(
        ImageFormat::Cr2.bit()
            | ImageFormat::Cr3.bit()
            | ImageFormat::Crw.bit()
            | ImageFormat::Nef.bit()
            | ImageFormat::Nrw.bit()
            | ImageFormat::Arw.bit()
            | ImageFormat::Srf.bit()
            | ImageFormat::Sr2.bit()
            | ImageFormat::Raf.bit()
            | ImageFormat::Orf.bit()
            | ImageFormat::Rw2.bit()
            | ImageFormat::Rwl.bit()
            | ImageFormat::Pef.bit()
            | ImageFormat::Srw.bit()
            | ImageFormat::X3f.bit()
            | ImageFormat::Mrw.bit()
            | ImageFormat::Dcr.bit()
            | ImageFormat::Kdc.bit()
            | ImageFormat::Iiq.bit()
            | ImageFormat::ThreeFr.bit()
            | ImageFormat::Fff.bit()
            | ImageFormat::Mef.bit()
            | ImageFormat::Mos.bit()
            | ImageFormat::Erf.bit()
            | ImageFormat::Dng.bit(),
    );

    pub const fn from_one(format: ImageFormat) -> Self {
        Self(format.bit())
    }

    pub fn contains(self, format: ImageFormat) -> bool {
        format != ImageFormat::Unknown && self.0 & format.bit() != 0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn bits(self) -> u64 {
        self.0
    }
}

pub fn is_raw_path(path: &Path) -> bool {
    detect_format_from_path(path).is_camera_raw()
}

pub(crate) fn resolve_format(
    hint: Option<ImageFormat>,
    path: Option<&Path>,
    bytes: Option<&[u8]>,
) -> Result<ImageFormat, DecodeError> {
    if let Some(format) = hint.filter(|format| *format != ImageFormat::Unknown) {
        return Ok(format);
    }
    if let Some(path) = path {
        let format = detect_format_from_path(path);
        if format != ImageFormat::Unknown {
            return Ok(format);
        }
    }
    if let Some(bytes) = bytes {
        let format = detect_format_from_bytes(bytes);
        if format != ImageFormat::Unknown {
            return Ok(format);
        }
    }
    Err(DecodeError {
        code: "format_unknown",
        message: "could not determine image format".to_string(),
    })
}

pub(crate) fn detect_format_from_path(path: &Path) -> ImageFormat {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return ImageFormat::Unknown;
    };
    match ext.to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" | "jpe" => ImageFormat::Jpeg,
        "png" => ImageFormat::Png,
        "gif" => ImageFormat::Gif,
        "bmp" => ImageFormat::Bmp,
        "tif" | "tiff" => ImageFormat::Tiff,
        "webp" => ImageFormat::Webp,
        "ico" => ImageFormat::Ico,
        "heic" | "heif" | "hif" | "avif" => ImageFormat::Heic,
        "cr2" => ImageFormat::Cr2,
        "cr3" => ImageFormat::Cr3,
        "crw" => ImageFormat::Crw,
        "nef" => ImageFormat::Nef,
        "nrw" => ImageFormat::Nrw,
        "arw" => ImageFormat::Arw,
        "srf" => ImageFormat::Srf,
        "sr2" => ImageFormat::Sr2,
        "raf" => ImageFormat::Raf,
        "orf" => ImageFormat::Orf,
        "rw2" => ImageFormat::Rw2,
        "raw" => ImageFormat::Rw2,
        "rwl" => ImageFormat::Rwl,
        "pef" => ImageFormat::Pef,
        "srw" => ImageFormat::Srw,
        "x3f" => ImageFormat::X3f,
        "mrw" => ImageFormat::Mrw,
        "dcr" => ImageFormat::Dcr,
        "kdc" => ImageFormat::Kdc,
        "iiq" => ImageFormat::Iiq,
        "3fr" => ImageFormat::ThreeFr,
        "fff" => ImageFormat::Fff,
        "mef" => ImageFormat::Mef,
        "mos" => ImageFormat::Mos,
        "erf" => ImageFormat::Erf,
        "dng" => ImageFormat::Dng,
        _ => ImageFormat::Unknown,
    }
}

pub(crate) fn detect_format_from_bytes(bytes: &[u8]) -> ImageFormat {
    if bytes.len() >= 12 && is_heic_bytes(bytes) {
        return ImageFormat::Heic;
    }
    image::guess_format(bytes)
        .ok()
        .and_then(|fmt| match fmt {
            image::ImageFormat::Jpeg => Some(ImageFormat::Jpeg),
            image::ImageFormat::Png => Some(ImageFormat::Png),
            image::ImageFormat::Gif => Some(ImageFormat::Gif),
            image::ImageFormat::Bmp => Some(ImageFormat::Bmp),
            image::ImageFormat::Tiff => Some(ImageFormat::Tiff),
            image::ImageFormat::WebP => Some(ImageFormat::Webp),
            image::ImageFormat::Ico => Some(ImageFormat::Ico),
            _ => None,
        })
        .unwrap_or(ImageFormat::Unknown)
}

fn is_heic_bytes(bytes: &[u8]) -> bool {
    if bytes.len() < 12 {
        return false;
    }
    if &bytes[4..8] != b"ftyp" {
        return false;
    }
    let brand = &bytes[8..12];
    matches!(brand, b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"avif")
}
