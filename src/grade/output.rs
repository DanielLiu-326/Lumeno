//! Final output profiles. Tone mapping stays in the document; this module only
//! describes the target display/encoding envelope.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputProfile {
    /// SDR display-referred sRGB.
    #[default]
    SdrSrgb,
    /// Rec.2020 primaries, BT.2100 PQ, 203-nit reference white.
    HdrRec2020Pq,
    /// Rec.2020 primaries, BT.2100 HLG.
    HdrRec2020Hlg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HdrTransfer {
    Pq,
    Hlg,
}

impl OutputProfile {
    pub fn is_hdr(self) -> bool {
        matches!(self, Self::HdrRec2020Pq | Self::HdrRec2020Hlg)
    }

    pub fn hdr_transfer(self) -> Option<HdrTransfer> {
        match self {
            Self::SdrSrgb => None,
            Self::HdrRec2020Pq => Some(HdrTransfer::Pq),
            Self::HdrRec2020Hlg => Some(HdrTransfer::Hlg),
        }
    }

    /// Display white in nits. Matches darktable's HDR output profile: 1.0 in
    /// the tone-mapped frame is 203 nits, and values above 1.0 are headroom.
    pub fn reference_white_nits(self) -> f32 {
        203.0
    }
}

/// Map the export dialog's color-space and transfer selections to a supported
/// output profile. Keep unsupported combinations explicit instead of silently
/// writing files with the wrong metadata.
pub fn output_profile_from_selection(space: &str, transfer: &str) -> Option<OutputProfile> {
    let space = space.trim().to_ascii_lowercase();
    let transfer = transfer.trim().to_ascii_lowercase();
    match (space.as_str(), transfer.as_str()) {
        ("srgb", "srgb") => Some(OutputProfile::SdrSrgb),
        ("rec2020", "pq") => Some(OutputProfile::HdrRec2020Pq),
        ("rec2020", "hlg") => Some(OutputProfile::HdrRec2020Hlg),
        _ => None,
    }
}
