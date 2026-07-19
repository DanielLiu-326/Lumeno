//! ONNX → Burn generated models (via `burn-onnx` in build.rs).

// Generated Burn ONNX code references `alloc::vec::Vec`.
extern crate alloc;

#[cfg(ai_onnx_generated)]
pub mod siglip2_vision {
    include!(concat!(env!("OUT_DIR"), "/model/siglip2_vision.rs"));
}

#[cfg(ai_onnx_generated)]
pub mod siglip2_text {
    include!(concat!(env!("OUT_DIR"), "/model/siglip2_text.rs"));
}

#[cfg(ai_onnx_generated)]
pub mod scrfd {
    include!(concat!(env!("OUT_DIR"), "/model/scrfd.rs"));
}

#[cfg(ai_onnx_generated)]
pub mod arcface_r50 {
    include!(concat!(env!("OUT_DIR"), "/model/arcface_r50.rs"));
}
