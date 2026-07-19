//! Burn backend for local AI inference: GPU when available, else CPU (Flex).
//!
//! Selection order (override with `LUMENO_AI_DEVICE=cpu|gpu|cuda|vulkan|auto`):
//! 1. CUDA (if built with `--features cuda`)
//! 2. Vulkan GPU (wgpu)
//! 3. Flex CPU

use burn::prelude::*;
use burn::Dispatch;
use burn::DispatchDevice;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;

pub type AiBackend = Dispatch;
pub type AiDevice = DispatchDevice;

#[derive(Debug, Clone)]
pub struct AiDeviceChoice {
    pub device: AiDevice,
    pub label: String,
}

static SELECTED: OnceLock<AiDeviceChoice> = OnceLock::new();

/// Selected once per process; shared by SigLIP / SCRFD / ArcFace.
pub fn selected_ai_device() -> AiDeviceChoice {
    SELECTED.get_or_init(select_ai_device).clone()
}

fn select_ai_device() -> AiDeviceChoice {
    let prefer = std::env::var("LUMENO_AI_DEVICE")
        .unwrap_or_else(|_| "auto".into())
        .to_ascii_lowercase();

    if prefer == "cpu" {
        return cpu_choice();
    }

    let force_gpu = matches!(prefer.as_str(), "gpu" | "cuda" | "vulkan");
    let mut candidates: Vec<(AiDevice, &'static str)> = Vec::new();

    #[cfg(feature = "cuda")]
    if matches!(prefer.as_str(), "auto" | "gpu" | "cuda") {
        candidates.push((DispatchDevice::Cuda(Default::default()), "cuda"));
    }

    if matches!(prefer.as_str(), "auto" | "gpu" | "vulkan") {
        candidates.push((
            DispatchDevice::Vulkan(burn::backend::wgpu::WgpuDevice::DefaultDevice),
            "vulkan",
        ));
    }

    for (device, label) in candidates {
        if probe_device(&device) {
            eprintln!("[ai] using {label} device for local inference ({device:?})");
            return AiDeviceChoice {
                device,
                label: label.to_string(),
            };
        }
        eprintln!("[ai] {label} probe failed; trying next backend");
    }

    if force_gpu {
        eprintln!("[ai] requested GPU but none available; falling back to CPU Flex");
    } else {
        eprintln!("[ai] using CPU Flex for local inference");
    }
    cpu_choice()
}

fn cpu_choice() -> AiDeviceChoice {
    AiDeviceChoice {
        device: DispatchDevice::Flex(Default::default()),
        label: "cpu".to_string(),
    }
}

fn probe_device(device: &AiDevice) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        let a = Tensor::<AiBackend, 1>::from_floats([1.0_f32, 2.0, 3.0, 4.0], device);
        let b = Tensor::<AiBackend, 1>::from_floats([4.0_f32, 3.0, 2.0, 1.0], device);
        let out = a * b;
        let _ = out.into_data();
    }))
    .is_ok()
}
