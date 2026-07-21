use image::DynamicImage;

pub fn exif_orientation(bytes: &[u8]) -> u16 {
    (|| -> Option<u16> {
        let mut cursor = std::io::Cursor::new(bytes);
        let exif = exif::Reader::new().read_from_container(&mut cursor).ok()?;
        let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
        if let exif::Value::Short(ref v) = field.value {
            v.first().copied()
        } else {
            None
        }
    })()
    .unwrap_or(1)
}

pub fn apply_exif_orientation(img: DynamicImage, orientation: u16) -> DynamicImage {
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

pub fn apply_exif_from_bytes(img: DynamicImage, bytes: &[u8]) -> DynamicImage {
    apply_exif_orientation(img, exif_orientation(bytes))
}
