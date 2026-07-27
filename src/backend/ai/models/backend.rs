//! Burn backend for local AI inference: GPU when available, else CPU (Flex).
//!
//! Selection order (override with `LUMENO_AI_DEVICE=cpu|gpu|cuda|vulkan|auto`):
//! 1. CUDA (if built with `--features cuda`)
//! 2. Vulkan GPU (wgpu)
//! 3. Flex CPU
//!
//! Vulkan memory uses a tight CubeCL `Custom` pool: exclusive pages capped well
//! below full VRAM, with short `dealloc_period` so idle pages return quickly.

use burn::Dispatch;
use burn::DispatchDevice;
use burn::backend::wgpu::{
    MemoryConfiguration, RuntimeOptions, WgpuDevice, graphics::Vulkan, init_setup,
};
use burn::prelude::*;
use cubecl_runtime::memory_management::{MemoryPoolOptions, PoolType};
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
        // Must configure CubeCL before the first Vulkan tensor touch, otherwise the
        // default SubSlices allocator wins for the process lifetime.
        let wgpu_device = WgpuDevice::DefaultDevice;
        if init_vulkan_tight_memory(&wgpu_device) {
            candidates.push((DispatchDevice::Vulkan(wgpu_device), "vulkan"));
        } else {
            eprintln!("[ai] vulkan tight-memory init failed; skipping vulkan backend");
        }
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

/// Exclusive-page buckets capped below full VRAM, with aggressive reclaim.
///
/// Max alloc (~1.5 GiB) still fits SigLIP text weights (~1.1 GiB) while preventing
/// the default SubSlices habit of growing toward heap/4 on large cards.
///
/// On Intel iGPU (shared RAM), prefer a short `dealloc_period` so idle activation
/// pages return quickly instead of ratcheting RSS for the whole index run.
fn tight_memory_config() -> MemoryConfiguration {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const MAX_ALLOC: u64 = 1536 * MB;
    const MIN_BUCKET: u64 = 32 * KB;
    // CubeCL: reclaim unused ExclusivePages after this many parent allocs.
    // Lower = faster return to the driver (better for shared iGPU memory).
    const DEALLOC_PERIOD: u64 = 5;

    let mut sizes = Vec::new();
    let mut size = MIN_BUCKET;
    while size < MAX_ALLOC {
        sizes.push(size);
        let next = size.saturating_mul(2);
        if next <= size {
            break;
        }
        size = next;
    }
    sizes.push(MAX_ALLOC);

    MemoryConfiguration::Custom {
        pool_options: sizes
            .into_iter()
            .map(|max_alloc_size| MemoryPoolOptions {
                pool_type: PoolType::ExclusivePages { max_alloc_size },
                dealloc_period: Some(DEALLOC_PERIOD),
            })
            .collect(),
    }
}

fn init_vulkan_tight_memory(device: &WgpuDevice) -> bool {
    catch_unwind(AssertUnwindSafe(|| {
        let _ = init_setup::<Vulkan>(
            device,
            RuntimeOptions {
                memory_config: tight_memory_config(),
                tasks_max: 1,
            },
        );
        eprintln!("[ai] vulkan memory strategy=tight (ExclusivePages + reclaim)");
    }))
    .is_ok()
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
