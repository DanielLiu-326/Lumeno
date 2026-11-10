# AI Models (SigLIP 2 + SCRFD / ArcFace via burn-onnx)

Local inference uses **Burn models generated from ONNX** (`burn-onnx` in `build.rs`).  
We do **not** hand-write ViT/ResNet layers or manually remap HF tensor names into a homemade Module tree.

See also: [`ai-provider-api.md`](./ai-provider-api.md).

> The local model pack includes InsightFace pretrained weights. Those weights
> are licensed for non-commercial research only. The repository, container
> images, and releases must not contain `dist/models/`.

## Pipeline

```bash
python3 -m venv .venv-ai
.venv-ai/bin/pip install torch --index-url https://download.pytorch.org/whl/cpu
.venv-ai/bin/pip install safetensors numpy huggingface_hub onnx onnxsim transformers tokenizers

# 1) Download + export ONNX → dist/models/onnx/
#    Requires acknowledging the InsightFace non-commercial terms.
.venv-ai/bin/python scripts/convert_ai_models.py --out dist/models --download \
    --accept-insightface-noncommercial

# 2) burn-onnx codegen + .bpk; build.rs also refreshes manifest.json checksums
cargo +nightly build

# Optional: same checksum refresh via Python
# .venv-ai/bin/python scripts/convert_ai_models.py --out dist/models --manifest-only
```

| Artifact | Role |
| --- | --- |
| `dist/models/onnx/*.onnx` | Inputs to `burn-onnx` (export with **batch=1**) |
| `dist/models/*.bpk` | Burnpack weights loaded at runtime |
| `OUT_DIR/model/*.rs` | Generated architecture (included via `onnx_gen`) |
| `dist/models/manifest.json` | sha256 + dims + revisions |

## Models

| Role | Source | License | Dim |
| --- | --- | --- | --- |
| Image–text | SigLIP 2 Base P32-256 (HF → ONNX vision/text) | Apache-2.0 | 768 |
| Face detect | InsightFace buffalo SCRFD (`det_10g`, opset↑19) | non-commercial research only | landmarks |
| Face embed | InsightFace `w600k_r50` | non-commercial research only | 512 |

## Behaviour

- Valid manifest + `.bpk` + build-time `ai_onnx_generated` → real models
- Missing ONNX at build → fail-closed local provider (`healthy=false`)
- `ai_fallback_interim=true` → DCT/heuristic only for emergencies/tests
- Inference always uses **batch size 1** (burn-onnx graphs hardcode batch=1 reshape / position embeds)
- Forward panics are caught and returned as errors so indexer workers stay alive
- Device: GPU first (CUDA if `--features cuda`, else Metal on macOS / Vulkan elsewhere), else Flex CPU. Override with `LUMENO_AI_DEVICE=cpu|gpu|cuda|metal|vulkan|auto`

## Smoke testset (<100 images)

```bash
.venv-ai/bin/python scripts/download_ai_testset.py --limit 60
RUST_MIN_STACK=134217728 cargo +nightly test --bin lumeno -- smoke_ai_testset --ignored --nocapture
# optional: LUMENO_AI_SMOKE_LIMIT=20
```

Results land in `dist/testdata/ai_smoke/results.json`.

## Public distribution

- Do not commit or publish `dist/models/` (ONNX, `.bpk`, or `manifest.json`), and do not add it to Docker images or GitHub Releases. `.dockerignore` excludes it; `.gitignore` excludes all of `dist/`.
- `--download` refuses to run without `--accept-insightface-noncommercial`; this is a local-use acknowledgement, not a redistribution license.
- Public or commercial deployments should use the remote provider (`ai_provider=remote`, see [`ai-provider-api.md`](./ai-provider-api.md)) or replace the face stack with commercially licensed models.

## License

See [`../THIRD_PARTY_LICENSES.md`](../THIRD_PARTY_LICENSES.md).

- SigLIP 2: Apache-2.0 (per the upstream model card)
- InsightFace weights: non-commercial research only — **do not bundle in public releases**
