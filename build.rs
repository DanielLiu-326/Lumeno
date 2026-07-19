//! Generate Burn models from ONNX via burn-onnx (no hand-written nets).
//!
//! Expects files under `dist/models/onnx/`:
//! - siglip2_vision.onnx
//! - siglip2_text.onnx
//! - scrfd.onnx
//! - arcface_r50.onnx
//!
//! When present, sets `cfg(ai_onnx_generated)` and emits Rust + `.bpk` into OUT_DIR.
//! After copying `.bpk` files, refreshes `dist/models/manifest.json` checksums.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn main() {
    let onnx_dir = PathBuf::from("dist/models/onnx");
    println!("cargo:rerun-if-changed=dist/models/onnx");
    println!("cargo:rerun-if-changed=dist/models/siglip2-tokenizer.json");

    let inputs = [
        "siglip2_vision.onnx",
        "siglip2_text.onnx",
        "scrfd.onnx",
        "arcface_r50.onnx",
    ];

    let mut generated = Vec::new();
    for name in inputs {
        let path = onnx_dir.join(name);
        if path.is_file() {
            generated.push(path);
        } else {
            println!("cargo:warning=AI ONNX missing: {}", path.display());
        }
    }

    if generated.is_empty() {
        println!("cargo:warning=No AI ONNX models found; local provider stays fail-closed");
        println!("cargo:rustc-check-cfg=cfg(ai_onnx_generated)");
        return;
    }

    convert_all(&generated);
    // ModelGen can leave OUT_DIR and dist/models out of sync across rebuilds;
    // always re-copy then hash the dest files for the manifest.
    sync_bpks_to_dist(&generated);
    refresh_manifest(Path::new("dist/models"));

    println!("cargo:rustc-check-cfg=cfg(ai_onnx_generated)");
    println!("cargo:rustc-cfg=ai_onnx_generated");
    for path in &generated {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn convert_all(paths: &[PathBuf]) {
    use burn_onnx::ModelGen;

    for path in paths {
        let input = path.to_string_lossy().into_owned();
        println!("cargo:warning=burn-onnx converting {input}");
        ModelGen::new()
            .input(&input)
            .out_dir("model/")
            .run_from_script();
    }
}

fn sync_bpks_to_dist(paths: &[PathBuf]) {
    let Some(out_dir) = std::env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    let model_dir = out_dir.join("model");
    let dist = Path::new("dist/models");
    let _ = std::fs::create_dir_all(dist);
    for path in paths {
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let bpk = model_dir.join(format!("{stem}.bpk"));
        let dest = dist.join(format!("{stem}.bpk"));
        if !bpk.is_file() {
            println!("cargo:warning=missing generated {}", bpk.display());
            continue;
        }
        match std::fs::copy(&bpk, &dest) {
            Ok(_) => println!(
                "cargo:warning=synced {} -> {}",
                bpk.display(),
                dest.display()
            ),
            Err(error) => println!(
                "cargo:warning=failed to sync {} -> {}: {error}",
                bpk.display(),
                dest.display()
            ),
        }
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let digest = Sha256::digest(&bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn refresh_manifest(out_dir: &Path) {
    let required = [
        "siglip2_vision.bpk",
        "siglip2_text.bpk",
        "scrfd.bpk",
        "arcface_r50.bpk",
        "siglip2-tokenizer.json",
    ];
    for name in required {
        if !out_dir.join(name).is_file() {
            println!(
                "cargo:warning=skip manifest refresh; missing {}",
                out_dir.join(name).display()
            );
            return;
        }
    }

    let vision = sha256_file(&out_dir.join("siglip2_vision.bpk"));
    let text = sha256_file(&out_dir.join("siglip2_text.bpk"));
    let scrfd = sha256_file(&out_dir.join("scrfd.bpk"));
    let arcface = sha256_file(&out_dir.join("arcface_r50.bpk"));
    let tokenizer = sha256_file(&out_dir.join("siglip2-tokenizer.json"));
    let (Ok(vision), Ok(text), Ok(scrfd), Ok(arcface), Ok(tokenizer)) =
        (vision, text, scrfd, arcface, tokenizer)
    else {
        println!("cargo:warning=failed to hash model artifacts for manifest");
        return;
    };

    let manifest = format!(
        r#"{{
  "schema_version": 1,
  "revision": "siglip2-base-patch32-256+scrfd-buffalo+arcface-r50-w600k",
  "image_text": {{
    "id": "google/siglip2-base-patch32-256",
    "revision": "siglip2-base-patch32-256",
    "format": "burnpack",
    "file": "siglip2_vision.bpk",
    "sha256": "{vision}",
    "dimension": 768,
    "input_size": [256, 256],
    "extras": {{
      "text_file": "siglip2_text.bpk",
      "text_sha256": "{text}",
      "source": "burn-onnx"
    }}
  }},
  "face_detector": {{
    "id": "insightface/scrfd_buffalo",
    "revision": "scrfd-buffalo",
    "format": "burnpack",
    "file": "scrfd.bpk",
    "sha256": "{scrfd}",
    "dimension": 5,
    "input_size": [640, 640],
    "extras": {{"source": "burn-onnx"}}
  }},
  "face_embedder": {{
    "id": "insightface/w600k_r50",
    "revision": "arcface-r50-w600k",
    "format": "burnpack",
    "file": "arcface_r50.bpk",
    "sha256": "{arcface}",
    "dimension": 512,
    "input_size": [112, 112],
    "extras": {{"source": "burn-onnx"}}
  }},
  "tokenizer": {{
    "file": "siglip2-tokenizer.json",
    "sha256": "{tokenizer}",
    "max_length": 64
  }},
  "notes": "Architecture comes from burn-onnx codegen (build.rs). Do not hand-write / hand-map network layers."
}}
"#
    );

    let path = out_dir.join("manifest.json");
    match std::fs::write(&path, manifest) {
        Ok(()) => println!("cargo:warning=refreshed {}", path.display()),
        Err(error) => println!(
            "cargo:warning=failed to write {}: {error}",
            path.display()
        ),
    }
}
