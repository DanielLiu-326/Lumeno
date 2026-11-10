# Third-Party Licenses

Lumeno itself is licensed under **AGPL-3.0-or-later**; see [LICENSE](LICENSE).

This file lists the third-party components that are vendored, statically linked, or dynamically linked, plus model assets that are **not** distributed with this repository.

## Vendored / linked components

### LibRaw

- Location: `vendor/rsraw-sys/LibRaw`
- License: LGPL-2.1 **or** CDDL-1.0 (dual-licensed)
- Full texts: `vendor/rsraw-sys/LibRaw/LICENSE.LGPL`, `vendor/rsraw-sys/LibRaw/LICENSE.CDDL`
- Notes: LibRaw is compiled into `rsraw-sys` as a static library. Distributors of binaries must satisfy the LGPL-2.1 relinking/source obligations for LibRaw. The vendored copy is patched for LibRaw 0.22.2; see `vendor/rsraw-sys/README.lumeno.md`.

### libheif

- License: LGPL-3.0
- Linkage: system `libheif` shared library via `libheif-rs`
- Notes: provides HEIC/AVIF support. Do not enable `embedded-libheif` unless you also satisfy the LGPL-3.0 static-linking obligations.

## Model assets (not in this repository)

### SigLIP 2

- Source: `google/siglip2-base-patch32-256`
- License: Apache-2.0
- Notes: generated ONNX/Burnpack artifacts are build outputs; check the upstream model card before redistributing weights.

### InsightFace SCRFD / ArcFace (buffalo_l)

- Source: `deepinsight/insightface`
- License: the InsightFace code is MIT; the pretrained models are for **non-commercial research only**
- Notes: Lumeno does not bundle these weights. `scripts/convert_ai_models.py --download` refuses to run without `--accept-insightface-noncommercial`; that flag is a local-use acknowledgement, not a redistribution license. Do not include the weights or converted `.bpk` files in public releases or commercial deployments without a separate license.

## Other dependencies

Rust dependencies are pinned in `Cargo.lock`; frontend dependencies are pinned in `frontend/package-lock.json`. Before distributing binaries, generate a complete inventory with tools such as `cargo-about` and `license-checker`.
