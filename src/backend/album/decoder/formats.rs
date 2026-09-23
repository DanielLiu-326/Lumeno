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

/// Resolve format from a file path extension (e.g. `.nef` → [`ImageFormat::Nef`]).
pub fn format_from_path(path: &Path) -> Result<ImageFormat, DecodeError> {
    let format = detect_format_from_path(path);
    if format == ImageFormat::Unknown {
        return Err(DecodeError {
            code: "format_unknown",
            message: format!(
                "could not determine image format from path: {}",
                path.display()
            ),
        });
    }
    Ok(format)
}

pub(crate) fn format_from_hint_or_path(
    hint: Option<ImageFormat>,
    path: Option<&Path>,
) -> Result<ImageFormat, DecodeError> {
    if let Some(format) = hint.filter(|format| *format != ImageFormat::Unknown) {
        return Ok(format);
    }
    if let Some(path) = path {
        return format_from_path(path);
    }
    Err(DecodeError {
        code: "format_required",
        message: "image format must be set in DecodeOptions or inferred from path".to_string(),
    })
}

pub(crate) fn require_format(hint: Option<ImageFormat>) -> Result<ImageFormat, DecodeError> {
    hint.filter(|format| *format != ImageFormat::Unknown)
        .ok_or_else(|| DecodeError {
            code: "format_required",
            message: "decode_bytes requires DecodeOptions.format; use decode_bytes_with_path or DecodeOptions::for_path".to_string(),
        })
}

pub(crate) fn resolve_format(
    hint: Option<ImageFormat>,
    path: Option<&Path>,
    bytes: Option<&[u8]>,
) -> Result<ImageFormat, DecodeError> {
    if let Ok(format) = format_from_hint_or_path(hint, path) {
        return Ok(format);
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
    if let Some(raw) = detect_raw_magic(bytes) {
        return raw;
    }
    if let Some(raw) = detect_tiff_based_raw(bytes) {
        return raw;
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

/// Sniff common camera-RAW containers that `image::guess_format` misses.
///
/// Panasonic RW2 uses TIFF-like little-endian with magic `IIU\0` (not classic `II*\0`).
fn detect_raw_magic(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.len() < 4 {
        return None;
    }
    // Panasonic / Leica RW2
    if bytes.starts_with(b"IIU\0") {
        return Some(ImageFormat::Rw2);
    }
    // Fujifilm RAF
    if bytes.starts_with(b"FUJIFILMCCD-RAW ") {
        return Some(ImageFormat::Raf);
    }
    // Canon CR3 (ISO BMFF with 'crx ' / 'CRX ' brand variants)
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        if brand == b"crx " || brand == b"CRX " || brand == b"cr3 " {
            return Some(ImageFormat::Cr3);
        }
    }
    None
}

/// TIFF-based camera RAW (NEF, CR2, ARW, …) shares `II*` / `MM*` magic with plain TIFF.
/// Without this sniff, `image::guess_format` returns TIFF and Bayer data renders as noise.
fn detect_tiff_based_raw(bytes: &[u8]) -> Option<ImageFormat> {
    if bytes.len() < 4 {
        return None;
    }
    let is_tiff = bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*");
    if !is_tiff {
        return None;
    }
    let scan = &bytes[..bytes.len().min(512 * 1024)];
    if contains_ascii_insensitive(scan, b"NIKON") {
        return Some(ImageFormat::Nef);
    }
    if contains_ascii_insensitive(scan, b"CANON") {
        return Some(ImageFormat::Cr2);
    }
    if contains_ascii_insensitive(scan, b"PENTAX") {
        return Some(ImageFormat::Pef);
    }
    if contains_ascii_insensitive(scan, b"OLYMPUS")
        || contains_ascii_insensitive(scan, b"OM DIGITAL")
    {
        return Some(ImageFormat::Orf);
    }
    if contains_ascii_insensitive(scan, b"SONY") {
        return Some(ImageFormat::Arw);
    }
    if contains_ascii_insensitive(scan, b"FUJIFILM") {
        return Some(ImageFormat::Raf);
    }
    if contains_ascii_insensitive(scan, b"LEICA") {
        return Some(ImageFormat::Dng);
    }
    if contains_ascii_insensitive(scan, b"PANASONIC") {
        return Some(ImageFormat::Rw2);
    }
    if contains_ascii_insensitive(scan, b"SAMSUNG") {
        return Some(ImageFormat::Srw);
    }
    if contains_ascii_insensitive(scan, b"HASSELBLAD") {
        return Some(ImageFormat::Fff);
    }
    if contains_ascii_insensitive(scan, b"Phase One")
        || contains_ascii_insensitive(scan, b"PHASEONE")
    {
        return Some(ImageFormat::Iiq);
    }
    if contains_ascii_insensitive(scan, b"DNG") {
        return Some(ImageFormat::Dng);
    }
    None
}

fn contains_ascii_insensitive(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || haystack.len() < needle.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

fn is_heic_bytes(bytes: &[u8]) -> bool {
    if bytes.len() < 12 {
        return false;
    }
    if &bytes[4..8] != b"ftyp" {
        return false;
    }
    let brand = &bytes[8..12];
    matches!(
        brand,
        b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"avif"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn rw2_extension_resolves() {
        assert_eq!(
            detect_format_from_path(Path::new("photo.RW2")),
            ImageFormat::Rw2
        );
    }

    #[test]
    fn panasonic_rw2_magic_resolves() {
        let mut bytes = vec![0_u8; 16];
        bytes[..4].copy_from_slice(b"IIU\0");
        assert_eq!(detect_format_from_bytes(&bytes), ImageFormat::Rw2);
    }

    #[test]
    fn resolve_prefers_path_over_unknown_bytes() {
        let bytes = b"not an image";
        let format = resolve_format(None, Some(Path::new("shot.rw2")), Some(bytes)).unwrap();
        assert_eq!(format, ImageFormat::Rw2);
    }

    #[test]
    fn nikon_nef_tiff_magic_resolves_as_nef() {
        let mut bytes = vec![0_u8; 256];
        bytes[..4].copy_from_slice(b"II*\0");
        bytes[32..48].copy_from_slice(b"NIKON CORPORATION");
        assert_eq!(detect_format_from_bytes(&bytes), ImageFormat::Nef);
    }

    #[test]
    fn plain_tiff_without_maker_stays_tiff() {
        let mut bytes = vec![0_u8; 64];
        bytes[..4].copy_from_slice(b"II*\0");
        assert_eq!(detect_format_from_bytes(&bytes), ImageFormat::Tiff);
    }
}
